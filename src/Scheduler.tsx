// Scheduler: downloads that start at a set time, once or every week, and
// pause again at an optional stop time. The schedules themselves are kept
// and run by the engine (crates/engine/src/schedule.rs), which works while
// the window is closed to the tray; this page only shows and edits them.

import { useMemo, useState, type FormEvent } from "react";
import { CalendarClock, Pause, Pencil, Play, Plus, Trash2 } from "lucide-react";
import { api, displayName, isDeleted, type Download, type Schedule, type ScheduleSpec } from "./api";
import { DateField, dateOfDay as dateOf, dayValue, FileIcon, Modal, TimeField, useApp } from "./ui";
import { dateText, listText, t } from "./i18n";
import { formatLocale } from "./i18n/format";

/* ---------- times and days ---------- */

/** Monday first, as the engine counts them (bit 0 is Monday). */
const WEEK = [0, 1, 2, 3, 4, 5, 6];
const EVERY_DAY = 0b1111111;
const WEEKDAYS = 0b0011111;
const WEEKENDS = 0b1100000;

/** The name of a day of the week (0 Monday), short or long, in the interface's language. */
function dayName(day: number, width: "short" | "long" = "short"): string {
  // 2024-01-01 was a Monday.
  return dateText(new Date(2024, 0, 1 + day), { weekday: width });
}

/** A time of day, given in minutes after midnight, written the language's way. */
function clockText(minutes: number): string {
  return dateText(new Date(2024, 0, 1, Math.floor(minutes / 60), minutes % 60), { hour: "numeric", minute: "2-digit" });
}

/** "in 3 hours", "2 days ago", the language's way. */
function relative(seconds: number): string {
  const diff = seconds - Date.now() / 1000;
  const fmt = new Intl.RelativeTimeFormat(formatLocale(), { numeric: "auto" });
  const abs = Math.abs(diff);
  if (abs < 3600) return fmt.format(Math.round(diff / 60), "minute");
  if (abs < 86400) return fmt.format(Math.round(diff / 3600), "hour");
  return fmt.format(Math.round(diff / 86400), "day");
}

/** A moment, with how far off it is: "Mon 30 Nov, 23:00 (in 3 hours)". */
function whenText(seconds: number): string {
  const date = dateText(seconds * 1000, { weekday: "short", day: "numeric", month: "short", hour: "numeric", minute: "2-digit" });
  return `${date} (${relative(seconds)})`;
}

/** When a schedule starts, in words: "Every Mon, Wed and Fri, 23:00 to 07:00". */
function repeatText(s: ScheduleSpec): string {
  const times = s.stop == null ? clockText(s.start) : t.scheduler.window(clockText(s.start), clockText(s.stop));
  if (s.repeat.kind === "once") {
    const day = dateOf(s.repeat.date);
    const date = day ? dateText(day, { weekday: "short", day: "numeric", month: "short", year: "numeric" }) : s.repeat.date;
    return `${t.scheduler.once(date)} · ${times}`;
  }
  const days = s.repeat.days & EVERY_DAY;
  const which =
    days === EVERY_DAY
      ? t.scheduler.everyDay
      : days === WEEKDAYS
        ? t.scheduler.weekdays
        : days === WEEKENDS
          ? t.scheduler.weekends
          : t.scheduler.onDays(listText(WEEK.filter((d) => days & (1 << d)).map((d) => dayName(d))));
  return `${which} · ${times}`;
}

/* ---------- the page ---------- */

export function SchedulerView(props: {
  schedules: Schedule[];
  onNew: () => void;
  onEdit: (s: Schedule) => void;
}) {
  const { downloads, actions } = useApp();
  const groups = useMemo(() => {
    const by = (status: Schedule["status"][]) =>
      props.schedules
        .filter((s) => status.includes(s.status))
        .sort((a, b) => (a.next_start ?? Infinity) - (b.next_start ?? Infinity) || a.id - b.id);
    return [
      { key: "active", title: t.scheduler.sections.active, items: by(["active"]) },
      { key: "upcoming", title: t.scheduler.sections.upcoming, items: by(["waiting"]) },
      { key: "paused", title: t.scheduler.sections.paused, items: by(["paused"]) },
      { key: "finished", title: t.scheduler.sections.finished, items: by(["done", "missed"]) },
    ].filter((g) => g.items.length > 0);
  }, [props.schedules]);

  async function run(work: Promise<unknown>, done?: string) {
    try {
      await work;
      if (done) actions.toast(done, "info");
    } catch (err) {
      actions.toast(String(err), "error");
    }
  }

  return (
    <div className="page page-scheduler">
      <div className="page-head">
        <h1>{t.scheduler.title}</h1>
        <span className="spacer" />
        <button type="button" className="btn btn-primary" onClick={props.onNew}>
          <Plus size={16} strokeWidth={2.2} aria-hidden="true" /> {t.scheduler.newSchedule}
        </button>
      </div>
      <p className="page-intro">{t.scheduler.intro}</p>

      {groups.length === 0 ? (
        <div className="empty">
          <CalendarClock size={34} strokeWidth={1.6} aria-hidden="true" className="empty-icon" />
          <h2>{t.scheduler.empty.title}</h2>
          <p>{t.scheduler.empty.text}</p>
          <button type="button" className="btn btn-primary" onClick={props.onNew}>
            {t.scheduler.newSchedule}
          </button>
        </div>
      ) : (
        <div className="schedule-groups">
          {groups.map((g) => (
            <section key={g.key} className="schedule-group" aria-label={g.title}>
              <h2>
                {g.title} <span className="chip-count">{g.items.length}</span>
              </h2>
              <ul className="schedule-list">
                {g.items.map((s) => (
                  <ScheduleCard
                    key={s.id}
                    s={s}
                    downloads={downloads}
                    onEdit={() => props.onEdit(s)}
                    onToggle={() => run(api.setScheduleEnabled(s.id, !s.enabled))}
                    onDelete={() => run(api.deleteSchedule(s.id), t.scheduler.deleted)}
                  />
                ))}
              </ul>
            </section>
          ))}
        </div>
      )}
    </div>
  );
}

/** How many of a schedule's downloads are named on its card. */
const NAMED = 3;

function ScheduleCard(props: {
  s: Schedule;
  downloads: Download[];
  onEdit: () => void;
  onToggle: () => void;
  onDelete: () => void;
}) {
  const { s } = props;
  const picked = s.downloads
    .map((id) => props.downloads.find((d) => d.id === id))
    .filter((d): d is Download => !!d);
  const finished = s.status === "done" || s.status === "missed";
  const last = s.last ? t.scheduler.last[s.last.kind](relative(s.last.at)) : null;

  return (
    <li className={`schedule-card is-${s.status}`}>
      <span className="schedule-badge" aria-hidden="true">
        <CalendarClock size={18} strokeWidth={2} />
      </span>
      <div className="schedule-body">
        <div className="schedule-title">
          <strong>{s.name || repeatText(s)}</strong>
          <span className={`schedule-status is-${s.status}`}>{t.scheduler.status[s.status]}</span>
        </div>
        <p className="schedule-when">{repeatText(s)}</p>
        <p className="schedule-next">
          {s.status === "active" && s.next_stop != null
            ? t.scheduler.stopsAt(whenText(s.next_stop))
            : s.enabled && s.next_start != null
              ? t.scheduler.nextStart(whenText(s.next_start))
              : finished || !s.enabled
                ? last
                : t.scheduler.noNext}
          {s.status === "active" && last && <span className="schedule-last"> · {last}</span>}
        </p>
        {picked.length > 0 ? (
          <ul className="schedule-files" aria-label={t.scheduler.downloads(picked.length)}>
            {picked.slice(0, NAMED).map((d) => (
              <li key={d.id} title={displayName(d)}>
                <FileIcon d={d} />
                <span>{displayName(d)}</span>
              </li>
            ))}
            {picked.length > NAMED && <li className="schedule-more">{t.scheduler.more(picked.length - NAMED)}</li>}
          </ul>
        ) : (
          <p className="schedule-next">{t.scheduler.noDownloads}</p>
        )}
      </div>
      <div className="schedule-actions">
        {!finished && (
          <button
            type="button"
            className="icon-btn is-outlined"
            title={s.enabled ? t.scheduler.actions.pause : t.scheduler.actions.resume}
            aria-label={s.enabled ? t.scheduler.actions.pause : t.scheduler.actions.resume}
            onClick={props.onToggle}
          >
            {s.enabled ? <Pause size={15} strokeWidth={2.2} /> : <Play size={15} strokeWidth={2.2} />}
          </button>
        )}
        <button
          type="button"
          className="icon-btn is-outlined"
          title={t.scheduler.actions.edit}
          aria-label={t.scheduler.actions.edit}
          onClick={props.onEdit}
        >
          <Pencil size={15} strokeWidth={2.2} />
        </button>
        <button
          type="button"
          className="icon-btn is-outlined is-danger"
          title={t.scheduler.actions.delete}
          aria-label={t.scheduler.actions.delete}
          onClick={props.onDelete}
        >
          <Trash2 size={15} strokeWidth={2.2} />
        </button>
      </div>
    </li>
  );
}

/* ---------- making or changing a schedule ---------- */

/** Downloads that a schedule can start: any that has not finished. */
function schedulable(d: Download): boolean {
  return d.status !== "completed" && !isDeleted(d);
}

export function ScheduleDialog(props: {
  /** The schedule to change; none to make a new one. */
  schedule?: Schedule;
  /** Downloads to have picked in a new schedule. */
  picked?: number[];
  onClose: () => void;
}) {
  const { downloads, actions } = useApp();
  const old = props.schedule;
  const soon = new Date(Date.now() + 60 * 60 * 1000);
  const [name, setName] = useState(old?.name ?? "");
  const [kind, setKind] = useState<"once" | "weekly">(old?.repeat.kind ?? "once");
  const [date, setDate] = useState(old?.repeat.kind === "once" ? old.repeat.date : dayValue(soon));
  const [days, setDays] = useState(old?.repeat.kind === "weekly" ? old.repeat.days : WEEKDAYS);
  const [start, setStart] = useState(old?.start ?? soon.getHours() * 60);
  const [hasStop, setHasStop] = useState(old ? old.stop != null : false);
  const [stop, setStop] = useState(old?.stop ?? ((old?.start ?? soon.getHours() * 60) + 240) % 1440);
  const [chosen, setChosen] = useState<number[]>(old?.downloads ?? props.picked ?? []);
  /** Where its downloads' files go once finished; null for their usual folder. */
  const [folder, setFolder] = useState<string | null>(old?.folder ?? null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);

  // What can be picked: unfinished downloads, and any already in this schedule.
  const choices = useMemo(
    () => downloads.filter((d) => schedulable(d) || chosen.includes(d.id)),
    // the list is read when the dialog opens; progress does not reorder it
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [downloads.length],
  );

  function toggle(id: number) {
    setError("");
    setChosen((c) => (c.includes(id) ? c.filter((x) => x !== id) : [...c, id]));
  }

  async function submit(e: FormEvent) {
    e.preventDefault();
    const startAt = start;
    const stopAt = hasStop ? stop : null;
    if (chosen.length === 0) return setError(t.scheduler.dialog.errors.downloads);
    if (kind === "weekly" && (days & EVERY_DAY) === 0) return setError(t.scheduler.dialog.errors.days);
    if (kind === "once") {
      const day = dateOf(date);
      if (!day) return;
      const startsAt = new Date(day.getFullYear(), day.getMonth(), day.getDate(), Math.floor(startAt / 60), startAt % 60);
      if (startsAt.getTime() <= Date.now()) return setError(t.scheduler.dialog.errors.past);
    }
    const spec: ScheduleSpec = {
      name: name.trim(),
      repeat: kind === "once" ? { kind: "once", date } : { kind: "weekly", days },
      start: startAt,
      stop: stopAt,
      downloads: chosen,
      folder,
    };
    setBusy(true);
    try {
      await api.saveSchedule(old?.id ?? null, spec);
      actions.toast(t.scheduler.saved, "info");
      props.onClose();
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  }

  const d = t.scheduler.dialog;
  return (
    <Modal
      title={old ? d.editTitle : d.newTitle}
      onClose={props.onClose}
      wide
      footer={
        <>
          {error && (
            <small className="field-error" role="alert">
              {error}
            </small>
          )}
          <span className="spacer" />
          <button type="button" className="btn" onClick={props.onClose}>
            {t.dialogs.cancel}
          </button>
          <button type="submit" form="schedule-form" className="btn btn-primary" disabled={busy}>
            {d.save}
          </button>
        </>
      }
    >
      <form id="schedule-form" className="form schedule-form" onSubmit={submit}>
        <label className="field">
          <span>{d.name}</span>
          <input
            data-autofocus
            type="text"
            value={name}
            maxLength={80}
            onChange={(e) => setName(e.target.value)}
            placeholder={d.namePlaceholder}
          />
        </label>

        <div className="field">
          <span id="schedule-repeat">{d.repeat}</span>
          <div className="segmented" role="radiogroup" aria-labelledby="schedule-repeat">
            {(["once", "weekly"] as const).map((k) => (
              <button
                key={k}
                type="button"
                role="radio"
                aria-checked={kind === k}
                className={kind === k ? "is-active" : undefined}
                onClick={() => {
                  setKind(k);
                  setError("");
                }}
              >
                {k === "once" ? d.once : d.weekly}
              </button>
            ))}
          </div>
        </div>

        {kind === "once" ? (
          <div className="field">
            <span id="schedule-date">{d.date}</span>
            <DateField
              aria-labelledby="schedule-date"
              value={date}
              min={dayValue(new Date())}
              onChange={(v) => {
                setDate(v);
                setError("");
              }}
            />
          </div>
        ) : (
          <div className="field">
            <span id="schedule-days">{d.days}</span>
            <div className="day-picker" role="group" aria-labelledby="schedule-days">
              {WEEK.map((day) => {
                const on = (days & (1 << day)) !== 0;
                return (
                  <button
                    key={day}
                    type="button"
                    className={`chip${on ? " is-active" : ""}`}
                    aria-pressed={on}
                    title={dayName(day, "long")}
                    onClick={() => {
                      setDays((v) => v ^ (1 << day));
                      setError("");
                    }}
                  >
                    {dayName(day)}
                  </button>
                );
              })}
            </div>
          </div>
        )}

        <div className="schedule-times">
          <div className="field">
            <span id="schedule-start">{d.start}</span>
            <TimeField
              aria-labelledby="schedule-start"
              value={start}
              onChange={(m) => {
                setStart(m);
                setError("");
              }}
            />
          </div>
          <div className="field">
            <label className="check-line">
              <input type="checkbox" checked={hasStop} onChange={(e) => setHasStop(e.target.checked)} />
              <span>{d.stop}</span>
            </label>
            <TimeField aria-label={d.stop} disabled={!hasStop} value={stop} onChange={setStop} />
          </div>
        </div>
        {hasStop && <small className="form-note">{d.stopHint}</small>}

        <div className="field">
          <span id="schedule-downloads">{d.downloads}</span>
          {choices.length === 0 ? (
            <p className="form-note">{d.noneToPick}</p>
          ) : (
            <ul className="schedule-picker" aria-labelledby="schedule-downloads">
              {choices.map((x) => {
                const at = chosen.indexOf(x.id);
                return (
                  <li key={x.id}>
                    <label className={`schedule-pick${at >= 0 ? " is-on" : ""}`}>
                      <input type="checkbox" checked={at >= 0} onChange={() => toggle(x.id)} />
                      <FileIcon d={x} />
                      <span className="schedule-pick-name" title={displayName(x)}>
                        {displayName(x)}
                      </span>
                      {at >= 0 && <span className="schedule-order">{at + 1}</span>}
                    </label>
                  </li>
                );
              })}
            </ul>
          )}
          <small>{d.downloadsHint}</small>
        </div>

        <div className="field">
          <span id="schedule-folder">{t.organize.schedule.folder}</span>
          <div className="field-row" aria-labelledby="schedule-folder">
            <input type="text" readOnly value={folder ?? t.organize.schedule.usual} title={folder ?? undefined} aria-labelledby="schedule-folder" />
            <button
              type="button"
              className="btn"
              onClick={async () => {
                const picked = await api.pickFolder(folder ?? undefined);
                if (picked) setFolder(picked);
              }}
            >
              {t.organize.schedule.choose}
            </button>
            {folder && (
              <button type="button" className="btn" onClick={() => setFolder(null)}>
                {t.organize.schedule.clear}
              </button>
            )}
          </div>
          <small>{t.organize.schedule.hint}</small>
        </div>
      </form>
    </Modal>
  );
}
