import {
  createContext,
  useContext,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type MouseEvent as ReactMouseEvent,
  type ReactNode,
} from "react";
import {
  Archive,
  Check,
  ChevronDown,
  CircleAlert,
  CircleCheck,
  Copy,
  Ellipsis,
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
  Trash2,
  X,
  type LucideIcon,
} from "lucide-react";
import {
  api,
  canPause,
  categoryOf,
  deletedAgo,
  displayName,
  isDeleted,
  etaOf,
  formatBytes,
  formatSpeed,
  percentOf,
  STATUS_LABEL,
  type Category,
  type Download,
  type MediaOptions,
  type Settings,
} from "./api";

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
  toast: (message: string, tone?: "error" | "info") => void;
}

/** How progress is drawn: one bar, or one bar for each connection. */
export type ProgressStyle = "single" | "threads";

export interface AppState {
  downloads: Download[];
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

/** Coloured file tile; the colour says what kind of file it is. A profile
 *  being saved shows whose it is instead: the profile's own picture. */
export function FileIcon({ d, large }: { d: Download; large?: boolean }) {
  const category = categoryOf(d);
  const Icon = CATEGORY_ICON[category];
  const [picture, pictureFailed] = useProfilePicture(d);
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
  return (
    <span className={`file-icon cat-${category}${large ? " file-icon-lg" : ""}`} aria-hidden="true">
      <Icon size={large ? 26 : 18} strokeWidth={2} />
    </span>
  );
}

/** Dot plus status word, used where a download is listed outside the main list. */
export function StatusText({ d }: { d: Download }) {
  return (
    <span className={`status status-${d.status}`}>
      <span className="status-dot" aria-hidden="true" />
      {STATUS_LABEL[d.status]}
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
    failures >= 2 ? "Still failing to connect, trying again" : failures === 1 ? "Connection failed, trying again" : "Connecting";
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

/** One continuous bar of overall progress. */
export function Progress({ d, tall }: { d: Download; tall?: boolean }) {
  const cls = `bar bar-${d.status}${tall ? " bar-tall" : ""}`;
  const pct = percentOf(d);
  const connecting = d.status === "connecting" && <ConnectingBar tall={tall} failures={d.connect_failures ?? 0} />;
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
          aria-label={`${d.filename} progress, ${parts.length} parts`}
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
        aria-label={`${d.filename} progress`}
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
  return `${n} ${n === 1 ? "file" : "files"}`;
}

/** "72.5 MB / 1.2 GB" for anything unfinished. */
export function amountOf(d: Download): string {
  if (d.profile) return `${fileCount(d.items)} · ${formatBytes(d.downloaded)}`;
  return d.total != null
    ? `${formatBytes(d.downloaded)} / ${formatBytes(d.total)}`
    : formatBytes(d.downloaded);
}

/** Line under the file name: status word in colour, then the facts. */
export function RowMeta({ d }: { d: Download }) {
  switch (d.status) {
    case "downloading":
      return <>{amountOf(d)} · {formatSpeed(d.speed)}</>;
    case "connecting":
      return <>Connecting</>;
    case "queued":
      return <>Queued{d.total != null ? ` · ${formatBytes(d.total)}` : " · waiting for a free slot"}</>;
    case "paused":
      return <>Paused · {amountOf(d)}</>;
    case "completed":
      // The file is gone from where it was saved: that is what there is to say.
      if (isDeleted(d)) {
        return (
          <>
            <span className="tone-deleted">Deleted</span> · {deletedAgo(d)}
            {d.total != null ? ` · was ${formatBytes(d.total)}` : ""}
          </>
        );
      }
      return (
        <>
          <span className="tone-ok">Completed</span>
          {d.profile ? ` · ${fileCount(d.items)}` : ""}
          {d.total != null ? ` · ${formatBytes(d.total)}` : ""}
        </>
      );
    case "failed":
      return (
        <>
          <span className="tone-err">Failed</span> · {d.error ?? "Unknown error"}
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
        Speed appears here while the download is running.
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
    const t = Math.min(1, Math.max(0, (px - seg * span) / span));
    const y0 = main[Math.max(0, seg - 1)][1];
    const y1 = main[seg][1];
    const y2 = main[seg + 1][1];
    const y3 = main[Math.min(count - 1, seg + 2)][1];
    const c1 = edge(y1 + (y2 - y0) / 6);
    const c2 = edge(y2 - (y3 - y1) / 6);
    const u = 1 - t;
    return u * u * u * y1 + 3 * u * u * t * c1 + 3 * u * t * t * c2 + t * t * t * y2;
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
            <small>{secondsAgo === 0 ? "now" : `about ${secondsAgo}s ago`}</small>
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
export function downloadMenu(d: Download, a: Actions): MenuItem[] {
  const items: MenuItem[] = [];
  if (canPause(d)) items.push({ label: "Pause", icon: Pause, onSelect: () => a.pause([d.id]) });
  if (d.status === "paused") items.push({ label: "Resume", icon: Play, onSelect: () => a.resume([d.id]) });
  if (d.status === "failed") items.push({ label: "Retry", icon: RotateCw, onSelect: () => a.resume([d.id]) });
  if (d.status === "failed" && d.media && !d.profile && /^https?:/i.test(d.url)) {
    items.push({ label: "Open on Browser page", icon: Globe, onSelect: () => a.browse(d.url) });
    items.push({ label: "Find media on this page", icon: ScanSearch, onSelect: () => a.scan(d.url) });
  }
  // A file that is gone cannot be opened, shown or deleted; it can be fetched again.
  const gone = isDeleted(d);
  if (gone && /^(https?|magnet):/i.test(d.url)) {
    items.push({ label: "Download again", icon: RotateCw, onSelect: () => a.again(d) });
  }
  if (d.status === "completed" && !gone) {
    items.push({ label: "Open file", icon: SquareArrowOutUpRight, onSelect: () => a.open(d) });
    items.push({ label: "Show in folder", icon: FolderOpen, onSelect: () => a.reveal(d) });
  }
  items.push("separator");
  items.push({ label: "Copy link", icon: LinkGlyph, onSelect: () => a.copy(d.url, "Link") });
  if (d.status === "completed" && !gone) {
    items.push({ label: "Copy file path", icon: Copy, onSelect: () => a.copy(d.path, "File path") });
  }
  items.push("separator");
  items.push({
    label: gone ? "Remove from history" : d.status === "completed" ? "Remove from list" : "Cancel and remove",
    icon: X,
    onSelect: () => a.remove([d.id]),
  });
  if (d.status === "completed" && !gone) {
    items.push({ label: "Delete file from disk", icon: Trash2, danger: true, onSelect: () => a.deleteFile(d) });
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
        title={again ? "Deleted. Download again" : "Deleted"}
        aria-label={again ? "Deleted. Download again" : "Deleted"}
        disabled={!again}
        onClick={() => actions.again(d)}
      >
        <RotateCw size={14} strokeWidth={2.5} aria-hidden="true" />
      </button>
    );
  }
  if (d.status === "completed") {
    return (
      <button type="button" className="round round-ok" title="Open file" aria-label="Completed. Open file" onClick={() => actions.open(d)}>
        <Check size={15} strokeWidth={3} aria-hidden="true" />
      </button>
    );
  }
  if (d.status === "failed") {
    return (
      <button type="button" className="round round-err" title="Retry" aria-label="Failed. Retry" onClick={() => actions.resume([d.id])}>
        <span aria-hidden="true">!</span>
      </button>
    );
  }
  if (d.status === "paused") {
    return (
      <button type="button" className="round" title="Resume" aria-label="Resume" onClick={() => actions.resume([d.id])}>
        <Play size={15} strokeWidth={0} fill="currentColor" aria-hidden="true" />
      </button>
    );
  }
  return (
    <button type="button" className="round" title="Pause" aria-label="Pause" onClick={() => actions.pause([d.id])}>
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
        <div className="row-line">
          <span className="row-meta" title={d.status === "failed" ? d.error ?? undefined : undefined}>
            <RowMeta d={d} />
          </span>
          {eta && <span className="row-eta">{eta}</span>}
        </div>
        {(active || (d.status === "paused" && d.total)) && <Progress d={d} />}
      </div>
      <div className="row-actions" onClick={(e) => e.stopPropagation()}>
        <RowAction d={d} />
        <IconBtn
          icon={Ellipsis}
          label="More actions"
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
      {props.toasts.map((t) => (
        <div key={t.id} className={`toast toast-${t.tone}`} role={t.tone === "error" ? "alert" : "status"}>
          {t.tone === "error" ? (
            <CircleAlert size={17} strokeWidth={2} aria-hidden="true" />
          ) : (
            <CircleCheck size={17} strokeWidth={2} aria-hidden="true" />
          )}
          <span>{t.message}</span>
          <IconBtn icon={X} label="Dismiss" onClick={() => props.dismiss(t.id)} />
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
          <IconBtn icon={X} label="Close" onClick={props.onClose} />
        </div>
        <div className="dialog-body">{props.children}</div>
        {props.footer && <div className="dialog-foot">{props.footer}</div>}
      </div>
    </div>
  );
}
