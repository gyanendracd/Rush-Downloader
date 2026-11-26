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
  etaOf,
  fileSrc,
  formatBytes,
  formatDate,
  formatDuration,
  formatSpeed,
  percentOf,
  pictureKind,
  STATUS_LABEL,
  type CurrentFile,
  type Download,
  type DownloadItem,
} from "./api";
import { amountOf, FileIcon, Modal, Progress, Sparkline, useApp } from "./ui";

type Tab = "progress" | "details" | "options";
const TABS: { id: Tab; label: string }[] = [
  { id: "progress", label: "Progress" },
  { id: "details", label: "Details" },
  { id: "options", label: "Options" },
];

function Fact({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="fact">
      <dt>{label}</dt>
      <dd>{children}</dd>
    </div>
  );
}

/** The one file a profile or playlist download is fetching right now. */
function CurrentFileProgress({ file }: { file: CurrentFile }) {
  const known = file.total != null && file.total > 0;
  const pct = known ? Math.min(100, Math.floor((file.downloaded / file.total!) * 100)) : null;
  const place =
    file.index != null ? (file.count != null ? `${file.index} of ${file.count}` : `No. ${file.index}`) : null;
  return (
    <section className="now-file" aria-label="File downloading now">
      <div className="now-file-head">
        <span>Downloading now{place ? ` · ${place}` : ""}</span>
        <span>
          {known ? `${formatBytes(file.downloaded)} / ${formatBytes(file.total!)}` : formatBytes(file.downloaded)}
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
            aria-label={`${file.name} progress`}
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={pct}
          >
            <div className="fill" style={{ width: `${pct}%` }} />
          </div>
        ) : (
          <div className="bar bar-tall bar-downloading bar-busy" />
        )}
        <span>{pct != null ? `${pct}%` : ""}</span>
      </div>
    </section>
  );
}

const ITEM_STATE: Record<DownloadItem["state"], string> = {
  done: "Downloaded",
  downloading: "Downloading",
  waiting: "Waiting",
  missed: "Not available",
  deleted: "Deleted",
};

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
        <Fact label="Status">
          {ITEM_STATE[item.state]}
          {props.pct != null ? ` · ${props.pct}%` : ""}
        </Fact>
        <Fact label="Position">{props.place}</Fact>
        {item.duration != null && <Fact label="Length">{formatDuration(item.duration)}</Fact>}
        {item.channel && <Fact label="Channel">{item.channel}</Fact>}
        {item.size != null && (
          <Fact label={item.state === "downloading" ? "Downloaded" : "Size"}>{formatBytes(item.size)}</Fact>
        )}
        {item.file && item.file !== item.title && <Fact label="File">{item.file}</Fact>}
        {item.url && <Fact label="Link">{item.url.replace(/^https?:\/\/(www\.)?/i, "")}</Fact>}
      </dl>
    </div>,
    document.body,
  );
}

/** More rows than this are counted, not drawn. */
const ITEMS_SHOWN = 500;

/** Every video or file of a profile or playlist download, with how far each is. */
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
          <span>Getting the list</span>
          <div className="bar bar-downloading bar-busy" />
        </div>
      </div>
    ) : null;
  }
  const done = items.filter((item) => item.state === "done").length;
  const current = d.current;
  const pctOf = (item: DownloadItem) =>
    item.state === "downloading" && current?.total
      ? Math.min(100, Math.floor((current.downloaded / current.total) * 100))
      : null;
  const shown = pointed && items[pointed.index];
  return (
    <section className="items" aria-label="What this download holds">
      <div className="items-head">
        <span>{items.length === 1 ? "1 item" : `${items.length} items`}</span>
        <span>{done} done</span>
      </div>
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
              <span className="item-title">{item.title}</span>
              <span className="item-state">
                {item.state === "done" ? (
                  <>
                    {item.size != null ? `${formatBytes(item.size)} ` : ""}
                    <Check size={14} strokeWidth={2.5} aria-label="Done" />
                  </>
                ) : item.state === "downloading" ? (
                  pct != null ? `${pct}%` : "Downloading"
                ) : item.state === "missed" ? (
                  "Not available"
                ) : item.state === "deleted" ? (
                  "Deleted"
                ) : (
                  "Waiting"
                )}
              </span>
            </li>
          );
        })}
      </ol>
      {items.length > ITEMS_SHOWN && <p className="items-more">and {items.length - ITEMS_SHOWN} more</p>}
      {pointed && shown && (
        <ItemCard item={shown} place={`${pointed.index + 1} of ${items.length}`} pct={pctOf(shown)} row={pointed.row} />
      )}
    </section>
  );
}

/** The detailed view of one download, opened by clicking its row. */
export function DetailDialog({ d, onClose }: { d: Download; onClose: () => void }) {
  const { history, actions } = useApp();
  const [tab, setTab] = useState<Tab>("progress");
  const eta = etaOf(d);
  const pct = percentOf(d);
  const running = d.status === "downloading";
  const segmented = d.segments.length > 0 && !!d.total;

  const headline =
    d.status === "completed"
      ? `Completed · ${d.total != null ? formatBytes(d.total) : ""}`
      : running
        ? `${amountOf(d)} · ${formatSpeed(d.speed)}`
        : `${STATUS_LABEL[d.status]} · ${amountOf(d)}`;

  return (
    <Modal
      bare
      title={`Details for ${d.filename}`}
      onClose={onClose}
      footer={
        d.status === "completed" ? (
          <>
            <button type="button" className="btn btn-wide" onClick={() => actions.open(d)}>
              <SquareArrowOutUpRight size={16} strokeWidth={2} aria-hidden="true" /> Open
            </button>
            <button type="button" className="btn btn-wide" onClick={() => actions.reveal(d)}>
              <FolderOpen size={16} strokeWidth={2} aria-hidden="true" /> Show in folder
            </button>
            <button type="button" className="btn btn-wide btn-danger" onClick={() => actions.remove([d.id])}>
              <X size={16} strokeWidth={2} aria-hidden="true" /> Remove
            </button>
          </>
        ) : (
          <>
            {/* One button that follows the download: Pause while it runs,
                Resume once paused, Retry after a failure. */}
            {canPause(d) ? (
              <button type="button" className="btn btn-wide" onClick={() => actions.pause([d.id])}>
                <Pause size={15} strokeWidth={0} fill="currentColor" aria-hidden="true" /> Pause
              </button>
            ) : (
              <button type="button" className="btn btn-wide btn-primary" onClick={() => actions.resume([d.id])}>
                {d.status === "failed" ? (
                  <RotateCw size={16} strokeWidth={2} aria-hidden="true" />
                ) : (
                  <Play size={15} strokeWidth={0} fill="currentColor" aria-hidden="true" />
                )}
                {d.status === "failed" ? "Retry" : "Resume"}
              </button>
            )}
            <button type="button" className="btn btn-wide btn-danger" onClick={() => actions.remove([d.id])}>
              <Trash2 size={16} strokeWidth={2} aria-hidden="true" /> Cancel
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
            <span>{eta || (pct != null ? `${pct}%` : "")}</span>
          </div>
        </div>
      </div>

      <div className="tabs-line" role="tablist">
        {TABS.map((t) => (
          <button
            key={t.id}
            type="button"
            role="tab"
            aria-selected={tab === t.id}
            className={tab === t.id ? "is-active" : undefined}
            onClick={() => setTab(t.id)}
          >
            {t.label}
          </button>
        ))}
      </div>

      {tab === "progress" && (
        <>
          {d.status === "failed" && d.error && (
            <div className="callout callout-error">
              <strong>Why it failed</strong>
              <p>{d.error}</p>
            </div>
          )}
          {d.status !== "completed" && (
            <Sparkline
              values={history.get(d.id) ?? []}
              label="Download speed over the last minute"
              now={running ? formatSpeed(d.speed) : undefined}
              speed={running ? d.speed : 0}
            />
          )}
          {running && d.current && <CurrentFileProgress file={d.current} />}
          <dl className="facts">
            <Fact label="Save Location">{d.dir}</Fact>
            <Fact label="File Size">{d.total != null ? formatBytes(d.total) : "Unknown"}</Fact>
            <Fact label="Downloaded">{formatBytes(d.downloaded)}</Fact>
            <Fact label="Time Remaining">
              {d.status === "completed" ? "Done" : eta.replace(" left", "") || "—"}
            </Fact>
          </dl>
        </>
      )}

      {tab === "details" && (
        <>
          <dl className="facts">
            <Fact label="Link"><span className="selectable break">{d.url}</span></Fact>
            <Fact label="Saved As"><span className="selectable break">{d.path}</span></Fact>
            <Fact label="Added">{formatDate(d.created_at)}</Fact>
            <Fact label="Source">{d.profile ? "Profile or playlist" : d.torrent ? "Torrent" : d.media ? "Video page" : "Direct file link"}</Fact>
            <Fact label="Pause and Resume">
              {d.media || d.resumable
                ? "Supported"
                : d.status === "queued" || d.status === "connecting"
                  ? "Not known yet"
                  : "Not supported by this server"}
            </Fact>
            <Fact label="Connections">
              {d.torrent
                ? "Shared between peers by the torrent engine"
                : d.media
                ? "Handled by the site backend"
                : running
                  ? `${d.active_connections} of ${d.connections}`
                  : `Up to ${d.connections}`}
            </Fact>
          </dl>
          {d.profile && <ItemList d={d} />}
          {segmented && (
            <ol className="segments" aria-label="Progress of each connection">
              {d.segments.map(([start, pos, end], i) => {
                const size = Math.max(1, end - start);
                const done = Math.min(100, Math.floor(((pos - start) / size) * 100));
                return (
                  <li key={i}>
                    <span className="segment-index">{i + 1}</span>
                    <div className="segment-main">
                      <div className={`bar bar-${done >= 100 ? "completed" : d.status}`}>
                        <div className="fill" style={{ width: `${done}%` }} />
                      </div>
                    </div>
                    <span className="segment-pct">{done}%</span>
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
                <SquareArrowOutUpRight size={17} strokeWidth={2} aria-hidden="true" /> Open file
              </button>
              <button type="button" onClick={() => actions.reveal(d)}>
                <FolderOpen size={17} strokeWidth={2} aria-hidden="true" /> Show in folder
              </button>
              <button type="button" onClick={() => actions.copy(d.path, "File path")}>
                <Copy size={17} strokeWidth={2} aria-hidden="true" /> Copy file path
              </button>
            </>
          )}
          <button type="button" onClick={() => actions.copy(d.url, "Link")}>
            <LinkGlyph size={17} strokeWidth={2} aria-hidden="true" /> Copy link
          </button>
          {d.media && d.status === "failed" && (
            <button
              type="button"
              onClick={() => {
                onClose();
                actions.scan(d.url);
              }}
            >
              <ScanSearch size={17} strokeWidth={2} aria-hidden="true" /> Find media on this page
            </button>
          )}
          {d.status === "completed" && (
            <button type="button" className="is-danger" onClick={() => actions.deleteFile(d)}>
              <Trash2 size={17} strokeWidth={2} aria-hidden="true" /> Delete file from disk
            </button>
          )}
        </div>
      )}
    </Modal>
  );
}
