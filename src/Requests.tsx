/* Request options: the form (headers, cookies, a sign-in, timeouts), the
 * Request part of the add dialog, and Settings › Network › Request
 * profiles. The engine keeps private values encrypted
 * (crates/engine/src/request.rs, vault.rs); they go in and never come back. */
import { useEffect, useState, type FormEvent } from "react";
import { createPortal } from "react-dom";
import { Plus, X } from "lucide-react";
import { api, type AuthKind, type PasswordChange, type RequestBook, type RequestOptions, type RequestProfile } from "./api";
import { t } from "./i18n";
import { IconBtn, Modal, Select } from "./ui";

export const BLANK_REQUEST: RequestOptions = {
  user_agent: null,
  referer: null,
  headers: [],
  auth: "none",
  username: "",
  read_timeout: null,
  connect_timeout: null,
};

const AUTH_KINDS: AuthKind[] = ["none", "basic", "digest", "bearer"];

/** What the form holds: the options, and private values typed in. */
export interface RequestDraft {
  options: RequestOptions;
  password: string;
  cookies: string;
  clearPassword: boolean;
  clearCookies: boolean;
}

export function draftOf(options: RequestOptions = BLANK_REQUEST): RequestDraft {
  return { options: { ...options, headers: options.headers.map(([n, v]) => [n, v]) }, password: "", cookies: "", clearPassword: false, clearCookies: false };
}

const blankToNull = (s: string) => (s.trim() ? s : null);
const seconds = (s: string) => {
  const n = Math.round(Number(s));
  return s.trim() && Number.isFinite(n) ? n : null;
};

/** The options to send with a new download, private values included. */
export function inlineOptions(d: RequestDraft): RequestOptions {
  return { ...d.options, password: d.password || null, cookies: d.cookies || null };
}

function change(typed: string, clear: boolean): PasswordChange {
  return typed ? { set: typed } : clear ? "clear" : "keep";
}

/** The form itself. `saved` is set when editing a profile whose private
 *  values are kept: their fields then say so instead of showing them. */
export function RequestForm(props: { draft: RequestDraft; onChange: (d: RequestDraft) => void; saved?: boolean }) {
  const { draft } = props;
  const o = draft.options;
  const set = (patch: Partial<RequestOptions>) => props.onChange({ ...draft, options: { ...o, ...patch } });
  const r = t.request;
  const passwordSaved = props.saved && o.has_password && !draft.clearPassword;
  const cookiesSaved = props.saved && o.has_cookies && !draft.clearCookies;
  const privateSaved = (name: string) => props.saved && (o.private_headers ?? []).some((p) => p.toLowerCase() === name.toLowerCase());

  return (
    <div className="request-form">
      <label className="field">
        <span>{r.userAgent}</span>
        <input type="text" value={o.user_agent ?? ""} placeholder={r.userAgentPlaceholder} spellCheck={false} onChange={(e) => set({ user_agent: blankToNull(e.target.value) })} />
      </label>
      <label className="field">
        <span>{r.referer}</span>
        <input type="text" value={o.referer ?? ""} placeholder="https://" spellCheck={false} onChange={(e) => set({ referer: blankToNull(e.target.value) })} />
      </label>
      <label className="field">
        <span>{r.cookies}</span>
        <textarea
          rows={2}
          value={draft.cookies}
          spellCheck={false}
          placeholder={cookiesSaved ? r.cookiesSaved : r.cookiesPlaceholder}
          onChange={(e) => props.onChange({ ...draft, cookies: e.target.value })}
        />
        {cookiesSaved && !draft.cookies && (
          <button type="button" className="link request-clear" onClick={() => props.onChange({ ...draft, clearCookies: true })}>
            {r.clearCookies}
          </button>
        )}
      </label>
      <div className="field">
        <span id="request-auth">{r.auth}</span>
        <Select
          aria-labelledby="request-auth"
          value={o.auth}
          onChange={(auth) => set({ auth })}
          options={AUTH_KINDS.map((k) => ({ value: k, label: r.authKinds[k] }))}
        />
      </div>
      {o.auth !== "none" && (
        <div className={`request-pair${o.auth === "bearer" ? " is-single" : ""}`}>
          {o.auth !== "bearer" && (
            <label className="field">
              <span>{r.username}</span>
              <input type="text" value={o.username} autoComplete="off" spellCheck={false} onChange={(e) => set({ username: e.target.value })} />
            </label>
          )}
          <label className="field">
            <span>{o.auth === "bearer" ? r.token : r.password}</span>
            <input
              type="password"
              value={draft.password}
              autoComplete="new-password"
              placeholder={passwordSaved ? r.secretSaved : ""}
              onChange={(e) => props.onChange({ ...draft, password: e.target.value })}
            />
            {passwordSaved && !draft.password && (
              <button type="button" className="link request-clear" onClick={() => props.onChange({ ...draft, clearPassword: true })}>
                {r.clearSecret}
              </button>
            )}
          </label>
        </div>
      )}
      <div className="field">
        <span>{r.headers}</span>
        {o.headers.map(([name, value], i) => (
          <div className="request-header" key={i}>
            <input
              type="text"
              value={name}
              placeholder={r.headerName}
              aria-label={r.headerName}
              spellCheck={false}
              onChange={(e) => set({ headers: o.headers.map((h, j) => (j === i ? [e.target.value, h[1]] : h)) })}
            />
            <input
              type="text"
              value={value}
              placeholder={privateSaved(name) ? r.privateKept : r.headerValue}
              aria-label={r.headerValue}
              spellCheck={false}
              onChange={(e) => set({ headers: o.headers.map((h, j) => (j === i ? [h[0], e.target.value] : h)) })}
            />
            <IconBtn icon={X} label={r.removeHeader(name)} onClick={() => set({ headers: o.headers.filter((_, j) => j !== i) })} />
          </div>
        ))}
        <button type="button" className="btn btn-small request-add" onClick={() => set({ headers: [...o.headers, ["", ""]] })}>
          <Plus size={14} strokeWidth={2.2} aria-hidden="true" /> {r.addHeader}
        </button>
        <small>{r.privateNote}</small>
      </div>
      <div className="request-pair">
        <label className="field">
          <span>{r.readTimeout}</span>
          <input type="number" min={1} max={600} value={o.read_timeout ?? ""} placeholder={r.timeoutPlaceholder(20)} onChange={(e) => set({ read_timeout: seconds(e.target.value) })} />
        </label>
        <label className="field">
          <span>{r.connectTimeout}</span>
          <input type="number" min={1} max={300} value={o.connect_timeout ?? ""} placeholder={r.timeoutPlaceholder(15)} onChange={(e) => set({ connect_timeout: seconds(e.target.value) })} />
        </label>
      </div>
    </div>
  );
}

/** The saved profiles, loaded once; null until known. */
export function useRequestProfiles(): [RequestBook | null, (b: RequestBook) => void] {
  const [book, setBook] = useState<RequestBook | null>(null);
  useEffect(() => {
    let live = true;
    api
      .requestProfiles()
      .then((b) => live && setBook(b))
      .catch(() => live && setBook({ next_id: 1, default: null, items: [] }));
    return () => {
      live = false;
    };
  }, []);
  return [book, setBook];
}

/** How a new download's requests are made. */
export type RequestChoice = { kind: "default" } | { kind: "profile"; id: number } | { kind: "custom"; draft: RequestDraft };

/** The parts of a new download that say so. */
export function requestOf(choice: RequestChoice): { request?: RequestOptions; request_profile?: number } {
  if (choice.kind === "profile") return { request_profile: choice.id };
  if (choice.kind === "custom") return { request: inlineOptions(choice.draft) };
  return {};
}

/** The Request part of the add dialog: the default, a profile, or options
 *  of its own. */
export function RequestField(props: { value: RequestChoice; onChange: (c: RequestChoice) => void }) {
  const [book] = useRequestProfiles();
  const r = t.request;
  const fallback = book?.items.find((p) => p.id === book.default)?.name ?? r.none;
  const value = props.value.kind === "profile" ? String(props.value.id) : props.value.kind;
  return (
    <>
      <div className="field">
        <span id="add-request">{r.field}</span>
        <Select
          aria-labelledby="add-request"
          value={value}
          onChange={(v) =>
            props.onChange(v === "default" ? { kind: "default" } : v === "custom" ? { kind: "custom", draft: draftOf() } : { kind: "profile", id: Number(v) })
          }
          options={[
            { value: "default", label: r.useDefault(fallback) },
            ...(book?.items ?? []).map((p) => ({ value: String(p.id), label: p.name })),
            { value: "custom", label: r.custom },
          ]}
        />
      </div>
      {props.value.kind === "custom" && (
        <RequestForm draft={props.value.draft} onChange={(draft) => props.onChange({ kind: "custom", draft })} />
      )}
    </>
  );
}

function summary(o: RequestOptions): string {
  const p = t.request.parts;
  return [
    o.auth !== "none" && t.request.authKinds[o.auth],
    o.user_agent && p.agent,
    o.referer && p.referer,
    o.has_cookies && p.cookies,
    o.headers.length > 0 && p.headers(o.headers.length),
    (o.read_timeout || o.connect_timeout) && p.timeouts,
  ]
    .filter(Boolean)
    .join(" · ");
}

/** Settings › Network › Request profiles. */
export function RequestProfiles() {
  const [book, setBook] = useRequestProfiles();
  const [editing, setEditing] = useState<RequestProfile | null>(null);
  const [error, setError] = useState("");
  if (!book) return <div className="setting-card" aria-busy="true" />;
  const r = t.request;

  async function setDefault(v: string) {
    setError("");
    try {
      setBook(await api.setDefaultRequestProfile(v === "none" ? null : Number(v)));
    } catch (e) {
      setError(String(e));
    }
  }

  return (
    <>
      <h3>{r.title}</h3>
      <div className="setting-card">
        <div className="setting-row">
          <div className="setting-text">
            <span id="set-request-default">{r.defaultProfile}</span>
            <small>{r.defaultHint}</small>
          </div>
          <div className="setting-control">
            <Select
              aria-labelledby="set-request-default"
              value={book.default == null ? "none" : String(book.default)}
              onChange={setDefault}
              options={[{ value: "none", label: r.none }, ...book.items.map((p) => ({ value: String(p.id), label: p.name }))]}
            />
          </div>
        </div>
        {book.items.length === 0 && (
          <div className="setting-row">
            <small className="proxy-empty">{r.noProfiles}</small>
          </div>
        )}
        {book.items.map((p) => (
          <div className="setting-row proxy-row" key={p.id}>
            <div className="setting-text">
              <span>
                {p.name}
                {book.default === p.id && <span className="soon">{r.isDefault}</span>}
              </span>
              <small>{summary(p.options) || r.none}</small>
            </div>
            <div className="setting-control">
              <button type="button" className="btn btn-small" aria-label={`${r.edit} ${p.name}`} onClick={() => setEditing(p)}>
                {r.edit}
              </button>
            </div>
          </div>
        ))}
        <div className="setting-row proxy-foot">
          <button type="button" className="btn btn-small" onClick={() => setEditing({ id: 0, name: "", options: BLANK_REQUEST })}>
            <Plus size={14} strokeWidth={2.2} aria-hidden="true" /> {r.add}
          </button>
        </div>
      </div>
      {error && <p className="proxy-error" role="alert">{error}</p>}
      <p className="proxy-limits">{r.hint}</p>
      {editing &&
        createPortal(
          <ProfileEditor
            profile={editing}
            onClose={() => setEditing(null)}
            onSaved={(b) => {
              setBook(b);
              setEditing(null);
            }}
          />,
          document.body,
        )}
    </>
  );
}

function ProfileEditor(props: { profile: RequestProfile; onClose: () => void; onSaved: (b: RequestBook) => void }) {
  const saved = props.profile.id !== 0;
  const [name, setName] = useState(props.profile.name);
  const [draft, setDraft] = useState(() => draftOf(props.profile.options));
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const r = t.request;

  async function save(e: FormEvent) {
    e.preventDefault();
    if (busy) return;
    setBusy(true);
    setError("");
    try {
      const options: RequestOptions = { ...draft.options, password: null, cookies: null };
      props.onSaved(
        await api.saveRequestProfile(props.profile.id, name, options, change(draft.password, draft.clearPassword), change(draft.cookies, draft.clearCookies)),
      );
    } catch (err) {
      setError(String(err));
      setBusy(false);
    }
  }

  async function remove() {
    setBusy(true);
    try {
      props.onSaved(await api.deleteRequestProfile(props.profile.id));
    } catch (err) {
      setError(String(err));
      setConfirmDelete(false);
      setBusy(false);
    }
  }

  return (
    <Modal
      title={saved ? props.profile.name : r.add}
      onClose={props.onClose}
      footer={
        confirmDelete ? (
          <>
            <span className="proxy-confirm">{r.deleteConfirm(props.profile.name)}</span>
            <span className="spacer" />
            <button type="button" className="btn" data-autofocus onClick={() => setConfirmDelete(false)}>
              {t.dialogs.cancel}
            </button>
            <button type="button" className="btn btn-danger-solid" disabled={busy} onClick={remove}>
              {r.delete}
            </button>
          </>
        ) : (
          <>
            {saved && (
              <button type="button" className="btn" onClick={() => setConfirmDelete(true)}>
                {r.delete}
              </button>
            )}
            <span className="spacer" />
            <button type="button" className="btn" onClick={props.onClose}>
              {t.dialogs.cancel}
            </button>
            <button type="submit" form="request-profile-form" className="btn btn-primary" disabled={busy}>
              {r.save}
            </button>
          </>
        )
      }
    >
      <form id="request-profile-form" className="form" onSubmit={save} autoComplete="off">
        <label className="field">
          <span>{r.name}</span>
          <input data-autofocus type="text" value={name} maxLength={60} placeholder={r.namePlaceholder} onChange={(e) => setName(e.target.value)} />
        </label>
        <RequestForm draft={draft} onChange={setDraft} saved={saved} />
        {error && (
          <small className="field-error" role="alert">
            {error}
          </small>
        )}
      </form>
    </Modal>
  );
}
