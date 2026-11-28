import {
  createContext,
  useContext,
  useEffect,
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type FormEvent,
  type MouseEvent as ReactMouseEvent,
  type ReactNode,
} from "react";
import { createPortal } from "react-dom";
import {
  Archive,
  Check,
  Clock,
  ArrowRight,
  CalendarDays,
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  CircleAlert,
  CircleCheck,
  Copy,
  Ellipsis,
  FolderInput,
  Gauge,
  Layers,
  File as FileGlyph,
  FileText,
  Film,
  FolderOpen,
  Globe,
  Image as ImageGlyph,
  Link as LinkGlyph,
  Music,
  Package,
  Pause,
  Play,
  RotateCw,
  ScanSearch,
  SquareArrowOutUpRight,
  CalendarClock,
  Timer,
  Trash2,
  X,
  type LucideIcon,
} from "lucide-react";
import {
  api,
  canPause,

  deletedAgo,
  displayName,
  isDeleted,
  checkingText,
  checkPercent,
  etaOf,
  formatBytes,
  formatSpeed,
  formatTook,
  percentOf,
  STATUS_LABEL,
  takesNewLink,
  type Category,
  type Download,
  type LinkCheck,
  type MediaOptions,
  type Settings,
} from "./api";
import { DocIcon } from "./DocIcon";
import { BatchMeta, BatchTile } from "./Batch";
import { dateText, t } from "./i18n";
import { formatLocale } from "./i18n/format";

/* ---------- shared app state ---------- */

export interface Actions {
  pause: (ids: number[]) => void;
  resume: (ids: number[]) => void;
  /** Removes from the list; unfinished downloads are cancelled after a confirmation. */
  remove: (ids: number[]) => void;
  deleteFile: (d: Download) => void;
  open: (d: Download) => void;
  reveal: (d: Download) => void;
  copy: (text: string, what: string) => void;
  scan: (url: string) => void;
  inspect: (id: number) => void;
  openAdd: (url?: string, mode?: "download" | "scan") => void;
  /** Opens the Batch Download dialog. */
  openBatch: () => void;
  /** Opens the torrent dialog (folder and file selection) for a magnet link,
   *  a link to a .torrent file, or a .torrent file's path. */
  openTorrent: (source: string) => void;
  /** Opens a page on the Browser page, where a playing video can be caught.
   *  With `fetch`, the app also starts the player, so the video's qualities
   *  show up by themselves. Downloading stays the user's choice. */
  browse: (url: string, fetch?: boolean) => void;
  /** Asks before a link that is already downloading is started again.
   *  "go": start it; "show": the user opened the running one instead;
   *  "cancel": do nothing. */
  checkDuplicate: (link: string) => Promise<"go" | "show" | "cancel">;
  /** Starts whatever the text is: torrents go to the torrent dialog, every
   *  other link starts downloading. Resolves false when nothing was started. */
  submitLink: (text: string, media?: MediaOptions) => Promise<boolean>;
  /** Fetches a download again from its link, as a new download: for one
   *  whose file has been deleted. */
  again: (d: Download) => void;
  /** Opens the dialog making a schedule, with these downloads picked. */
  schedule: (ids: number[]) => void;
  /** Opens the dialog setting one download's own speed limit. */
  limit: (d: Download) => void;
  /** Opens the dialog giving a download a new link (one that expired). */
  refreshLink: (d: Download) => void;
  toast: (message: string, tone?: "error" | "info") => void;
}

/** How progress is drawn: one bar, or one bar for each connection. */
export type ProgressStyle = "single" | "threads";

export interface AppState {
  /** Every download shown as a row: a batch's own downloads are not among them. */
  downloads: Download[];
  /** The downloads inside each batch, by the batch's id. */
  batchItems: Map<number, Download[]>;
  progressStyle: ProgressStyle;
  settings: Settings | null;
  /** Recent speed samples per download id, oldest first. */
  history: Map<number, number[]>;
  actions: Actions;
}

export const AppContext = createContext<AppState | null>(null);

export function useApp(): AppState {
  const ctx = useContext(AppContext);
  if (!ctx) throw new Error("AppContext is missing");
  return ctx;
}

/* ---------- logo ---------- */

/** The RushDownload bolt, in the two blues of the brand logo. */
export function Logo({ size = 24 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" aria-hidden="true" className="logo">
      <defs>
        <linearGradient id="rd-bolt" x1="5" y1="3" x2="19" y2="21" gradientUnits="userSpaceOnUse">
          <stop offset="0" stopColor="#2F7DFC" />
          <stop offset="1" stopColor="#0B60FB" />
        </linearGradient>
      </defs>
      <path
        fill="url(#rd-bolt)"
        d="M4 14a1 1 0 0 1-.78-1.63l9.9-10.2a.5.5 0 0 1 .86.46l-1.92 6.02A1 1 0 0 0 13 10h7a1 1 0 0 1 .78 1.63l-9.9 10.2a.5.5 0 0 1-.86-.46l1.92-6.02A1 1 0 0 0 11 14z"
      />
    </svg>
  );
}

/* ---------- small controls ---------- */

export function IconBtn(props: {
  icon: LucideIcon;
  label: string;
  onClick: (e: ReactMouseEvent<HTMLButtonElement>) => void;
  active?: boolean;
  outlined?: boolean;
}) {
  const Icon = props.icon;
  const cls = ["icon-btn", props.outlined && "is-outlined", props.active && "is-active"]
    .filter(Boolean)
    .join(" ");
  return (
    <button
      type="button"
      className={cls}
      title={props.label}
      aria-label={props.label}
      aria-pressed={props.active}
      onClick={props.onClick}
    >
      <Icon size={17} strokeWidth={2} aria-hidden="true" />
    </button>
  );
}

const CATEGORY_ICON: Record<Category, LucideIcon> = {
  video: Film,
  audio: Music,
  image: ImageGlyph,
  document: FileText,
  archive: Archive,
  software: Package,
  other: FileGlyph,
};

export function categoryIcon(c: Category): LucideIcon {
  return CATEGORY_ICON[c];
}

/** Profile pictures already asked for, by the profile's link. One question
 *  per link while the app is open; an answer is also kept for next time. */
const faces = new Map<string, Promise<string | null>>();
const FACE_KEPT = "rushdownload.face.";

function kept(link: string): string | null {
  try {
    return window.localStorage.getItem(FACE_KEPT + link);
  } catch {
    return null;
  }
}

function forget(link: string) {
  faces.delete(link);
  try {
    window.localStorage.removeItem(FACE_KEPT + link);
  } catch {
    // nothing was kept
  }
}

/** The picture of the profile a download is saving, once it is known. */
function useProfilePicture(d: Download): [string | null, () => void] {
  // A playlist is saved the same way but is nobody's profile.
  const link = d.profile && !/[?&]list=/.test(d.url) ? d.url : null;
  const [picture, setPicture] = useState<string | null>(() => (link ? kept(link) : null));
  useEffect(() => {
    if (!link) {
      setPicture(null);
      return;
    }
    const had = kept(link);
    setPicture(had);
    if (had) return;
    let stale = false;
    let asked = faces.get(link);
    if (!asked) {
      asked = api
        .profileFace(link)
        .then((face) => face.picture)
        .catch(() => null);
      faces.set(link, asked);
    }
    asked.then((found) => {
      if (!found) return;
      try {
        window.localStorage.setItem(FACE_KEPT + link, found);
      } catch {
        // kept for this run only
      }
      if (!stale) setPicture(found);
    });
    return () => {
      stale = true;
    };
  }, [link]);
  // A picture the site no longer serves (its address has changed) is dropped,
  // and asked for afresh the next time.
  const failed = () => {
    if (link) forget(link);
    setPicture(null);
  };
  return [picture, failed];
}

/** A page marked with the file's type and extension (see DocIcon). A profile
 *  being saved shows whose it is instead: the profile's own picture. */
export function FileIcon({ d, large }: { d: Download; large?: boolean }) {
  const [picture, pictureFailed] = useProfilePicture(d);
  if (d.batch) return <BatchTile large={large} />;
  // A video from a site (or a playlist of them) shows the site's picture of
  // it, in the tile's own shape. One that will not load gives way to the tile.
  const [thumbFailed, setThumbFailed] = useState<string | null>(null);
  if (!picture && d.thumbnail && thumbFailed !== d.thumbnail) {
    return (
      <img
        className={`file-icon file-thumb${large ? " file-icon-lg" : ""}`}
        src={d.thumbnail}
        alt=""
        aria-hidden="true"
        referrerPolicy="no-referrer"
        onError={() => setThumbFailed(d.thumbnail)}
      />
    );
  }
  if (picture) {
    return (
      <img
        className={`file-icon file-face${large ? " file-icon-lg" : ""}`}
        src={picture}
        alt=""
        aria-hidden="true"
        referrerPolicy="no-referrer"
        onError={pictureFailed}
      />
    );
  }
  return <DocIcon d={d} large={large} />;
}

/** Dot plus status word, used where a download is listed outside the main list. */
export function StatusText({ d }: { d: Download }) {
  // A finished file that does not match its checksum is not shown as fine.
  if (d.status === "completed" && d.checksum?.state === "failed") {
    return (
      <span className="status status-failed">
        <span className="status-dot" aria-hidden="true" />
        {t.checksum.rowFailed}
      </span>
    );
  }
  if (d.status === "completed" && d.checksum?.state === "calculating") {
    return (
      <span className="status status-connecting">
        <span className="status-dot" aria-hidden="true" />
        {t.checksum.rowCalculating}
      </span>
    );
  }
  return (
    <span className={`status status-${d.status}`}>
      <span className="status-dot" aria-hidden="true" />
      {checkPercent(d) != null ? t.ui.checking : STATUS_LABEL[d.status]}
    </span>
  );
}

const CONNECT_SEGMENTS = 12;

/**
 * Shown above the progress bar only while a download is still connecting:
 * a framed strip of slanted segments, pale to deep blue, lighting up in a
 * wave. It goes away as soon as the download is connected.
 */
function ConnectingBar({ tall, failures }: { tall?: boolean; failures: number }) {
  // Blue while all is well, orange after a failed attempt, red if it keeps failing.
  const tone = failures >= 2 ? " is-failing" : failures === 1 ? " is-trouble" : "";
  const label =
    failures >= 2 ? t.ui.connect.stillFailing : failures === 1 ? t.ui.connect.failed : t.ui.connect.connecting;
  return (
    <div className={`connect-bar${tall ? " connect-bar-tall" : ""}${tone}`} role="status" aria-label={label} title={label}>
      {Array.from({ length: CONNECT_SEGMENTS }, (_, i) => (
        <span
          key={i}
          style={
            {
              "--shade": `${Math.round((i / (CONNECT_SEGMENTS - 1)) * 100)}%`,
              animationDelay: `${i * 110}ms`,
            } as React.CSSProperties
          }
        />
      ))}
    </div>
  );
}

/**
 * Shown above the progress bar in place of the connecting strip while what
 * is already on disk is being checked (a resumed torrent): how far the
 * check has got. The bar under it keeps what was downloaded before.
 */
function CheckingBar({ tall, percent }: { tall?: boolean; percent: number }) {
  return (
    <div
      className={`check-bar${tall ? " check-bar-tall" : ""}`}
      role="progressbar"
      aria-label={t.ui.checkBar}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={percent}
      title={t.common.checking(percent)}
    >
      <span style={{ width: `${percent}%` }} />
    </div>
  );
}

/** One continuous bar of overall progress. */
export function Progress({ d, tall }: { d: Download; tall?: boolean }) {
  const cls = `bar bar-${d.status}${tall ? " bar-tall" : ""}`;
  const pct = percentOf(d);
  const checked = checkPercent(d);
  const connecting =
    d.status === "connecting" &&
    (checked != null ? (
      <CheckingBar tall={tall} percent={checked} />
    ) : (
      <ConnectingBar tall={tall} failures={d.connect_failures ?? 0} />
    ));
  const { progressStyle } = useApp();
  // One bar for each connection, each as wide as its share of the file.
  // Torrents report equal stretches of the file with the pieces done in each.
  // Site videos report nothing per connection, so their overall progress is
  // shown across as many equal bars as the download has connections.
  let parts: [number, number, number][] = [];
  if (progressStyle === "threads" && d.total) {
    if (d.segments.length > 1) {
      parts = d.segments;
    } else if (d.segments.length === 0) {
      const count = Math.min(16, Math.max(2, d.connections));
      const size = d.total / count;
      parts = Array.from({ length: count }, (_, i) => {
        const start = i * size;
        return [start, Math.min(start + size, Math.max(start, d.downloaded)), start + size];
      });
    }
  }
  if (parts.length > 1) {
    return (
      <>
        {connecting}
        <div
          className={`${cls} bar-threads`}
          role="progressbar"
          aria-label={t.ui.progress.parts(d.filename, parts.length)}
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={pct ?? 0}
        >
          {parts.map(([start, pos, end], i) => {
            const size = Math.max(1, end - start);
            return (
              <span key={i} className="thread" style={{ flexGrow: size }}>
                <span
                  className={`fill${pos >= end ? " is-done" : ""}`}
                  style={{ width: `${Math.min(100, ((pos - start) / size) * 100)}%` }}
                />
              </span>
            );
          })}
        </div>
      </>
    );
  }
  if (pct == null) {
    const busy = d.status === "downloading";
    return (
      <>
        {connecting}
        <div className={`${cls}${busy ? " bar-busy" : ""}`} />
      </>
    );
  }
  return (
    <>
      {connecting}
      <div
        className={cls}
        role="progressbar"
        aria-label={t.ui.progress.label(d.filename)}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={pct}
      >
        <div className="fill" style={{ width: `${d.total ? (d.downloaded / d.total) * 100 : pct}%` }} />
      </div>
    </>
  );
}

export function fileCount(n: number): string {
  return t.ui.files(n);
}

/** "72.5 MB / 1.2 GB" for anything unfinished. */
export function amountOf(d: Download): string {
  if (d.profile) return t.ui.profileAmount(fileCount(d.items), formatBytes(d.downloaded));
  return d.total != null
    ? `${formatBytes(d.downloaded)} / ${formatBytes(d.total)}`
    : formatBytes(d.downloaded);
}

/** Line under the file name: status word in colour, then the facts. */
export function RowMeta({ d }: { d: Download }) {
  if (d.batch) return <BatchMeta d={d} />;
  switch (d.status) {
    case "downloading":
      return <>{amountOf(d)} · {formatSpeed(d.speed)}</>;
    case "connecting":
      return <>{checkingText(d) ?? t.common.status.connecting}</>;
    case "queued":
      return <>{d.total != null ? t.ui.meta.queuedSize(formatBytes(d.total)) : t.ui.meta.queuedWaiting}</>;
    case "paused":
      return <>{t.ui.meta.paused(amountOf(d))}</>;
    case "completed":
      // The file is gone from where it was saved: that is what there is to say.
      if (isDeleted(d)) {
        return (
          <>
            <span className="tone-deleted">{t.ui.meta.deleted}</span> · {deletedAgo(d)}
            {d.total != null ? ` · ${t.ui.meta.was(formatBytes(d.total))}` : ""}
          </>
        );
      }
      // A file that does not match its checksum is never shown as fine.
      if (d.checksum?.state === "failed") {
        return (
          <>
            <span className="tone-err">{t.checksum.rowFailed}</span>
            {d.total != null ? ` · ${formatBytes(d.total)}` : ""}
          </>
        );
      }
      return (
        <>
          <span className="tone-ok">{t.common.status.completed}</span>
          {d.checksum?.state === "calculating" ? ` · ${t.checksum.rowCalculating}` : ""}
          {d.checksum?.state === "verified" ? ` · ${t.checksum.algorithms[d.checksum.algorithm]} ${t.checksum.states.verified.toLowerCase()}` : ""}
          {d.profile ? ` · ${fileCount(d.items)}` : ""}
          {d.total != null ? ` · ${formatBytes(d.total)}` : ""}
        </>
      );
    case "failed":
      return (
        <>
          <span className="tone-err">{t.common.status.failed}</span> · {d.error ?? t.ui.meta.unknownError}
        </>
      );
  }
}

/* ---------- speed graph ---------- */

const MB_PER_SECOND = 1024 * 1024;

/** Colour band for a speed: the faster the download, the cooler the colour. */
export function speedBand(bytesPerSecond: number): "fast" | "good" | "slow" | "crawl" | "stopped" {
  const mb = bytesPerSecond / MB_PER_SECOND;
  if (mb <= 0) return "stopped";
  if (mb < 1.5) return "crawl";
  if (mb < 10) return "slow";
  if (mb < 15) return "good";
  return "fast";
}

/**
 * Shade of the logo blue for a speed, measured against what the graph is
 * showing: the slowest moment on screen is the palest, the fastest is the
 * deepest. A fixed scale in MB/s would leave a steady download looking like one
 * flat colour; this way any rise or dip is visible. The actual speed is in the
 * label and the hover tooltip.
 */
export function speedShade(bytesPerSecond: number, slowest: number, fastest: number): string {
  const range = fastest - slowest;
  // nothing to compare when the speed never changed: use the middle of the scale
  const position = range > 0 ? Math.min(1, Math.max(0, (bytesPerSecond - slowest) / range)) : 0.6;
  // keep the palest end visible against the background
  const share = Math.round(12 + position * 88);
  return `color-mix(in srgb, var(--speed-max) ${share}%, var(--speed-min))`;
}

/** Smooth curve through points (Catmull-Rom turned into Bezier pieces). */
function wave(pts: (readonly [number, number])[], top: number, bottom: number): string {
  const clamp = (y: number) => Math.min(bottom, Math.max(top, y));
  let d = `M${pts[0][0].toFixed(1)},${pts[0][1].toFixed(1)}`;
  for (let i = 0; i < pts.length - 1; i++) {
    const p0 = pts[Math.max(0, i - 1)];
    const p1 = pts[i];
    const p2 = pts[i + 1];
    const p3 = pts[Math.min(pts.length - 1, i + 2)];
    const c1 = [p1[0] + (p2[0] - p0[0]) / 6, clamp(p1[1] + (p2[1] - p0[1]) / 6)];
    const c2 = [p2[0] - (p3[0] - p1[0]) / 6, clamp(p2[1] - (p3[1] - p1[1]) / 6)];
    d += ` C${c1[0].toFixed(1)},${c1[1].toFixed(1)} ${c2[0].toFixed(1)},${c2[1].toFixed(1)} ${p2[0].toFixed(1)},${p2[1].toFixed(1)}`;
  }
  return d;
}

/**
 * Speed over the last minute as one bold wave. The readings are grouped into a
 * handful of points, with a dense row of thin bars beneath that follows the
 * same curve. Every stretch of the wave takes the colour of the
 * speed it shows, and pointing at the graph tells the speed at that moment.
 */
export function Sparkline(props: { values: number[]; label: string; now?: string; speed?: number }) {
  const { values, label, now } = props;
  const gradient = useId().replace(/[^a-zA-Z0-9]/g, "");
  const [hover, setHover] = useState<number | null>(null);
  const w = 300;
  const h = 96;
  if (values.length < 3) {
    return (
      <div className="spark spark-empty" role="img" aria-label={label}>
        {t.ui.spark.empty}
      </div>
    );
  }

  // Group the readings into at most nine points.
  const count = Math.min(9, values.length);
  const perPoint = values.length / count;
  const points: number[] = [];
  for (let i = 0; i < count; i++) {
    const from = Math.floor(i * perPoint);
    const to = Math.max(from + 1, Math.floor((i + 1) * perPoint));
    const slice = values.slice(from, to);
    points.push(slice.reduce((sum, v) => sum + v, 0) / slice.length);
  }

  // Stretch the wave between the lowest and highest point so its shape shows.
  const high = Math.max(...points, 1);
  const low = Math.min(...points);
  const pad = Math.max((high - low) * 0.35, high * 0.06);
  const floor = Math.max(0, low - pad);
  const ceiling = high + pad;
  const top = 6;
  const bottom = h - 6;
  const x = (i: number) => (i / (count - 1)) * w;
  const y = (v: number) => bottom - ((v - floor) / (ceiling - floor)) * (bottom - top);
  const main = points.map((v, i) => [x(i), y(v)] as const);

  const line = wave(main, top, bottom);
  const area = `${line} L${w},${h} L0,${h} Z`;
  const band = speedBand(props.speed ?? values[values.length - 1]);

  // A dense row of thin bars whose tops trace the wave itself, a little below
  // the line. Each bar reads the height of the very curve drawn above it.
  const barCount = 96;
  const span = w / (count - 1);
  const edge = (yy: number) => Math.min(bottom, Math.max(top, yy));
  const waveY = (px: number) => {
    const seg = Math.min(count - 2, Math.max(0, Math.floor(px / span)));
    const k = Math.min(1, Math.max(0, (px - seg * span) / span));
    const y0 = main[Math.max(0, seg - 1)][1];
    const y1 = main[seg][1];
    const y2 = main[seg + 1][1];
    const y3 = main[Math.min(count - 1, seg + 2)][1];
    const c1 = edge(y1 + (y2 - y0) / 6);
    const c2 = edge(y2 - (y3 - y1) / 6);
    const u = 1 - k;
    return u * u * u * y1 + 3 * u * u * k * c1 + 3 * u * k * k * c2 + k * k * k * y2;
  };
  const bars = Array.from({ length: barCount }, (_, i) => {
    const px = ((i + 0.5) / barCount) * w;
    // percent of the plot's height, stopping a little short of the line
    return Math.max(3, ((h - waveY(px)) / h) * 100 - 9);
  });
  const shown = hover != null && hover < count ? hover : null;
  // Readings arrive about twice a second.
  const secondsAgo = shown == null ? 0 : Math.round((count - 1 - shown) * perPoint * 0.5);

  return (
    <div className={`spark spark-${band}`}>
      <div
        className="spark-plot"
        onMouseMove={(e) => {
          const box = e.currentTarget.getBoundingClientRect();
          const ratio = Math.min(1, Math.max(0, (e.clientX - box.left) / box.width));
          setHover(Math.round(ratio * (count - 1)));
        }}
        onMouseLeave={() => setHover(null)}
      >
        {/* plain elements rather than SVG, so the rounded tops keep their shape
            however wide the graph is stretched */}
        <div className="spark-bars" aria-hidden="true">
          {bars.map((b, i) => (
            <span key={i} style={{ height: `${b}%` }} />
          ))}
        </div>
        <svg viewBox={`0 0 ${w} ${h}`} preserveAspectRatio="none" role="img" aria-label={label}>
          <defs>
            {/* left to right: each point's own speed colour, blended between points */}
            <linearGradient id={`${gradient}hue`} gradientUnits="userSpaceOnUse" x1="0" y1="0" x2={w} y2="0">
              {points.map((v, i) => (
                <stop key={i} offset={i / (count - 1)} style={{ stopColor: speedShade(v, low, high) }} />
              ))}
            </linearGradient>
            <linearGradient id={`${gradient}fade`} gradientUnits="userSpaceOnUse" x1="0" y1="0" x2="0" y2={h}>
              <stop offset="0" stopColor="#fff" stopOpacity="0.34" />
              <stop offset="1" stopColor="#fff" stopOpacity="0" />
            </linearGradient>
            <mask id={`${gradient}mask`} maskUnits="userSpaceOnUse" x="0" y="0" width={w} height={h}>
              <rect width={w} height={h} fill={`url(#${gradient}fade)`} />
            </mask>
          </defs>
          <path d={area} fill={`url(#${gradient}hue)`} mask={`url(#${gradient}mask)`} />
          <path d={line} className="spark-line" stroke={`url(#${gradient}hue)`} />
        </svg>
        {shown != null && (
          <span className="spark-guide" style={{ left: `${(main[shown][0] / w) * 100}%` }} aria-hidden="true" />
        )}
        {/* no rings along the wave; only the pointed-at moment gets a marker */}
        {main.map(([px, py], i) => i !== shown ? null : (
          <span
            key={i}
            className={`spark-mark${i === count - 1 ? " is-now" : ""}${i === shown ? " is-hover" : ""}`}
            style={
              {
                left: `${(px / w) * 100}%`,
                top: `${(py / h) * 100}%`,
                "--wave": speedShade(points[i], low, high),
              } as React.CSSProperties
            }
            aria-hidden="true"
          />
        ))}
        {shown != null && (
          <span
            className={`spark-tip${shown === 0 ? " is-left" : shown === count - 1 ? " is-right" : ""}`}
            style={
              {
                left: `${(main[shown][0] / w) * 100}%`,
                top: `${(main[shown][1] / h) * 100}%`,
                "--wave": speedShade(points[shown], low, high),
              } as React.CSSProperties
            }
            role="status"
          >
            <strong>{formatSpeed(points[shown])}</strong>
            <small>{secondsAgo === 0 ? t.ui.spark.now : t.ui.spark.secondsAgo(secondsAgo)}</small>
          </span>
        )}
      </div>
      {now && <span className="spark-now">{now}</span>}
    </div>
  );
}

/* ---------- menus ---------- */

export type MenuItem =
  | { label: string; icon?: LucideIcon; onSelect: () => void; danger?: boolean; checked?: boolean }
  | "separator";

export function Menu(props: { x: number; y: number; items: MenuItem[]; onClose: () => void }) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState({ left: props.x, top: props.y });

  // Keep the menu inside the window.
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    setPos({
      left: Math.max(8, Math.min(props.x, window.innerWidth - r.width - 8)),
      top: Math.max(8, Math.min(props.y, window.innerHeight - r.height - 8)),
    });
    el.querySelector<HTMLButtonElement>("button")?.focus();
  }, [props.x, props.y]);

  useEffect(() => {
    const close = () => props.onClose();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        props.onClose();
        return;
      }
      if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
      e.preventDefault();
      const buttons = Array.from(ref.current?.querySelectorAll<HTMLButtonElement>("button") ?? []);
      const i = buttons.indexOf(document.activeElement as HTMLButtonElement);
      const next = e.key === "ArrowDown" ? i + 1 : i - 1;
      buttons[(next + buttons.length) % buttons.length]?.focus();
    };
    window.addEventListener("resize", close);
    window.addEventListener("blur", close);
    window.addEventListener("keydown", onKey, true);
    return () => {
      window.removeEventListener("resize", close);
      window.removeEventListener("blur", close);
      window.removeEventListener("keydown", onKey, true);
    };
  }, [props.onClose]);

  return (
    <div className="menu-layer" onMouseDown={props.onClose} onContextMenu={(e) => e.preventDefault()}>
      <div ref={ref} className="menu" role="menu" style={pos} onMouseDown={(e) => e.stopPropagation()}>
        {props.items.map((item, i) =>
          item === "separator" ? (
            <div key={i} className="menu-sep" role="separator" />
          ) : (
            <button
              key={i}
              type="button"
              role={item.checked == null ? "menuitem" : "menuitemradio"}
              aria-checked={item.checked}
              className={[item.danger && "is-danger", item.checked && "is-checked"].filter(Boolean).join(" ") || undefined}
              onClick={() => {
                props.onClose();
                item.onSelect();
              }}
            >
              {item.icon ? <item.icon size={16} strokeWidth={2} aria-hidden="true" /> : <span />}
              <span>{item.label}</span>
              {item.checked && <Check size={15} strokeWidth={2.25} aria-hidden="true" />}
            </button>
          ),
        )}
      </div>
    </div>
  );
}

/** One choice of a `Select`. */
export interface SelectOption<T extends string> {
  value: T;
  label: string;
  /** The language the label is written in, where it is not the app's. */
  lang?: string;
}

/**
 * The app's own drop-down list, in place of the system's `<select>`, whose
 * open list cannot be styled: a field that shows the choice, and a list in
 * the Float material under it (above it when there is no room below). It
 * works like a select from the keyboard: arrows, Home, End, a letter to jump,
 * Enter or Space to pick, Escape or Tab to close.
 */
export function Select<T extends string>(props: {
  value: T;
  options: SelectOption<T>[];
  onChange: (value: T) => void;
  "aria-label"?: string;
  "aria-labelledby"?: string;
  disabled?: boolean;
  className?: string;
}) {
  const listId = useId();
  const trigger = useRef<HTMLButtonElement>(null);
  const list = useRef<HTMLDivElement>(null);
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(0);
  const [pos, setPos] = useState<{ top: number; left: number; minWidth: number; maxHeight: number } | null>(null);
  const typed = useRef({ text: "", at: 0 });
  const chosenIndex = Math.max(
    0,
    props.options.findIndex((o) => o.value === props.value),
  );
  const chosen = props.options[chosenIndex];

  function show() {
    if (props.disabled) return;
    setActive(chosenIndex);
    setOpen(true);
  }

  function close(refocus = true) {
    setOpen(false);
    setPos(null);
    if (refocus) trigger.current?.focus();
  }

  function pick(i: number) {
    const option = props.options[i];
    close();
    if (option && option.value !== props.value) props.onChange(option.value);
  }

  // Place the list under the field, inside the window, the field's width at least.
  useLayoutEffect(() => {
    if (!open) return;
    const field = trigger.current?.getBoundingClientRect();
    const el = list.current;
    if (!field || !el) return;
    const gap = 6;
    const margin = 8;
    const below = window.innerHeight - field.bottom - gap - margin;
    const above = field.top - gap - margin;
    const wanted = Math.min(el.scrollHeight, 320);
    const down = below >= wanted || below >= above;
    const maxHeight = Math.max(120, Math.min(320, down ? below : above));
    const height = Math.min(wanted, maxHeight);
    const width = Math.max(field.width, el.offsetWidth);
    const rtl = document.documentElement.dir === "rtl";
    const start = rtl ? field.right - width : field.left;
    setPos({
      top: down ? field.bottom + gap : field.top - gap - height,
      left: Math.max(margin, Math.min(start, window.innerWidth - width - margin)),
      minWidth: field.width,
      maxHeight,
    });
  }, [open]);

  // Bring the line being pointed at into view and give it the focus.
  useEffect(() => {
    if (!open || !pos) return;
    const item = list.current?.querySelectorAll<HTMLElement>("[role=option]")[active];
    item?.focus({ preventScroll: true });
    item?.scrollIntoView({ block: "nearest" });
  }, [open, pos, active]);

  // A change of window size, or the page scrolling under it, closes the list.
  useEffect(() => {
    if (!open) return;
    const shut = () => close(false);
    const onScroll = (e: Event) => {
      if (list.current && e.target instanceof Node && list.current.contains(e.target)) return;
      shut();
    };
    window.addEventListener("resize", shut);
    window.addEventListener("blur", shut);
    window.addEventListener("scroll", onScroll, true);
    return () => {
      window.removeEventListener("resize", shut);
      window.removeEventListener("blur", shut);
      window.removeEventListener("scroll", onScroll, true);
    };
  }, [open]);

  function onTriggerKey(e: React.KeyboardEvent) {
    if (["ArrowDown", "ArrowUp", "Enter", " "].includes(e.key)) {
      e.preventDefault();
      show();
    }
  }

  function onListKey(e: React.KeyboardEvent) {
    // Keys stay here: Escape closes the list, not the dialog it is in.
    e.stopPropagation();
    const last = props.options.length - 1;
    switch (e.key) {
      case "ArrowDown":
        e.preventDefault();
        setActive((i) => Math.min(last, i + 1));
        return;
      case "ArrowUp":
        e.preventDefault();
        setActive((i) => Math.max(0, i - 1));
        return;
      case "Home":
        e.preventDefault();
        setActive(0);
        return;
      case "End":
        e.preventDefault();
        setActive(last);
        return;
      case "Enter":
      case " ":
        e.preventDefault();
        pick(active);
        return;
      case "Escape":
      case "Tab":
        e.preventDefault();
        close();
        return;
    }
    // A letter jumps to the next choice starting with what was typed.
    if (e.key.length === 1 && !e.ctrlKey && !e.metaKey && !e.altKey) {
      const now = Date.now();
      const text = (now - typed.current.at < 700 ? typed.current.text : "") + e.key.toLowerCase();
      typed.current = { text, at: now };
      const from = text.length === 1 ? active + 1 : active;
      for (let k = 0; k <= last; k++) {
        const i = (from + k) % (last + 1);
        if (props.options[i].label.toLowerCase().startsWith(text)) {
          setActive(i);
          break;
        }
      }
    }
  }

  return (
    <>
      <button
        ref={trigger}
        type="button"
        className={`select-btn${props.className ? ` ${props.className}` : ""}`}
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-controls={open ? listId : undefined}
        aria-label={props["aria-label"]}
        aria-labelledby={props["aria-labelledby"]}
        disabled={props.disabled}
        onClick={() => (open ? close() : show())}
        onKeyDown={onTriggerKey}
      >
        <span className="select-value" lang={chosen?.lang}>
          {chosen?.label}
        </span>
        <ChevronDown className="select-chevron" size={15} strokeWidth={2} aria-hidden="true" />
      </button>
      {open &&
        createPortal(
          <div
            className="menu-layer"
            onMouseDown={(e) => {
              e.stopPropagation();
              close(false);
            }}
          >
            <div
              ref={list}
              id={listId}
              className="menu select-list"
              role="listbox"
              aria-label={props["aria-label"]}
              aria-labelledby={props["aria-labelledby"]}
              style={
                pos
                  ? { top: pos.top, left: pos.left, minWidth: pos.minWidth, maxHeight: pos.maxHeight }
                  : { visibility: "hidden", top: 0, left: 0 }
              }
              onMouseDown={(e) => e.stopPropagation()}
              onKeyDown={onListKey}
            >
              {props.options.map((option, i) => {
                const selected = option.value === props.value;
                return (
                  <div
                    key={option.value}
                    role="option"
                    tabIndex={-1}
                    aria-selected={selected}
                    lang={option.lang}
                    className={`select-option${selected ? " is-checked" : ""}${i === active ? " is-active" : ""}`}
                    onMouseEnter={() => setActive(i)}
                    onClick={() => pick(i)}
                  >
                    <span>{option.label}</span>
                    {selected && <Check size={15} strokeWidth={2.25} aria-hidden="true" />}
                  </div>
                );
              })}
            </div>
          </div>,
          document.body,
        )}
    </>
  );
}

/* ---------- a download's own speed limit ---------- */

const KB = 1024;
const MB = 1024 * 1024;

/** A speed limit in words: "2 MB/s", or "No limit" for 0. */
export function limitText(rate: number): string {
  return rate > 0 ? formatSpeed(rate) : t.ui.limit.none;
}

/**
 * A download's own speed limit: off, or an amount in KB/s or MB/s. The value
 * is in bytes per second; 0 means no limit of its own (the global limit in
 * Settings applies either way).
 */
export function SpeedLimitField(props: { value: number; onChange: (rate: number) => void; id?: string }) {
  const [on, setOn] = useState(props.value > 0);
  const [unit, setUnit] = useState<"kb" | "mb">(props.value > 0 && props.value < MB ? "kb" : "mb");
  const [amount, setAmount] = useState(() =>
    props.value > 0 ? String(+(props.value / (props.value < MB ? KB : MB)).toFixed(2)) : "2",
  );
  const base = props.id ?? "speed-limit";

  const emit = (nextOn: boolean, nextAmount: string, nextUnit: "kb" | "mb") => {
    const n = Number(nextAmount);
    props.onChange(nextOn && Number.isFinite(n) && n > 0 ? Math.round(n * (nextUnit === "kb" ? KB : MB)) : 0);
  };

  return (
    <div className="field speed-limit">
      <label className="check-line">
        <input
          type="checkbox"
          checked={on}
          onChange={(e) => {
            setOn(e.target.checked);
            emit(e.target.checked, amount, unit);
          }}
        />
        <span>{t.ui.limit.enable}</span>
      </label>
      <div className="field-row">
        <input
          type="number"
          min={1}
          step="any"
          inputMode="decimal"
          aria-label={t.ui.limit.amount}
          id={`${base}-amount`}
          disabled={!on}
          value={amount}
          onChange={(e) => {
            setAmount(e.target.value);
            emit(on, e.target.value, unit);
          }}
        />
        <Select<"kb" | "mb">
          aria-label={t.ui.limit.unit}
          disabled={!on}
          value={unit}
          onChange={(v) => {
            setUnit(v);
            emit(on, amount, v);
          }}
          options={[
            { value: "kb", label: t.common.perSecond(t.common.units[1]) },
            { value: "mb", label: t.common.perSecond(t.common.units[2]) },
          ]}
        />
      </div>
      <small>{t.ui.limit.hint}</small>
    </div>
  );
}

/** Sets one download's own speed limit, from its menu or its details. */
export function SpeedLimitDialog({ d, onClose }: { d: Download; onClose: () => void }) {
  const { actions } = useApp();
  const [rate, setRate] = useState(d.speed_limit);
  const running = d.status === "downloading" || d.status === "connecting";
  async function save() {
    try {
      await api.setSpeedLimit(d.id, rate);
      actions.toast(rate > 0 ? t.ui.limit.set(limitText(rate)) : t.ui.limit.cleared, "info");
      onClose();
    } catch (e) {
      actions.toast(String(e), "error");
    }
  }
  return (
    <Modal
      title={t.ui.limit.title}
      subtitle={displayName(d)}
      onClose={onClose}
      footer={
        <>
          <span className="spacer" />
          <button type="button" className="btn" onClick={onClose}>
            {t.ui.close}
          </button>
          <button type="button" className="btn btn-primary" onClick={save}>
            {t.ui.limit.save}
          </button>
        </>
      }
    >
      <div className="form">
        <SpeedLimitField value={d.speed_limit} onChange={setRate} id="limit-dialog" />
        {running && d.torrent && <small className="form-note">{t.ui.limit.torrentNote}</small>}
        {running && (d.media || d.profile) && <small className="form-note">{t.ui.limit.programNote}</small>}
      </div>
    </Modal>
  );
}

/* ---------- a new link for a download ---------- */

/** "Name: value" lines as headers; the line that is not one, if any. */
function parseHeaders(text: string): { headers: [string, string][]; bad: string | null } {
  const headers: [string, string][] = [];
  for (const raw of text.split(/\r?\n/)) {
    const line = raw.trim();
    if (!line) continue;
    const at = line.indexOf(":");
    if (at <= 0 || !/^[A-Za-z0-9-]+$/.test(line.slice(0, at).trim())) return { headers, bad: line };
    headers.push([line.slice(0, at).trim(), line.slice(at + 1).trim()]);
  }
  return { headers, bad: null };
}

/**
 * Gives a download whose link stopped working a new one. The new link is
 * checked first (the engine compares pieces of it with what is on disk); the
 * download carries on only if it is the same file, and otherwise can only
 * start over, which is said plainly with what would be lost.
 */
export function RefreshLinkDialog({ d, onClose }: { d: Download; onClose: () => void }) {
  const { actions } = useApp();
  const r = t.ui.refresh;
  const [url, setUrl] = useState("");
  const [headersOpen, setHeadersOpen] = useState(false);
  const [headerText, setHeaderText] = useState("");
  const [check, setCheck] = useState<LinkCheck | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  const headersFor = (): [string, string][] | undefined | null => {
    if (!headerText.trim()) return undefined;
    const { headers, bad } = parseHeaders(headerText);
    if (bad) {
      setError(r.badHeader(bad));
      return null;
    }
    return headers;
  };

  async function runCheck(e?: FormEvent) {
    e?.preventDefault();
    const headers = headersFor();
    if (headers === null || !url.trim() || busy) return;
    setBusy(true);
    setError("");
    setCheck(null);
    try {
      setCheck(await api.checkNewLink(d.id, url.trim(), headers));
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  }

  async function use(restart: boolean) {
    const headers = headersFor();
    if (headers === null || busy) return;
    setBusy(true);
    try {
      await api.useNewLink(d.id, url.trim(), headers, restart);
      actions.toast(r.done, "info");
      onClose();
    } catch (err) {
      setError(String(err));
      setBusy(false);
    }
  }

  const reset = () => {
    setCheck(null);
    setError("");
  };

  return (
    <Modal
      title={r.title}
      subtitle={displayName(d)}
      onClose={onClose}
      footer={
        <>
          <span className="spacer" />
          <button type="button" className="btn" onClick={onClose}>
            {t.ui.close}
          </button>
          {!check ? (
            <button type="submit" form="refresh-form" className="btn btn-primary" disabled={busy || !url.trim()}>
              {busy ? r.checking : r.check}
            </button>
          ) : check.resume ? (
            <button type="button" className="btn btn-primary" disabled={busy} onClick={() => use(false)}>
              {r.use}
            </button>
          ) : (
            <button type="button" className="btn btn-danger-solid" disabled={busy} onClick={() => use(true)}>
              {r.restart}
            </button>
          )}
        </>
      }
    >
      <form id="refresh-form" className="form" onSubmit={runCheck}>
        <label className="field">
          <span>{r.link}</span>
          <input
            data-autofocus
            type="text"
            value={url}
            spellCheck={false}
            placeholder="https://"
            onChange={(e) => {
              setUrl(e.target.value);
              reset();
            }}
          />
          <small className="refresh-old" title={d.url}>
            {r.old(d.url)}
          </small>
        </label>
        <button type="button" className="disclosure" aria-expanded={headersOpen} onClick={() => setHeadersOpen((v) => !v)}>
          {headersOpen ? <ChevronDown size={15} aria-hidden="true" /> : <ChevronRight size={15} aria-hidden="true" />}
          {r.headers}
        </button>
        {headersOpen && (
          <label className="field">
            <textarea
              rows={3}
              spellCheck={false}
              aria-label={r.headers}
              placeholder="Authorization: Bearer …"
              value={headerText}
              onChange={(e) => {
                setHeaderText(e.target.value);
                reset();
              }}
            />
            <small>{r.headersHint}</small>
          </label>
        )}
        {error && (
          <small className="field-error" role="alert">
            {error}
          </small>
        )}
        {check && (
          <div className={`refresh-verdict ${check.resume ? "is-ok" : "is-warn"}`} role="status">
            {check.resume ? (
              <p>{check.downloaded > 0 ? r.same(formatBytes(check.downloaded)) : r.fresh}</p>
            ) : (
              <>
                <p>
                  <strong>{check.problem ? r.problems[check.problem] : r.problems.content_differs}</strong>
                </p>
                <p>{r.restartNote(formatBytes(check.downloaded))}</p>
              </>
            )}
          </div>
        )}
      </form>
    </Modal>
  );
}

/* ---------- a button with an arrow that fills it ---------- */

/**
 * A pill button with a round mark at its end holding an arrow. Under the
 * pointer (or the keyboard's focus) the mark grows until it fills the
 * button, the text takes the mark's colour, and a new arrow slides in as
 * the old one leaves. The look is in styles.css ("arrow fill").
 */
export function ArrowFillButton({
  children,
  className,
  ref,
  ...rest
}: React.ButtonHTMLAttributes<HTMLButtonElement> & { ref?: React.Ref<HTMLButtonElement> }) {
  return (
    <button {...rest} ref={ref} className={`arrow-fill${className ? ` ${className}` : ""}`}>
      <span className="arrow-fill-text">{children}</span>
      <span className="arrow-fill-fill" aria-hidden="true" />
      <span className="arrow-fill-clone" aria-hidden="true">
        <span>{children}</span>
      </span>
      <span className="arrow-fill-mark" aria-hidden="true">
        <ArrowRight className="arrow-fill-in" size={16} strokeWidth={2.2} />
        <ArrowRight className="arrow-fill-out" size={16} strokeWidth={2.2} />
      </span>
    </button>
  );
}

/* ---------- pickers that open under a field ---------- */

/**
 * Places a picker under its field, inside the window (above the field when
 * there is no room below, mirrored in right-to-left text), and closes it
 * when the window is resized, loses focus, or the page scrolls under it.
 * Returns the position to give the picker; null until it is measured.
 */
function usePopover(
  open: boolean,
  trigger: React.RefObject<HTMLElement | null>,
  pop: React.RefObject<HTMLElement | null>,
  close: () => void,
) {
  const [pos, setPos] = useState<{ top: number; left: number } | null>(null);

  useLayoutEffect(() => {
    if (!open) {
      setPos(null);
      return;
    }
    const field = trigger.current?.getBoundingClientRect();
    const el = pop.current;
    if (!field || !el) return;
    const gap = 6;
    const margin = 8;
    const height = el.offsetHeight;
    const width = el.offsetWidth;
    const down = window.innerHeight - field.bottom - gap - margin >= height || field.top < height + gap + margin;
    const rtl = document.documentElement.dir === "rtl";
    const start = rtl ? field.right - width : field.left;
    setPos({
      top: down ? field.bottom + gap : field.top - gap - height,
      left: Math.max(margin, Math.min(start, window.innerWidth - width - margin)),
    });
    // measured once, when it opens
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const onScroll = (e: Event) => {
      if (pop.current && e.target instanceof Node && pop.current.contains(e.target)) return;
      close();
    };
    window.addEventListener("resize", close);
    window.addEventListener("blur", close);
    window.addEventListener("scroll", onScroll, true);
    return () => {
      window.removeEventListener("resize", close);
      window.removeEventListener("blur", close);
      window.removeEventListener("scroll", onScroll, true);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  return pos;
}

/* ---------- time of day ---------- */

/** A name the browser gives a part of a date ("hour", "Stunde"), in the
 *  interface's language; `fallback` where it has none. */
function fieldName(field: "hour" | "minute" | "dayPeriod", fallback: string): string {
  try {
    return new Intl.DisplayNames(formatLocale(), { type: "dateTimeField" }).of(field) ?? fallback;
  } catch {
    return fallback;
  }
}

const pad2 = (n: number) => String(n).padStart(2, "0");

/**
 * A time of day, in minutes after midnight, picked from three columns
 * (hours, minutes, and AM/PM where the language uses a 12-hour clock) in
 * the Float material, in place of the system's grey time picker.
 * Arrow keys move through a column, Enter or Escape closes it.
 */
export function TimeField(props: {
  value: number;
  onChange: (minutes: number) => void;
  disabled?: boolean;
  "aria-label"?: string;
  "aria-labelledby"?: string;
}) {
  const trigger = useRef<HTMLButtonElement>(null);
  const pop = useRef<HTMLDivElement>(null);
  const [open, setOpen] = useState(false);
  const pos = usePopover(open, trigger, pop, () => close(false));
  const locale = formatLocale();

  const { h12, periods, names } = useMemo(() => {
    const cycle = new Intl.DateTimeFormat(locale, { hour: "numeric" }).resolvedOptions().hourCycle;
    const period = (h: number, fallback: string) =>
      new Intl.DateTimeFormat(locale, { hour: "numeric", hour12: true })
        .formatToParts(new Date(2024, 0, 1, h))
        .find((part) => part.type === "dayPeriod")?.value ?? fallback;
    return {
      h12: cycle === "h11" || cycle === "h12",
      periods: [period(9, "AM"), period(21, "PM")],
      names: [fieldName("hour", "Hour"), fieldName("minute", "Minute"), fieldName("dayPeriod", "AM/PM")],
    };
  }, [locale]);

  const hour = Math.floor(props.value / 60) % 24;
  const minute = props.value % 60;
  const pm = hour >= 12;
  const hours = h12 ? [12, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11] : Array.from({ length: 24 }, (_, i) => i);
  const shownHour = h12 ? hour % 12 || 12 : hour;
  const minutes = Array.from({ length: 60 }, (_, i) => i);

  const set = (h: number, m: number) => {
    const next = (((h % 24) + 24) % 24) * 60 + m;
    if (next !== props.value) props.onChange(next);
  };
  const pickHour = (shown: number) => set(h12 ? (shown % 12) + (pm ? 12 : 0) : shown, minute);
  const pickMinute = (m: number) => set(hour, m);
  const pickPeriod = (toPm: boolean) => set((hour % 12) + (toPm ? 12 : 0), minute);

  function close(refocus = true) {
    setOpen(false);
    if (refocus) trigger.current?.focus();
  }

  // Each column shows its choice in the middle; the hour takes the focus.
  useEffect(() => {
    if (!open || !pos) return;
    pop.current?.querySelectorAll<HTMLElement>(".time-col").forEach((col) => {
      const chosen = col.querySelector<HTMLElement>(".is-checked");
      if (chosen) col.scrollTop = chosen.offsetTop - col.clientHeight / 2 + chosen.offsetHeight / 2;
    });
    pop.current?.querySelector<HTMLElement>(".time-col .is-checked")?.focus({ preventScroll: true });
    // the value changes while open; only the first placing scrolls
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, pos != null]);

  /** Up and down move a column's choice and keep it in view and in focus. */
  function onColumnKey(e: React.KeyboardEvent, step: (by: number) => void) {
    if (e.key !== "ArrowUp" && e.key !== "ArrowDown") return;
    e.preventDefault();
    step(e.key === "ArrowDown" ? 1 : -1);
    const col = e.currentTarget;
    requestAnimationFrame(() => {
      const chosen = col.querySelector<HTMLElement>(".is-checked");
      chosen?.focus({ preventScroll: true });
      chosen?.scrollIntoView({ block: "nearest" });
    });
  }

  function onPopKey(e: React.KeyboardEvent) {
    // Keys stay here: Escape closes the picker, not the dialog it is in.
    e.stopPropagation();
    if (e.key === "Escape" || e.key === "Enter") {
      e.preventDefault();
      close();
    }
  }

  const column = (
    label: string,
    items: { key: string; text: string; on: boolean; pick: () => void }[],
    step: (by: number) => void,
  ) => (
    <div className="time-col" role="listbox" aria-label={label} onKeyDown={(e) => onColumnKey(e, step)}>
      {items.map((item) => (
        <button
          key={item.key}
          type="button"
          role="option"
          aria-selected={item.on}
          tabIndex={item.on ? 0 : -1}
          className={item.on ? "is-checked" : undefined}
          onClick={item.pick}
        >
          {item.text}
        </button>
      ))}
    </div>
  );

  const hourIndex = hours.indexOf(shownHour);
  return (
    <>
      <button
        ref={trigger}
        type="button"
        className="select-btn time-btn"
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-label={props["aria-label"]}
        aria-labelledby={props["aria-labelledby"]}
        disabled={props.disabled}
        onClick={() => (open ? close() : setOpen(true))}
        onKeyDown={(e) => {
          if (["ArrowDown", "ArrowUp", "Enter", " "].includes(e.key)) {
            e.preventDefault();
            setOpen(true);
          }
        }}
      >
        <span className="select-value">
          {dateText(new Date(2024, 0, 1, hour, minute), { hour: "numeric", minute: "2-digit" })}
        </span>
        <Clock className="select-chevron" size={15} strokeWidth={2} aria-hidden="true" />
      </button>
      {open &&
        createPortal(
          <div
            className="menu-layer"
            onMouseDown={(e) => {
              e.stopPropagation();
              close(false);
            }}
          >
            <div
              ref={pop}
              className="menu time-pop"
              role="dialog"
              aria-label={props["aria-label"]}
              aria-labelledby={props["aria-labelledby"]}
              style={pos ? { top: pos.top, left: pos.left } : { visibility: "hidden", top: 0, left: 0 }}
              onMouseDown={(e) => e.stopPropagation()}
              onKeyDown={onPopKey}
            >
              {column(
                names[0],
                hours.map((h) => ({
                  key: String(h),
                  text: h12 ? String(h) : pad2(h),
                  on: h === shownHour,
                  pick: () => pickHour(h),
                })),
                (by) => pickHour(hours[(hourIndex + by + hours.length) % hours.length]),
              )}
              {column(
                names[1],
                minutes.map((m) => ({ key: String(m), text: pad2(m), on: m === minute, pick: () => pickMinute(m) })),
                (by) => pickMinute((minute + by + 60) % 60),
              )}
              {h12 &&
                column(
                  names[2],
                  periods.map((text, i) => ({ key: text, text, on: pm === (i === 1), pick: () => pickPeriod(i === 1) })),
                  () => pickPeriod(!pm),
                )}
            </div>
          </div>,
          document.body,
        )}
    </>
  );
}

/* ---------- a day ---------- */

/** "2026-11-28" for a date, on this computer's calendar. */
export function dayValue(d: Date): string {
  return `${d.getFullYear()}-${pad2(d.getMonth() + 1)}-${pad2(d.getDate())}`;
}

/** The date "2026-11-28" stands for, at midnight; null for anything else. */
export function dateOfDay(value: string): Date | null {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value);
  return m ? new Date(Number(m[1]), Number(m[2]) - 1, Number(m[3])) : null;
}

const addDays = (d: Date, n: number) => new Date(d.getFullYear(), d.getMonth(), d.getDate() + n);
const addMonths = (d: Date, n: number) => {
  // the same day of the month, or the month's last day where it is shorter
  const last = new Date(d.getFullYear(), d.getMonth() + n + 1, 0).getDate();
  return new Date(d.getFullYear(), d.getMonth() + n, Math.min(d.getDate(), last));
};

/**
 * A day, as "YYYY-MM-DD", picked from a month in the Float material, in
 * place of the system's grey calendar. Weeks start on Monday, as in the
 * Scheduler's day picker. Arrow keys move by a day or a week, Page Up and
 * Page Down by a month, Enter picks, Escape closes.
 */
export function DateField(props: {
  value: string;
  onChange: (day: string) => void;
  /** The first day that can be picked. */
  min?: string;
  disabled?: boolean;
  "aria-label"?: string;
  "aria-labelledby"?: string;
}) {
  const trigger = useRef<HTMLButtonElement>(null);
  const pop = useRef<HTMLDivElement>(null);
  const [open, setOpen] = useState(false);
  const pos = usePopover(open, trigger, pop, () => close(false));
  const chosen = dateOfDay(props.value);
  const min = props.min ? dateOfDay(props.min) : null;
  /** The day the keyboard is on; its month is the one shown. */
  const [focus, setFocus] = useState<Date>(() => chosen ?? new Date());
  const today = dayValue(new Date());

  function show() {
    if (props.disabled) return;
    setFocus(chosen ?? min ?? new Date());
    setOpen(true);
  }

  function close(refocus = true) {
    setOpen(false);
    if (refocus) trigger.current?.focus();
  }

  const allowed = (d: Date) => !min || d >= min;

  function pick(d: Date) {
    if (!allowed(d)) return;
    close();
    const value = dayValue(d);
    if (value !== props.value) props.onChange(value);
  }

  // The day the keyboard is on holds the focus as it moves.
  useEffect(() => {
    if (!open || !pos) return;
    pop.current?.querySelector<HTMLElement>(`[data-day="${dayValue(focus)}"]`)?.focus({ preventScroll: true });
  }, [open, pos, focus]);

  const first = new Date(focus.getFullYear(), focus.getMonth(), 1);
  // the Monday on or before the 1st, then six weeks
  const start = addDays(first, -((first.getDay() + 6) % 7));
  const days = Array.from({ length: 42 }, (_, i) => addDays(start, i));
  const weekdays = Array.from({ length: 7 }, (_, i) => addDays(new Date(2024, 0, 1), i));
  const monthName = (d: Date) => dateText(d, { month: "long", year: "numeric" });
  const previous = addMonths(first, -1);
  const next = addMonths(first, 1);
  const canGoBack = !min || new Date(first.getFullYear(), first.getMonth(), 0) >= min;

  function onGridKey(e: React.KeyboardEvent) {
    const move: Record<string, () => Date> = {
      ArrowLeft: () => addDays(focus, document.documentElement.dir === "rtl" ? 1 : -1),
      ArrowRight: () => addDays(focus, document.documentElement.dir === "rtl" ? -1 : 1),
      ArrowUp: () => addDays(focus, -7),
      ArrowDown: () => addDays(focus, 7),
      PageUp: () => addMonths(focus, -1),
      PageDown: () => addMonths(focus, 1),
      Home: () => addDays(focus, -((focus.getDay() + 6) % 7)),
      End: () => addDays(focus, 6 - ((focus.getDay() + 6) % 7)),
    };
    if (move[e.key]) {
      e.preventDefault();
      const to = move[e.key]();
      setFocus(min && to < min ? min : to);
    }
  }

  function onPopKey(e: React.KeyboardEvent) {
    // Keys stay here: Escape closes the calendar, not the dialog it is in.
    e.stopPropagation();
    if (e.key === "Escape") {
      e.preventDefault();
      close();
    }
  }

  return (
    <>
      <button
        ref={trigger}
        type="button"
        className="select-btn date-btn"
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-label={props["aria-label"]}
        aria-labelledby={props["aria-labelledby"]}
        disabled={props.disabled}
        onClick={() => (open ? close() : show())}
        onKeyDown={(e) => {
          if (["ArrowDown", "ArrowUp", "Enter", " "].includes(e.key)) {
            e.preventDefault();
            show();
          }
        }}
      >
        <span className="select-value">
          {chosen ? dateText(chosen, { weekday: "short", day: "numeric", month: "short", year: "numeric" }) : props.value}
        </span>
        <CalendarDays className="select-chevron" size={15} strokeWidth={2} aria-hidden="true" />
      </button>
      {open &&
        createPortal(
          <div
            className="menu-layer"
            onMouseDown={(e) => {
              e.stopPropagation();
              close(false);
            }}
          >
            <div
              ref={pop}
              className="menu date-pop"
              role="dialog"
              aria-label={props["aria-label"]}
              aria-labelledby={props["aria-labelledby"]}
              style={pos ? { top: pos.top, left: pos.left } : { visibility: "hidden", top: 0, left: 0 }}
              onMouseDown={(e) => e.stopPropagation()}
              onKeyDown={onPopKey}
            >
              <div className="date-head">
                <button
                  type="button"
                  className="icon-btn"
                  aria-label={monthName(previous)}
                  title={monthName(previous)}
                  disabled={!canGoBack}
                  onClick={() => setFocus(min && previous < min ? min : previous)}
                >
                  <ChevronLeft size={16} strokeWidth={2.2} aria-hidden="true" />
                </button>
                <strong aria-live="polite">{monthName(first)}</strong>
                <button
                  type="button"
                  className="icon-btn"
                  aria-label={monthName(next)}
                  title={monthName(next)}
                  onClick={() => setFocus(next)}
                >
                  <ChevronRight size={16} strokeWidth={2.2} aria-hidden="true" />
                </button>
              </div>
              <div className="date-grid" role="grid" aria-label={monthName(first)} onKeyDown={onGridKey}>
                {weekdays.map((d) => (
                  <span key={d.getDay()} className="date-weekday" role="columnheader" title={dateText(d, { weekday: "long" })}>
                    {dateText(d, { weekday: "narrow" })}
                  </span>
                ))}
                {days.map((d) => {
                  const value = dayValue(d);
                  const inMonth = d.getMonth() === first.getMonth();
                  const isChosen = value === props.value;
                  const isFocus = value === dayValue(focus);
                  const cls = [
                    !inMonth && "is-outside",
                    value === today && "is-today",
                    isChosen && "is-checked",
                  ]
                    .filter(Boolean)
                    .join(" ");
                  return (
                    <button
                      key={value}
                      type="button"
                      role="gridcell"
                      data-day={value}
                      aria-selected={isChosen}
                      aria-current={value === today ? "date" : undefined}
                      aria-label={dateText(d, { weekday: "long", day: "numeric", month: "long", year: "numeric" })}
                      tabIndex={isFocus ? 0 : -1}
                      disabled={!allowed(d)}
                      className={cls || undefined}
                      onClick={() => pick(d)}
                      onKeyDown={(e) => {
                        if (e.key === "Enter" || e.key === " ") {
                          e.preventDefault();
                          pick(d);
                        }
                      }}
                    >
                      {d.getDate()}
                    </button>
                  );
                })}
              </div>
            </div>
          </div>,
          document.body,
        )}
    </>
  );
}

/** Pill button that opens a list of choices under itself. */
export function Dropdown<T extends string>(props: {
  label: string;
  value: T;
  options: { value: T; label: string; icon?: LucideIcon }[];
  onChange: (v: T) => void;
  /** Show the chosen option instead of the label once it differs from the first option. */
  showValue?: boolean;
}) {
  const [at, setAt] = useState<{ x: number; y: number } | null>(null);
  const changed = props.value !== props.options[0].value;
  const current = props.options.find((o) => o.value === props.value);
  return (
    <>
      <button
        type="button"
        className={`pill-btn${changed ? " is-set" : ""}`}
        aria-haspopup="menu"
        aria-expanded={at != null}
        onClick={(e) => {
          const r = e.currentTarget.getBoundingClientRect();
          setAt({ x: r.left, y: r.bottom + 6 });
        }}
      >
        {props.showValue && changed && current ? current.label : props.label}
        <ChevronDown size={15} strokeWidth={2.25} aria-hidden="true" />
      </button>
      {at && (
        <Menu
          x={at.x}
          y={at.y}
          onClose={() => setAt(null)}
          items={props.options.map((o) => ({
            label: o.label,
            icon: o.icon,
            checked: o.value === props.value,
            onSelect: () => props.onChange(o.value),
          }))}
        />
      )}
    </>
  );
}

/** Every action that applies to one download, in menu order. */
/** Asks for a folder, then moves a finished download there (never over a file). */
export async function moveDownload(d: Download, a: Actions) {
  const dir = await api.pickFolder(d.dir);
  if (!dir) return;
  try {
    const now = await api.moveDownload(d.id, dir);
    a.toast(t.organize.moved(now));
  } catch (e) {
    a.toast(String(e), "error");
  }
}

export function downloadMenu(d: Download, a: Actions): MenuItem[] {
  const items: MenuItem[] = [];
  if (d.batch) {
    const b = d.batch;
    const fail = (e: unknown) => a.toast(String(e), "error");
    items.push({ label: t.batch.details, icon: Layers, onSelect: () => a.inspect(d.id) });
    if (b.downloading + b.queued > 0) items.push({ label: t.batch.pauseAll, icon: Pause, onSelect: () => a.pause([d.id]) });
    if (b.paused > 0) items.push({ label: t.batch.resumeAll, icon: Play, onSelect: () => a.resume([d.id]) });
    if (b.failed + b.cancelled > 0) items.push({ label: t.batch.retryFailed, icon: RotateCw, onSelect: () => api.retryFailed(d.id).catch(fail) });
    if (d.status !== "completed" && d.status !== "failed") {
      items.push({ label: t.scheduler.menu, icon: CalendarClock, onSelect: () => a.schedule([d.id]) });
      items.push({ label: t.ui.limit.menu, icon: Gauge, onSelect: () => a.limit(d) });
    }
    items.push({ label: t.batch.openFolder, icon: FolderOpen, onSelect: () => api.showFolder(d.id).catch(fail) });
    if (d.status === "completed") items.push({ label: t.organize.moveTo, icon: FolderInput, onSelect: () => moveDownload(d, a) });
    items.push({ label: t.ui.menu.copyPath, icon: Copy, onSelect: () => a.copy(d.path, t.ui.copyWhat.path) });
    items.push("separator");
    const ended = d.status === "completed" || d.status === "failed";
    items.push({ label: ended ? t.ui.menu.removeFromList : t.batch.cancelBatch, icon: X, onSelect: () => a.remove([d.id]) });
    return items;
  }
  if (canPause(d)) items.push({ label: t.ui.menu.pause, icon: Pause, onSelect: () => a.pause([d.id]) });
  if (d.status === "paused") items.push({ label: t.ui.menu.resume, icon: Play, onSelect: () => a.resume([d.id]) });
  if (d.status === "failed") items.push({ label: t.ui.menu.retry, icon: RotateCw, onSelect: () => a.resume([d.id]) });
  if (takesNewLink(d)) items.push({ label: t.ui.refresh.menu, icon: LinkGlyph, onSelect: () => a.refreshLink(d) });
  if (d.status === "failed" && d.media && !d.profile && /^https?:/i.test(d.url)) {
    items.push({ label: t.ui.menu.openOnBrowser, icon: Globe, onSelect: () => a.browse(d.url) });
    items.push({ label: t.ui.menu.findMedia, icon: ScanSearch, onSelect: () => a.scan(d.url) });
  }
  // A file that is gone cannot be opened, shown or deleted; it can be fetched again.
  const gone = isDeleted(d);
  if (gone && /^(https?|magnet):/i.test(d.url)) {
    items.push({ label: t.ui.menu.downloadAgain, icon: RotateCw, onSelect: () => a.again(d) });
  }
  if (d.status !== "completed" && !gone) {
    items.push({ label: t.scheduler.menu, icon: CalendarClock, onSelect: () => a.schedule([d.id]) });
    items.push({ label: t.ui.limit.menu, icon: Gauge, onSelect: () => a.limit(d) });
  }
  if (d.status === "completed" && !gone) {
    items.push({ label: t.ui.menu.openFile, icon: SquareArrowOutUpRight, onSelect: () => a.open(d) });
    items.push({ label: t.ui.menu.showInFolder, icon: FolderOpen, onSelect: () => a.reveal(d) });
    items.push({ label: t.organize.moveTo, icon: FolderInput, onSelect: () => moveDownload(d, a) });
  }
  items.push("separator");
  items.push({ label: t.ui.menu.copyLink, icon: LinkGlyph, onSelect: () => a.copy(d.url, t.ui.copyWhat.link) });
  if (d.status === "completed" && !gone) {
    items.push({ label: t.ui.menu.copyPath, icon: Copy, onSelect: () => a.copy(d.path, t.ui.copyWhat.path) });
  }
  items.push("separator");
  items.push({
    label: gone ? t.ui.menu.removeFromHistory : d.status === "completed" ? t.ui.menu.removeFromList : t.ui.menu.cancelAndRemove,
    icon: X,
    onSelect: () => a.remove([d.id]),
  });
  if (d.status === "completed" && !gone) {
    items.push({ label: t.ui.menu.deleteFile, icon: Trash2, danger: true, onSelect: () => a.deleteFile(d) });
  }
  return items;
}

/* ---------- download row ---------- */

/** The round control at the end of a row: what you can do, or how it ended. */
function RowAction({ d }: { d: Download }) {
  const { actions } = useApp();
  // Nothing is left to open; what can be done is to fetch it again.
  if (isDeleted(d)) {
    const again = /^(https?|magnet):/i.test(d.url);
    return (
      <button
        type="button"
        className="round round-deleted"
        title={again ? t.ui.row.deletedAgain : t.ui.row.deleted}
        aria-label={again ? t.ui.row.deletedAgain : t.ui.row.deleted}
        disabled={!again}
        onClick={() => actions.again(d)}
      >
        <RotateCw size={14} strokeWidth={2.5} aria-hidden="true" />
      </button>
    );
  }
  // A file that failed its checksum is not opened with one click: its
  // details, which say what is wrong, open instead.
  if (d.status === "completed" && d.checksum?.state === "failed") {
    return (
      <button type="button" className="round round-err" title={t.checksum.rowFailed} aria-label={t.checksum.rowFailed} onClick={() => actions.inspect(d.id)}>
        <span aria-hidden="true">!</span>
      </button>
    );
  }
  if (d.status === "completed") {
    return (
      <button type="button" className="round round-ok" title={t.ui.row.openFile} aria-label={t.ui.row.completedOpen} onClick={() => actions.open(d)}>
        <Check size={15} strokeWidth={3} aria-hidden="true" />
      </button>
    );
  }
  if (d.status === "failed") {
    return (
      <button type="button" className="round round-err" title={t.ui.row.retry} aria-label={t.ui.row.failedRetry} onClick={() => actions.resume([d.id])}>
        <span aria-hidden="true">!</span>
      </button>
    );
  }
  if (d.status === "paused") {
    return (
      <button type="button" className="round" title={t.ui.row.resume} aria-label={t.ui.row.resume} onClick={() => actions.resume([d.id])}>
        <Play size={15} strokeWidth={0} fill="currentColor" aria-hidden="true" />
      </button>
    );
  }
  return (
    <button type="button" className="round" title={t.ui.row.pause} aria-label={t.ui.row.pause} onClick={() => actions.pause([d.id])}>
      <Pause size={15} strokeWidth={0} fill="currentColor" aria-hidden="true" />
    </button>
  );
}

export function DownloadRow(props: {
  d: Download;
  selected: boolean;
  onSelect: (e: ReactMouseEvent) => void;
  onMenu: (x: number, y: number) => void;
}) {
  const { d } = props;
  const { actions } = useApp();
  const active = d.status === "downloading" || d.status === "connecting";
  const eta = etaOf(d);

  return (
    <li
      className={`row row-${d.status}${isDeleted(d) ? " is-deleted" : ""}${props.selected ? " is-selected" : ""}`}
      aria-selected={props.selected}
      onClick={props.onSelect}
      onContextMenu={(e) => {
        e.preventDefault();
        props.onMenu(e.clientX, e.clientY);
      }}
    >
      <FileIcon d={d} />
      <div className="row-main">
        <div className="row-name" title={displayName(d)}>{displayName(d)}</div>
        {d.batch?.current && active && (
          <div className="row-now" title={d.batch.current}>{t.batch.now(d.batch.current)}</div>
        )}
        <div className="row-line">
          <span className="row-meta" title={d.status === "failed" ? d.error ?? undefined : undefined}>
            <RowMeta d={d} />
          </span>
          {eta && <span className="row-eta">{eta}</span>}
        </div>
        {(active || (d.status === "paused" && d.total)) && <Progress d={d} />}
      </div>
      <div className="row-actions" onClick={(e) => e.stopPropagation()}>
        {d.status === "completed" && d.took != null && !isDeleted(d) && (
          <span className="row-took" title={t.ui.row.took(formatTook(d.took))}>
            <Timer size={13} strokeWidth={2} aria-hidden="true" />
            {formatTook(d.took)}
          </span>
        )}
        <IconBtn
          icon={FolderOpen}
          label={t.ui.menu.showInFolder}
          onClick={() => api.showFolder(d.id).catch((err) => actions.toast(String(err), "error"))}
        />
        <RowAction d={d} />
        <IconBtn
          icon={Ellipsis}
          label={t.ui.row.more}
          onClick={(e) => {
            const r = e.currentTarget.getBoundingClientRect();
            props.onMenu(r.right - 210, r.bottom + 4);
          }}
        />
      </div>
    </li>
  );
}

/* ---------- toasts ---------- */

export interface Toast {
  id: number;
  message: string;
  tone: "error" | "info";
}

export function Toasts(props: { toasts: Toast[]; dismiss: (id: number) => void }) {
  return (
    <div className="toasts" aria-live="polite">
      {props.toasts.map((item) => (
        <div key={item.id} className={`toast toast-${item.tone}`} role={item.tone === "error" ? "alert" : "status"}>
          {item.tone === "error" ? (
            <CircleAlert size={17} strokeWidth={2} aria-hidden="true" />
          ) : (
            <CircleCheck size={17} strokeWidth={2} aria-hidden="true" />
          )}
          <span>{item.message}</span>
          <IconBtn icon={X} label={t.ui.dismiss} onClick={() => props.dismiss(item.id)} />
        </div>
      ))}
    </div>
  );
}

/* ---------- modal shell ---------- */

export function Modal(props: {
  title: string;
  subtitle?: string;
  onClose: () => void;
  children: ReactNode;
  footer?: ReactNode;
  wide?: boolean;
  /** Hide the title row's text (the body supplies its own heading). */
  bare?: boolean;
}) {
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const previous = document.activeElement as HTMLElement | null;
    const root = ref.current;
    const first =
      root?.querySelector<HTMLElement>("[data-autofocus]") ??
      root?.querySelector<HTMLElement>(".dialog-body input, .dialog-body select") ??
      root?.querySelector<HTMLElement>(".dialog-foot .btn-primary, .dialog-foot button") ??
      root?.querySelector<HTMLElement>("button");
    first?.focus();
    return () => previous?.focus?.();
  }, []);

  // Escape closes; Tab stays inside the dialog.
  function onKeyDown(e: React.KeyboardEvent) {
    if (e.key === "Escape") {
      e.stopPropagation();
      props.onClose();
      return;
    }
    if (e.key !== "Tab") return;
    const focusable = Array.from(
      ref.current?.querySelectorAll<HTMLElement>(
        "button:not(:disabled), input:not(:disabled), select:not(:disabled), [tabindex='0']",
      ) ?? [],
    );
    if (focusable.length === 0) return;
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    if (e.shiftKey && document.activeElement === first) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && document.activeElement === last) {
      e.preventDefault();
      first.focus();
    }
  }

  return (
    <div className="overlay" onMouseDown={props.onClose}>
      <div
        ref={ref}
        className={`dialog${props.wide ? " dialog-wide" : ""}${props.bare ? " dialog-bare" : ""}`}
        role="dialog"
        aria-modal="true"
        aria-label={props.title}
        onMouseDown={(e) => e.stopPropagation()}
        onKeyDown={onKeyDown}
      >
        <div className="dialog-head">
          <div className="dialog-titles">
            {!props.bare && <h2>{props.title}</h2>}
            {props.subtitle && <p title={props.subtitle}>{props.subtitle}</p>}
          </div>
          <IconBtn icon={X} label={t.ui.close} onClick={props.onClose} />
        </div>
        <div className="dialog-body">{props.children}</div>
        {props.footer && <div className="dialog-foot">{props.footer}</div>}
      </div>
    </div>
  );
}
