import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { open as openDialog } from "@tauri-apps/plugin-dialog";

/** False when the interface is opened in a plain browser for development. */
export const isTauri = "__TAURI_INTERNALS__" in window;

/** An address the window can show a downloaded file from, or null where it
 *  cannot (a plain browser). Only files of a download whose list has been
 *  asked for are let through. */
export function fileSrc(path: string): string | null {
  return isTauri ? convertFileSrc(path) : null;
}

const VIDEO_FILE = /\.(mp4|m4v|mov|webm|mkv)$/i;
const PICTURE_FILE = /\.(jpe?g|jfif|png|gif|webp|bmp|avif|svg)$/i;

/** What kind of picture a downloaded file can give of itself: a photo is
 *  shown as it is, a video by its first frame, anything else by neither. */
export function pictureKind(name: string): "photo" | "video" | null {
  return PICTURE_FILE.test(name) ? "photo" : VIDEO_FILE.test(name) ? "video" : null;
}

export type Status =
  | "queued"
  | "connecting"
  | "downloading"
  | "paused"
  | "completed"
  | "failed";

export interface Download {
  id: number;
  url: string;
  filename: string;
  dir: string;
  path: string;
  status: Status;
  error: string | null;
  total: number | null;
  downloaded: number;
  speed: number;
  resumable: boolean;
  media: boolean;
  /** A BitTorrent download; `path` is the torrent's own folder. */
  torrent: boolean;
  connections: number;
  active_connections: number;
  /** Connection attempts that have failed in a row while connecting. */
  connect_failures: number;
  /** A whole social media profile, saved as a folder. */
  profile: boolean;
  /** Files fetched so far, for profile downloads. */
  items: number;
  /** The file a running profile or playlist download is fetching right now. */
  current: CurrentFile | null;
  /** The picture the site shows for a video or playlist, once it is known. */
  thumbnail: string | null;
  /** When a finished download's file was found to be gone (deleted or moved
   *  away outside the app), in seconds since 1970; null while it is there. */
  deleted_at: number | null;
  created_at: number;
  /** [start, position, end] per segment */
  segments: [number, number, number][];
}

/** One file of a profile or playlist, while it is being fetched. */
export interface CurrentFile {
  name: string;
  downloaded: number;
  /** Its full size, where the site says. */
  total: number | null;
  /** Its place in a playlist, counted from 1. */
  index: number | null;
  /** How many files the playlist lists. */
  count: number | null;
}

/** One video or file of a profile or playlist download. */
export interface DownloadItem {
  title: string;
  /** "missed": the download finished without it. "deleted": it was fetched,
   *  and its file has since been deleted or moved away. */
  state: "done" | "downloading" | "waiting" | "missed" | "deleted";
  /** Its size on disk, once it is there. */
  size: number | null;
  /** The name of its file, once it is there. */
  file: string | null;
  /** Where that file is on this computer, so that it can be shown. */
  path: string | null;
  /** Its picture on the site. */
  thumbnail: string | null;
  /** How long a video runs, in seconds. */
  duration: number | null;
  /** Who posted it. */
  channel: string | null;
  /** Its own page on the site. */
  url: string | null;
}

/** What to take from a video page; ignored for direct file links. */
export interface MediaOptions {
  max_height: number | null;
  audio_only: boolean;
}

export interface NewDownload {
  url: string;
  media: MediaOptions;
  dir?: string;
  filename?: string;
  connections?: number;
  headers?: [string, string][];
  via_site?: boolean;
  /** For a torrent: indexes of the files to download; leave out for all. */
  torrent_files?: number[];
}

/** One file inside a torrent. */
export interface TorrentFile {
  index: number;
  /** Folders, then the file name. */
  path: string[];
  size: number;
}

/** What a torrent contains, read before anything is downloaded. */
export interface TorrentPreview {
  name: string;
  total: number;
  files: TorrentFile[];
}

/** What a video page offers, shown before the download starts. */
export interface MediaInfo {
  title: string;
  thumbnail: string | null;
  /** Length in seconds. */
  duration: number | null;
  /** Picture heights on offer, largest first. */
  heights: number[];
  site: string | null;
}

/** A media link found by scanning a page. */
export interface Found {
  url: string;
  kind: "video" | "stream" | "audio" | "embed";
  name: string;
  size: number | null;
  /** Page the link was found on. */
  referer: string;
}

export interface Settings {
  download_dir: string;
  max_concurrent: number;
  connections: number;
  /** bytes per second, 0 = unlimited */
  speed_limit: number;
  /** Browser whose sign-in site downloads may use; null = never. */
  cookies_browser: string | null;
  /** Look site names up at Cloudflare (1.1.1.1) instead of the network's own resolver. */
  secure_dns: boolean;
}

export const COOKIE_BROWSERS: { value: string; label: string }[] = [
  { value: "firefox", label: "Firefox" },
  { value: "chrome", label: "Chrome" },
  { value: "edge", label: "Edge" },
  { value: "brave", label: "Brave" },
  { value: "opera", label: "Opera" },
  { value: "vivaldi", label: "Vivaldi" },
];

/** A video or audio file a browsed page loaded. */
export interface CaughtMedia {
  url: string;
  kind: "video" | "audio" | "stream";
  /** The page (or frame) that loaded it. */
  referer: string;
  /** Its resolution when known ("720p"), else empty. */
  label: string;
  /** For a stream: the picture heights its playlist names, largest first. */
  heights: number[];
  /** For a stream: this is the playlist that lists every quality. */
  master: boolean;
}

/** A rectangle of the window, in the interface's own pixels. */
export interface Area {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** Sites with a sign-in page that can be opened inside the app. */
/** The sites offered on the Sign in page, the most used first. */
export const SIGN_IN_SITES: { label: string; url: string }[] = [
  { label: "YouTube", url: "https://www.youtube.com/signin" },
  { label: "Instagram", url: "https://www.instagram.com/accounts/login/" },
  { label: "TikTok", url: "https://www.tiktok.com/login" },
  { label: "Facebook", url: "https://www.facebook.com/login" },
  { label: "X", url: "https://x.com/login" },
  { label: "Vimeo", url: "https://vimeo.com/log_in" },
  { label: "Dailymotion", url: "https://www.dailymotion.com/signin" },
  { label: "Reddit", url: "https://www.reddit.com/login" },
  { label: "VK", url: "https://vk.com/login" },
  { label: "Pinterest", url: "https://www.pinterest.com/login/" },
  { label: "Twitch", url: "https://www.twitch.tv/login" },
  { label: "Bilibili", url: "https://passport.bilibili.com/login" },
  { label: "Snapchat", url: "https://accounts.snapchat.com/accounts/login" },
  { label: "Rumble", url: "https://rumble.com/login.php" },
  // Kick signs in from a panel on its front page; it has no address of its own.
  { label: "Kick", url: "https://kick.com/" },
  { label: "Streamable", url: "https://streamable.com/login" },
  { label: "Odysee", url: "https://odysee.com/$/signin" },
  { label: "Tumblr", url: "https://www.tumblr.com/login" },
  { label: "LinkedIn", url: "https://www.linkedin.com/login" },
  { label: "Threads", url: "https://www.threads.com/login" },
];

/** Other addresses the same sites answer to. */
const SITE_ALIASES: Record<string, string> = {
  "youtu.be": "YouTube",
  "twitter.com": "X",
  "fb.watch": "Facebook",
  "threads.net": "Threads",
  "b23.tv": "Bilibili",
  "vk.ru": "VK",
  "vkvideo.ru": "VK",
  "pin.it": "Pinterest",
};

/** The site on the Sign in page a link belongs to ("Instagram"), if it is one of them. */
export function signInSiteOf(link: string): string | null {
  try {
    const host = new URL(link.trim()).hostname.toLowerCase();
    const under = (base: string) => host === base || host.endsWith(`.${base}`);
    for (const [base, label] of Object.entries(SITE_ALIASES)) if (under(base)) return label;
    for (const site of SIGN_IN_SITES) {
      if (under(new URL(site.url).hostname.split(".").slice(-2).join("."))) return site.label;
    }
  } catch {
    // not a web address
  }
  return null;
}

/**
 * Whether a failed lookup or download failed because the site wants a
 * signed-in visitor: a private or members-only post, an age check, or a site
 * that shows nothing to visitors who are not signed in.
 */
export function needsSignIn(error: string): boolean {
  const said = error.toLowerCase();
  return [
    "sign in",
    "log in",
    "login",
    "logged in",
    "logged-in",
    "private",
    "--cookies",
    "authentication",
    "members-only",
    "members only",
    "registered users",
    "confirm your age",
    "age-restricted",
    "age restricted",
  ].some((sign) => said.includes(sign));
}

export interface ToolStatus {
  yt_dlp: boolean;
  ffmpeg: boolean;
  gallery_dl: boolean;
}

/** What a playlist holds, shown before it is downloaded. */
export interface PlaylistInfo {
  title: string;
  /** How many videos it lists. */
  count: number;
  /** How many photos it lists. */
  photos: number;
  /** The counting stopped early: it holds more than this. */
  more: boolean;
  /** How long all its videos run together, in seconds, where the site says. */
  duration: number | null;
  /** A few of its videos, looked up to see which resolutions it really has. */
  samples: string[];
  thumbnail: string | null;
  channel: string | null;
}

/** Whose profile it is, as its page says: each part only where the site gives it. */
export interface ProfileFace {
  /** The name the profile shows, which is not always the one in its link. */
  name: string | null;
  picture: string | null;
  /** The profile's own line about itself, or the site's line about it. */
  about: string | null;
  /** The wide picture across the top of the profile. */
  banner: string | null;
  /** How many posts it has made, and how many of them carry a photo or video. */
  posts: number | null;
  media: number | null;
  followers: number | null;
  following: number | null;
  /** The day it joined the site, as YYYY-MM-DD. */
  joined: string | null;
}

/** A social media profile recognised in a link. */
export interface ProfileRef {
  site: string;
  name: string;
  /** Which program fetches it, for a collection found by asking the
   *  programs (any site they know); absent for the sites the app reads itself. */
  by?: "ytdlp" | "gallerydl";
}

export const api = {
  list: () => invoke<Download[]>("list_downloads"),
  /** The videos or files of a profile or playlist download, with how far each is. */
  items: (id: number) => invoke<DownloadItem[]>("download_items", { id }),
  add: (request: NewDownload) => invoke<number>("add_download", { request }),
  scan: (url: string) => invoke<Found[]>("scan_page", { url }),
  torrentPreview: (source: string) => invoke<TorrentPreview>("torrent_preview", { source }),
  mediaInfo: (url: string) => invoke<MediaInfo>("media_info", { url }),
  /** The profile a link points at, or null for any other link. */
  profileOf: (url: string) => invoke<ProfileRef | null>("profile_of", { url }),
  /** Whether a page is a whole collection (someone's videos, photos or tracks)
   *  on any site the downloading programs know. A few seconds the first time. */
  collectionOf: (url: string) => invoke<ProfileRef | null>("collection_of", { url }),
  /** A playlist's name, picture and number of videos. Nothing is downloaded. */
  playlistInfo: (url: string) => invoke<PlaylistInfo>("playlist_info", { url }),
  /** Whose profile a link points at. Quick, and works without being signed in. */
  profileFace: (url: string) => invoke<ProfileFace>("profile_face", { url }),
  /** The same with its numbers (posts, followers, joined), asked as a visitor who is not signed in. */
  profileStats: (url: string) => invoke<ProfileFace>("profile_stats", { url }),
  /** Calls back with the paths of files dropped onto the window. */
  onFileDrop: (cb: (paths: string[]) => void) =>
    getCurrentWebview().onDragDropEvent((e) => {
      if (e.payload.type === "drop") cb(e.payload.paths);
    }),
  pause: (id: number) => invoke<void>("pause_download", { id }),
  resume: (id: number) => invoke<void>("resume_download", { id }),
  remove: (id: number, deleteFile: boolean) =>
    invoke<void>("remove_download", { id, deleteFile }),
  open: (id: number) => invoke<void>("open_file", { id }),
  reveal: (id: number) => invoke<void>("reveal_file", { id }),
  openDownloadDir: () => invoke<void>("open_download_dir"),
  tools: () => invoke<ToolStatus>("tool_status"),
  getSettings: () => invoke<Settings>("get_settings"),
  setSettings: (settings: Settings) => invoke<void>("set_settings", { settings }),
  /** Shows a site's sign-in page inside the window, over the given area. */
  signinOpen: (url: string, area: Area, browse = false) =>
    invoke<void>("signin_open", { url, ...area, browse }),
  /** Back, forward or reload on the Browser page. */
  browserGo: (action: "back" | "forward" | "reload") => invoke<void>("browser_go", { action }),
  /** Starts the player on the browsed page, as a visitor pressing play would. */
  browserAutoplay: () => invoke<void>("browser_autoplay"),
  /** Keeps the browsing session's sign-in for a download about to start. */
  browserKeepSignIn: () => invoke<void>("browser_keep_sign_in"),
  /** Size in bytes of a file a browsed page offers; null when the server does not say. */
  linkSize: (url: string, referer: string) =>
    invoke<number | null>("link_size", { url, referer, agent: navigator.userAgent }),
  /** Calls back with each video or audio file a browsed page loads. */
  onBrowserMedia: (cb: (found: CaughtMedia) => void) =>
    listen<CaughtMedia>("browser-media", (e) => cb(e.payload)),
  /** Calls back with the address the browsed page moves to. */
  onBrowserNav: (cb: (url: string) => void) => listen<string>("browser-nav", (e) => cb(e.payload)),
  onBrowserTitle: (cb: (title: string) => void) => listen<string>("browser-title", (e) => cb(e.payload)),
  /** Calls back with the picture a browsed page offers for itself. */
  onBrowserPage: (cb: (picture: string) => void) => listen<string>("browser-page", (e) => cb(e.payload)),
  /** Calls back with an address a browsed page tried to open in a new window. */
  onBrowserPopup: (cb: (url: string) => void) => listen<string>("browser-popup", (e) => cb(e.payload)),
  /** Calls back with the host a sign-in page tried to open in a new window. */
  onSignInPopup: (cb: (host: string) => void) => listen<string>("sign-in-popup", (e) => cb(e.payload)),
  /** Keeps the sign-in page over its area. */
  signinBounds: (area: Area) => invoke<void>("signin_bounds", { ...area }),
  /** Saves the sign-in, closes the page and resolves with every saved site. */
  signinClose: () => invoke<string[]>("signin_close"),
  /** Sites a sign-in is saved for. */
  signInStatus: () => invoke<string[]>("sign_in_status"),
  forgetSignIns: () => invoke<void>("forget_sign_ins"),
  /** Calls back with a link the user just copied, anywhere on the computer. */
  onClipboardLink: (cb: (link: string) => void) => listen<string>("clipboard-link", (e) => cb(e.payload)),
  /** Turns picking up copied links on or off. */
  setClipboardWatch: (on: boolean) => invoke<void>("set_clipboard_watch", { on }),
  /** Picks a cookies file exported from a browser; null when cancelled. */
  pickCookiesFile: async (): Promise<string | null> => {
    const picked = await openDialog({
      multiple: false,
      filters: [{ name: "Cookies file", extensions: ["txt"] }],
    });
    return typeof picked === "string" ? picked : null;
  },
  /** Adds the sign-ins in a cookies file; resolves with every saved site. */
  importSignIn: (path: string) => invoke<string[]>("import_sign_in", { path }),
  /** Brings over the sign-ins a browser on this computer keeps; resolves with every saved site. */
  importBrowserSignIn: (browser: string) => invoke<string[]>("import_browser_sign_in", { browser }),
  onChange: (cb: (downloads: Download[]) => void) =>
    listen<Download[]>("downloads", (e) => cb(e.payload)),
  /** Picks a .torrent file or a text file holding a list of links; null when cancelled. */
  pickLinkFile: async (): Promise<string | null> => {
    const picked = await openDialog({
      multiple: false,
      filters: [
        { name: "Torrents and link lists", extensions: ["torrent", "txt", "csv", "list", "md", "html"] },
        { name: "Torrent files", extensions: ["torrent"] },
      ],
    });
    return typeof picked === "string" ? picked : null;
  },
  importLinks: (path: string) => invoke<string[]>("import_links", { path }),
  windowMinimize: () => getCurrentWindow().minimize(),
  windowToggleMaximize: () => getCurrentWindow().toggleMaximize(),
  windowClose: () => getCurrentWindow().close(),
  /** Native folder picker; resolves null when cancelled. */
  pickFolder: async (defaultPath?: string): Promise<string | null> => {
    const picked = await openDialog({ directory: true, defaultPath });
    return typeof picked === "string" ? picked : null;
  },
};

/* ---------- formatting ---------- */

const UNITS = ["B", "KB", "MB", "GB", "TB"];

export function formatBytes(n: number): string {
  let i = 0;
  while (n >= 1024 && i < UNITS.length - 1) {
    n /= 1024;
    i++;
  }
  return `${i === 0 ? Math.round(n) : n.toFixed(n >= 100 ? 0 : 1)} ${UNITS[i]}`;
}

export function formatSpeed(n: number): string {
  return `${formatBytes(n)}/s`;
}

export function formatEta(seconds: number): string {
  if (!isFinite(seconds) || seconds < 0) return "";
  const s = Math.ceil(seconds);
  if (s < 60) return `${s}s left`;
  if (s < 3600) return `${Math.floor(s / 60)}m ${s % 60}s left`;
  return `${Math.floor(s / 3600)}h ${Math.floor((s % 3600) / 60)}m left`;
}

export function formatDate(unixSeconds: number): string {
  if (!unixSeconds) return "Unknown";
  return new Date(unixSeconds * 1000).toLocaleString(undefined, {
    day: "numeric",
    month: "short",
    year: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

/** Name to show for a download. A site link whose title is not known yet
 *  (still starting, or failed before the site answered) shows its address
 *  instead of a meaningless path piece such as "watch". */
export function displayName(d: Download): string {
  if (d.media && !/\.[a-z0-9]{2,5}$/i.test(d.filename)) {
    return d.url.replace(/^https?:\/\/(www\.)?/i, "");
  }
  return d.filename;
}

export function percentOf(d: Download): number | null {
  if (d.status === "completed") return 100;
  if (!d.total) return null;
  return Math.min(100, Math.floor((d.downloaded / d.total) * 100));
}

export function etaOf(d: Download): string {
  if (d.status !== "downloading" || !d.total || d.speed <= 0) return "";
  return formatEta((d.total - d.downloaded) / d.speed);
}

/* ---------- status ---------- */

export const STATUS_LABEL: Record<Status, string> = {
  queued: "Queued",
  connecting: "Connecting",
  downloading: "Downloading",
  paused: "Paused",
  completed: "Completed",
  failed: "Failed",
};

/** A finished download whose file is no longer where it was saved. */
export const isDeleted = (d: Download) => d.status === "completed" && d.deleted_at != null;

/** How long a deleted download stays on the Downloads page before it is
 *  only to be found in History. */
export const DELETED_SHOWN_DAYS = 10;

/** Whether a download belongs on the Downloads page: everything that is
 *  running, waiting, failed or still on disk, and what was deleted lately. */
export function onDownloadsPage(d: Download, now = Date.now()): boolean {
  if (!isDeleted(d)) return true;
  return now / 1000 - d.deleted_at! <= DELETED_SHOWN_DAYS * 86400;
}

/** "today", "yesterday", "3 days ago": how long ago a file was found deleted. */
export function deletedAgo(d: Download, now = Date.now()): string {
  const days = Math.floor((now / 1000 - (d.deleted_at ?? 0)) / 86400);
  return days <= 0 ? "today" : days === 1 ? "yesterday" : `${days} days ago`;
}

export const isRunning = (d: Download) =>
  d.status === "downloading" || d.status === "connecting";
export const canPause = (d: Download) => isRunning(d) || d.status === "queued";
export const canResume = (d: Download) => d.status === "paused" || d.status === "failed";

/* ---------- categories ---------- */

export type Category =
  | "video"
  | "audio"
  | "image"
  | "document"
  | "archive"
  | "software"
  | "other";

export const CATEGORIES: { id: Category; label: string }[] = [
  { id: "video", label: "Videos" },
  { id: "audio", label: "Music" },
  { id: "image", label: "Images" },
  { id: "document", label: "Documents" },
  { id: "archive", label: "Archives" },
  { id: "software", label: "Software" },
  { id: "other", label: "Other" },
];

const EXTENSIONS: Record<Exclude<Category, "other">, string[]> = {
  video: ["mp4", "mkv", "avi", "mov", "webm", "m4v", "flv", "wmv", "mpg", "mpeg", "3gp", "ts"],
  audio: ["mp3", "wav", "flac", "aac", "m4a", "ogg", "opus", "wma"],
  image: ["jpg", "jpeg", "png", "webp", "svg", "gif", "bmp", "tiff", "avif", "heic"],
  document: ["pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "txt", "csv", "epub", "md", "rtf"],
  archive: ["zip", "rar", "7z", "tar", "gz", "bz2", "xz", "zst", "iso"],
  software: ["exe", "msi", "msix", "apk", "dmg", "pkg", "deb", "rpm", "appimage"],
};

const BY_EXT = new Map<string, Category>();
for (const [cat, exts] of Object.entries(EXTENSIONS)) {
  for (const ext of exts) BY_EXT.set(ext, cat as Category);
}

export function categoryOf(d: Download): Category {
  if (d.torrent || d.profile) return "other";
  const dot = d.filename.lastIndexOf(".");
  const ext = dot >= 0 ? d.filename.slice(dot + 1).toLowerCase() : "";
  return BY_EXT.get(ext) ?? (d.media ? "video" : "other");
}

/* ---------- media quality ---------- */

export const QUALITIES: { value: string; label: string; options: MediaOptions }[] = [
  { value: "best", label: "Best quality", options: { max_height: null, audio_only: false } },
  ...[2160, 1440, 1080, 720, 480, 360].map((h) => ({
    value: String(h),
    label: h === 2160 ? "2160p (4K)" : `${h}p`,
    options: { max_height: h, audio_only: false },
  })),
  { value: "audio", label: "Audio only (MP3)", options: { max_height: null, audio_only: true } },
];

export function qualityOptions(value: string): MediaOptions {
  return (QUALITIES.find((q) => q.value === value) ?? QUALITIES[0]).options;
}

/** A .torrent file on this computer, given as a path (quotes allowed). */
export function isLocalTorrentPath(text: string) {
  return /^"?(?:[a-z]:[\\/]|\\\\|\/).+\.torrent"?$/i.test(text.trim());
}

/** Anything that should open the torrent dialog: magnet links, links to
 *  .torrent files, and .torrent files on this computer. */
export function isTorrentSource(text: string) {
  const t = text.trim();
  return /^magnet:\?\S+$/i.test(t) || /^https?:\/\/[^\s?#]+\.torrent(?:[?#]\S*)?$/i.test(t) || isLocalTorrentPath(t);
}

/** Anything the one link box accepts: web links, magnet links and .torrent files. */
export function looksLikeUrl(text: string) {
  return /^(https?:\/\/|magnet:\?)\S+$/i.test(text.trim()) || isLocalTorrentPath(text);
}

/** A web page that may hold a video, as opposed to a link to a file. */
export function looksLikePage(text: string) {
  const link = text.trim();
  if (!/^https?:\/\/\S+$/i.test(link) || isTorrentSource(link)) return false;
  try {
    const last = new URL(link).pathname.split("/").pop() ?? "";
    const ext = /\.([a-z0-9]{1,5})$/i.exec(last)?.[1].toLowerCase();
    return !ext || ["html", "htm", "php", "asp", "aspx", "jsp"].includes(ext);
  } catch {
    return false;
  }
}

/** "1:02:45" or "3:07". */
export function formatDuration(seconds: number): string {
  const h = Math.floor(seconds / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  const s = String(Math.floor(seconds % 60)).padStart(2, "0");
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${s}` : `${m}:${s}`;
}

/* ---------- small persisted preferences ---------- */

export function loadPref<T extends string>(key: string, fallback: T, allowed?: readonly T[]): T {
  try {
    const v = localStorage.getItem(key) as T | null;
    if (v != null && (!allowed || allowed.includes(v))) return v;
  } catch {
    /* storage unavailable */
  }
  return fallback;
}

export function savePref(key: string, value: string) {
  try {
    localStorage.setItem(key, value);
  } catch {
    /* storage unavailable */
  }
}
