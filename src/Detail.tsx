import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import {
  Check,
  Copy,
  File as FileGlyph,
  Film,
  FolderOpen,
  Image as ImageGlyph,
  Link as LinkGlyph,
  Pause,
  Play,
  RotateCw,
  ScanSearch,
  SquareArrowOutUpRight,
  Trash2,
  X,
} from "lucide-react";
import {
  api,
  canPause,
  displayName,
  checkingText,
  etaOf,
  fileSrc,
  formatBytes,
  formatDate,
  formatDuration,
  formatSpeed,
  pictureKind,
  STATUS_LABEL,
  type CurrentFile,
  type Download,
  type DownloadItem,
  type FileNet,
} from "./api";
import { amountOf, FileIcon, Modal, Progress, Sparkline, useApp } from "./ui";
import { pct as pctText, t } from "./i18n";

type Tab = "progress" | "details" | "options";
const TABS: Tab[] = ["progress", "details", "options"];

/** A time left without its "left" word: "3h 0m 12s". */
function withoutLeft(eta: string): string {
  const [before, after = ""] = t.common.eta.left("\u0000").split("\u0000");
  let rest = eta;
  if (before && rest.startsWith(before)) rest = rest.slice(before.length);
  if (after && rest.endsWith(after)) rest = rest.slice(0, -after.length);
  return rest;
}

function Fact({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="fact">
      <dt>{label}</dt>
      <dd>{children}</dd>
    </div>
  );
}

/** Decimals the detail shows a percentage with: a 100 GB file moves by a
 *  whole percent only every gigabyte. */
const PCT_DIGITS = 3;

/** How much of `whole` is `part`, from 0 to 100, cut (not rounded) to
 *  PCT_DIGITS decimals, so an unfinished file never reads 100.000%. */
function exactPct(part: number, whole: number): number {
  const scale = 10 ** PCT_DIGITS;
  return Math.min(100, Math.floor((part / whole) * 100 * scale) / scale);
}

/** "12 of 40 peers · 3 seeders · ↓ 1.2 MB/s · ↑ 200 KB/s": where a torrent
 *  file's speed comes from, and how much of the line uploading takes. */
function netLine(net: FileNet): string {
  return [
    t.detail.items.peersOf(net.peers, net.found),
    t.detail.items.seeders(net.seeders),
    `↓ ${formatSpeed(net.down)}`,
    `↑ ${formatSpeed(net.up)}`,
  ].join(" · ");
}

/** The one file a profile or playlist download is fetching right now. */
function CurrentFileProgress({ file }: { file: CurrentFile }) {
  const known = file.total != null && file.total > 0;
  const pct = known ? exactPct(file.downloaded, file.total!) : null;
  const place =
    file.index != null
      ? file.count != null
        ? t.detail.placeOf(file.index, file.count)
        : t.detail.now.number(file.index)
      : null;
  return (
    <section className="now-file" aria-label={t.detail.now.label}>
      <div className="now-file-head">
        <span>{place ? t.detail.now.headingAt(place) : t.detail.now.heading}</span>
        <span>
          {known ? t.detail.now.amount(formatBytes(file.downloaded), formatBytes(file.total!)) : formatBytes(file.downloaded)}
        </span>
      </div>
      <p className="now-file-name" title={file.name}>
        {file.name}
      </p>
      <div className="now-file-bar">
        {pct != null ? (
          <div
            className={`bar bar-tall bar-${pct >= 100 ? "completed" : "downloading"}`}
            role="progressbar"
            aria-label={t.detail.now.progressOf(file.name)}
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={pct}
          >
            <div className="fill" style={{ width: `${pct}%` }} />
          </div>
        ) : (
          <div className="bar bar-tall bar-downloading bar-busy" />
        )}
        <span>{pct != null ? pctText(pct, PCT_DIGITS) : ""}</span>
      </div>
    </section>
  );
}

/** True once the element has come into view (or at once, when not `wanted`). */
function useSeen<T extends Element>(wanted: boolean): [React.RefObject<T | null>, boolean] {
  const place = useRef<T | null>(null);
  const [seen, setSeen] = useState(false);
  useEffect(() => {
    const el = place.current;
    if (!wanted || seen || !el) return;
    const watch = new IntersectionObserver(
      (entries) => {
        if (entries.some((entry) => entry.isIntersecting)) setSeen(true);
      },
      // A little ahead of the scroll, so pictures are there on arrival.
      { rootMargin: "120px" },
    );
    watch.observe(el);
    return () => watch.disconnect();
  }, [wanted, seen]);
  return [place, seen];
}

/** A small picture of one item: the site's own, else the file itself, else a sign of its kind. */
export function ItemPicture({ item, large }: { item: DownloadItem; large?: boolean }) {
  const [broken, setBroken] = useState(false);
  const cls = `item-thumb${large ? " item-thumb-large" : ""}`;
  // The site's picture where there is one; otherwise the file itself, once it
  // is on this computer: a photo as it is, a video by its first frame.
  const kind = item.path ? pictureKind(item.file ?? item.path) : null;
  const local = !item.thumbnail && item.path && kind ? fileSrc(item.path) : null;
  // A row's own file is only opened once the row has come into view: a long
  // list would otherwise read every photo and video in the folder at once.
  const [place, seen] = useSeen<HTMLSpanElement>(!!local);
  if (item.thumbnail && !broken) {
    return (
      <img
        className={cls}
        src={item.thumbnail}
        alt=""
        loading="lazy"
        referrerPolicy="no-referrer"
        onError={() => setBroken(true)}
      />
    );
  }
  if (local && !broken && seen) {
    return kind === "video" ? (
      <video
        className={cls}
        // "#t=0.1": just past the start, where the first real frame is.
        src={`${local}#t=0.1`}
        preload="metadata"
        muted
        playsInline
        disablePictureInPicture
        tabIndex={-1}
        aria-hidden="true"
        onError={() => setBroken(true)}
      />
    ) : (
      <img className={cls} src={local} alt="" decoding="async" onError={() => setBroken(true)} />
    );
  }
  if (local && !broken) {
    return <span ref={place} className={`${cls} item-thumb-empty shimmer`} aria-hidden="true" />;
  }
  const ext = /\.([a-z0-9]{2,5})$/i.exec(item.file ?? item.title)?.[1].toLowerCase() ?? "";
  const Glyph = /^(jpe?g|png|gif|webp|bmp|avif|heic|svg)$/.test(ext) ? ImageGlyph : item.file && !/^(mp4|mkv|webm|mov|m4v)$/.test(ext) ? FileGlyph : Film;
  return (
    <span className={`${cls} item-thumb-empty`} aria-hidden="true">
      <Glyph size={large ? 28 : 14} strokeWidth={2} />
    </span>
  );
}

/** Everything known about one item, shown beside its row while it is pointed at. */
function ItemCard(props: { item: DownloadItem; place: string; pct: number | null; row: DOMRect }) {
  const { item, row } = props;
  const WIDTH = 300;
  const left = Math.max(8, Math.min(row.left + 36, window.innerWidth - WIDTH - 8));
  // Below the row where there is room, else above it.
  const below = window.innerHeight - row.bottom > 330;
  const style: React.CSSProperties = below
    ? { left, top: row.bottom + 6, width: WIDTH }
    : { left, bottom: window.innerHeight - row.top + 6, width: WIDTH };
  return createPortal(
    <div className="item-card" role="tooltip" style={style}>
      {(item.thumbnail || (item.path && pictureKind(item.file ?? item.path) && fileSrc(item.path))) && (
        <ItemPicture item={item} large />
      )}
      <strong>{item.title}</strong>
      <dl>
        <Fact label={t.detail.card.status}>
          {props.pct != null
            ? t.detail.card.stateWithPct(t.detail.items.state[item.state], pctText(props.pct, PCT_DIGITS))
            : t.detail.items.state[item.state]}
        </Fact>
        <Fact label={t.detail.card.position}>{props.place}</Fact>
        {item.duration != null && <Fact label={t.detail.card.length}>{formatDuration(item.duration)}</Fact>}
        {item.channel && <Fact label={t.detail.card.channel}>{item.channel}</Fact>}
        {item.downloaded != null && item.size != null ? (
          <Fact label={t.detail.card.downloaded}>
            {formatBytes(item.downloaded)} / {formatBytes(item.size)}
          </Fact>
        ) : (
          item.size != null && (
            <Fact label={item.state === "downloading" ? t.detail.card.downloaded : t.detail.card.size}>
              {formatBytes(item.size)}
            </Fact>
          )
        )}
        {item.file && item.file !== item.title && <Fact label={t.detail.card.file}>{item.file}</Fact>}
        {item.url && <Fact label={t.detail.card.link}>{item.url.replace(/^https?:\/\/(www\.)?/i, "")}</Fact>}
      </dl>
    </div>,
    document.body,
  );
}

/** More rows than this are counted, not drawn. */
const ITEMS_SHOWN = 500;

/** Every video or file of a profile, playlist or torrent download, with how
 *  far each is. A torrent's files all download at once, each with its bar. */
function ItemList({ d }: { d: Download }) {
  const [items, setItems] = useState<DownloadItem[] | null>(null);
  /** The row under the pointer (or focused), and where it is on screen. */
  const [pointed, setPointed] = useState<{ index: number; row: DOMRect } | null>(null);
  const live = d.status === "downloading" || d.status === "connecting";
  useEffect(() => {
    let stale = false;
    const load = () =>
      api
        .items(d.id)
        .then((found) => !stale && setItems(found))
        .catch(() => !stale && setItems((had) => had ?? []));
    load();
    // While it runs the list keeps moving; otherwise one look is enough.
    const timer = live ? window.setInterval(load, 1000) : undefined;
    return () => {
      stale = true;
      window.clearInterval(timer);
    };
  }, [d.id, d.status, live]);

  if (items == null) {
    return (
      <div className="items items-loading" aria-hidden="true">
        {[72, 58, 66].map((width, i) => (
          <span key={i} className="shimmer" style={{ width: `${width}%` }} />
        ))}
      </div>
    );
  }
  if (items.length === 0) {
    return live ? (
      <div className="items">
        <div className="card-loading" role="status">
          <span>{t.detail.items.gettingList}</span>
          <div className="bar bar-downloading bar-busy" />
        </div>
      </div>
    ) : null;
  }
  // A torrent of one file has nothing to list: its bar is the download's own.
  if (d.torrent && items.length < 2) return null;
  const done = items.filter((item) => item.state === "done").length;
  const current = d.current;
  const pctOf = (item: DownloadItem) =>
    item.downloaded != null && item.size
      ? exactPct(item.downloaded, item.size)
      : item.state === "downloading" && current?.total
        ? exactPct(current.downloaded, current.total)
        : null;
  const shown = pointed && items[pointed.index];
  // Every file's engine together, while a torrent downloads file by file.
  const nets = items.map((item) => item.net).filter((net): net is FileNet => net != null);
  const total =
    nets.length > 0
      ? nets.reduce((sum, net) => ({
          peers: sum.peers + net.peers,
          seeders: sum.seeders + net.seeders,
          found: sum.found + net.found,
          down: sum.down + net.down,
          up: sum.up + net.up,
        }))
      : null;
  return (
    <section className="items" aria-label={t.detail.items.label}>
      <div className="items-head">
        <span>{d.torrent ? t.ui.files(items.length) : t.detail.items.count(items.length)}</span>
        <span>{t.detail.items.done(done)}</span>
      </div>
      {total && <p className="items-net">{netLine(total)}</p>}
      <ol className="items-list" onScroll={() => setPointed(null)}>
        {items.slice(0, ITEMS_SHOWN).map((item, i) => {
          const pct = pctOf(item);
          const point = (e: { currentTarget: HTMLElement }) =>
            setPointed({ index: i, row: e.currentTarget.getBoundingClientRect() });
          return (
            <li
              key={i}
              className={`item item-${item.state}`}
              tabIndex={0}
              onMouseEnter={point}
              onMouseLeave={() => setPointed(null)}
              onFocus={point}
              onBlur={() => setPointed(null)}
            >
              <span className="item-index">{i + 1}</span>
              <ItemPicture item={item} />
              <span className="item-main">
                <span className="item-title">{item.title}</span>
                {item.downloaded != null && item.size != null && (
                  <span className="item-bar" aria-hidden="true">
                    <span style={{ width: `${pct ?? 0}%` }} />
                  </span>
                )}
                {item.net && <span className="item-net">{netLine(item.net)}</span>}
              </span>
              <span className="item-state">
                {item.state === "done" ? (
                  <>
                    {item.size != null ? `${formatBytes(item.size)} ` : ""}
                    <Check size={14} strokeWidth={2.5} aria-label={t.detail.items.doneMark} />
                  </>
                ) : pct != null && (item.state === "downloading" || pct > 0) ? (
                  pctText(pct, PCT_DIGITS)
                ) : item.state === "downloading" ? (
                  t.detail.items.state.downloading
                ) : (
                  t.detail.items.state[item.state]
                )}
              </span>
            </li>
          );
        })}
      </ol>
      {items.length > ITEMS_SHOWN && <p className="items-more">{t.detail.items.more(items.length - ITEMS_SHOWN)}</p>}
      {pointed && shown && (
        <ItemCard item={shown} place={t.detail.placeOf(pointed.index + 1, items.length)} pct={pctOf(shown)} row={pointed.row} />
      )}
    </section>
  );
}

/** The detailed view of one download, opened by clicking its row. */
export function DetailDialog({ d, onClose }: { d: Download; onClose: () => void }) {
  const { history, actions } = useApp();
  const [tab, setTab] = useState<Tab>("progress");
  const eta = etaOf(d);
  const pct = d.status === "completed" ? 100 : d.total ? exactPct(d.downloaded, d.total) : null;
  const running = d.status === "downloading";
  const segmented = d.segments.length > 0 && !!d.total;

  const headline =
    d.status === "completed"
      ? t.detail.headline.completed(d.total != null ? formatBytes(d.total) : "")
      : running
        ? t.detail.headline.running(amountOf(d), formatSpeed(d.speed))
        : checkingText(d)
          ? t.detail.headline.checking(checkingText(d)!, amountOf(d))
          : t.detail.headline.status(STATUS_LABEL[d.status], amountOf(d));

  return (
    <Modal
      bare
      title={t.detail.title(d.filename)}
      onClose={onClose}
      footer={
        d.status === "completed" ? (
          <>
            <button type="button" className="btn btn-wide" onClick={() => actions.open(d)}>
              <SquareArrowOutUpRight size={16} strokeWidth={2} aria-hidden="true" /> {t.detail.buttons.open}
            </button>
            <button type="button" className="btn btn-wide" onClick={() => actions.reveal(d)}>
              <FolderOpen size={16} strokeWidth={2} aria-hidden="true" /> {t.detail.buttons.showInFolder}
            </button>
            <button type="button" className="btn btn-wide btn-danger" onClick={() => actions.remove([d.id])}>
              <X size={16} strokeWidth={2} aria-hidden="true" /> {t.detail.buttons.remove}
            </button>
          </>
        ) : (
          <>
            {/* One button that follows the download: Pause while it runs,
                Resume once paused, Retry after a failure. */}
            {canPause(d) ? (
              <button type="button" className="btn btn-wide" onClick={() => actions.pause([d.id])}>
                <Pause size={15} strokeWidth={0} fill="currentColor" aria-hidden="true" /> {t.detail.buttons.pause}
              </button>
            ) : (
              <button type="button" className="btn btn-wide btn-primary" onClick={() => actions.resume([d.id])}>
                {d.status === "failed" ? (
                  <RotateCw size={16} strokeWidth={2} aria-hidden="true" />
                ) : (
                  <Play size={15} strokeWidth={0} fill="currentColor" aria-hidden="true" />
                )}
                {d.status === "failed" ? t.detail.buttons.retry : t.detail.buttons.resume}
              </button>
            )}
            <button type="button" className="btn btn-wide btn-danger" onClick={() => actions.remove([d.id])}>
              <Trash2 size={16} strokeWidth={2} aria-hidden="true" /> {t.detail.buttons.cancel}
            </button>
          </>
        )
      }
    >
      <div className="detail-head">
        <FileIcon d={d} large />
        <div className="detail-title">
          <h2 title={displayName(d)}>{displayName(d)}</h2>
          <p className={`detail-headline detail-${d.status}`}>{headline}</p>
          <div className="detail-progress">
            <Progress d={d} tall />
            {/* How far, to three decimals, and the time left beside it while it runs. */}
            <span>{[pct != null ? pctText(pct, PCT_DIGITS) : "", eta].filter(Boolean).join(" · ")}</span>
          </div>
        </div>
      </div>

      <div className="tabs-line" role="tablist">
        {TABS.map((id) => (
          <button
            key={id}
            type="button"
            role="tab"
            aria-selected={tab === id}
            className={tab === id ? "is-active" : undefined}
            onClick={() => setTab(id)}
          >
            {t.detail.tabs[id]}
          </button>
        ))}
      </div>

      {tab === "progress" && (
        <>
          {d.status === "failed" && d.error && (
            <div className="callout callout-error">
              <strong>{t.detail.progress.whyFailed}</strong>
              <p>{d.error}</p>
            </div>
          )}
          {d.status !== "completed" && (
            <Sparkline
              values={history.get(d.id) ?? []}
              label={t.detail.progress.speedChart}
              now={running ? formatSpeed(d.speed) : undefined}
              speed={running ? d.speed : 0}
            />
          )}
          {running && d.current && <CurrentFileProgress file={d.current} />}
          <dl className="facts">
            <Fact label={t.detail.progress.saveLocation}>{d.dir}</Fact>
            <Fact label={t.detail.progress.fileSize}>
              {d.total != null ? formatBytes(d.total) : t.detail.progress.unknown}
            </Fact>
            <Fact label={t.detail.progress.downloaded}>{formatBytes(d.downloaded)}</Fact>
            <Fact label={t.detail.progress.timeRemaining}>
              {d.status === "completed" ? t.detail.progress.done : withoutLeft(eta) || "—"}
            </Fact>
          </dl>
          {/* A torrent's files fill side by side: each one's bar, under the whole. */}
          {d.torrent && <ItemList d={d} />}
        </>
      )}

      {tab === "details" && (
        <>
          <dl className="facts">
            <Fact label={t.detail.details.link}><span className="selectable break">{d.url}</span></Fact>
            <Fact label={t.detail.details.savedAs}><span className="selectable break">{d.path}</span></Fact>
            <Fact label={t.detail.details.added}>{formatDate(d.created_at)}</Fact>
            <Fact label={t.detail.details.source}>
              {d.profile
                ? t.detail.details.sourceProfile
                : d.torrent
                  ? t.detail.details.sourceTorrent
                  : d.media
                    ? t.detail.details.sourceMedia
                    : t.detail.details.sourceDirect}
            </Fact>
            <Fact label={t.detail.details.pauseResume}>
              {d.media || d.resumable
                ? t.detail.details.supported
                : d.status === "queued" || d.status === "connecting"
                  ? t.detail.details.notKnownYet
                  : t.detail.details.notSupported}
            </Fact>
            <Fact label={t.detail.details.connections}>
              {d.torrent
                ? t.detail.details.connectionsTorrent
                : d.media
                ? t.detail.details.connectionsMedia
                : running
                  ? t.detail.details.connectionsActive(d.active_connections, d.connections)
                  : t.detail.details.connectionsUpTo(d.connections)}
            </Fact>
          </dl>
          {d.profile && <ItemList d={d} />}
          {segmented && (
            <ol className="segments" aria-label={t.detail.details.segments}>
              {d.segments.map(([start, pos, end], i) => {
                const size = Math.max(1, end - start);
                const done = exactPct(pos - start, size);
                return (
                  <li key={i}>
                    <span className="segment-index">{i + 1}</span>
                    <div className="segment-main">
                      <div className={`bar bar-${done >= 100 ? "completed" : d.status}`}>
                        <div className="fill" style={{ width: `${done}%` }} />
                      </div>
                    </div>
                    <span className="segment-pct">{pctText(done, PCT_DIGITS)}</span>
                  </li>
                );
              })}
            </ol>
          )}
        </>
      )}

      {tab === "options" && (
        <div className="option-list">
          {d.status === "completed" && (
            <>
              <button type="button" onClick={() => actions.open(d)}>
                <SquareArrowOutUpRight size={17} strokeWidth={2} aria-hidden="true" /> {t.detail.options.openFile}
              </button>
              <button type="button" onClick={() => actions.reveal(d)}>
                <FolderOpen size={17} strokeWidth={2} aria-hidden="true" /> {t.detail.options.showInFolder}
              </button>
              <button type="button" onClick={() => actions.copy(d.path, t.detail.options.filePath)}>
                <Copy size={17} strokeWidth={2} aria-hidden="true" /> {t.detail.options.copyFilePath}
              </button>
            </>
          )}
          <button type="button" onClick={() => actions.copy(d.url, t.detail.options.link)}>
            <LinkGlyph size={17} strokeWidth={2} aria-hidden="true" /> {t.detail.options.copyLink}
          </button>
          {d.media && d.status === "failed" && (
            <button
              type="button"
              onClick={() => {
                onClose();
                actions.scan(d.url);
              }}
            >
              <ScanSearch size={17} strokeWidth={2} aria-hidden="true" /> {t.detail.options.findMedia}
            </button>
          )}
          {d.status === "completed" && (
            <button type="button" className="is-danger" onClick={() => actions.deleteFile(d)}>
              <Trash2 size={17} strokeWidth={2} aria-hidden="true" /> {t.detail.options.deleteFile}
            </button>
          )}
        </div>
      )}
    </Modal>
  );
}
