import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { isPermissionGranted, requestPermission, sendNotification } from "@tauri-apps/plugin-notification";
import { dateText, isRtl, num, t } from "./i18n";

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
  /** While what is already on disk is being checked before the download
   *  carries on (a resumed torrent): bytes of `total` checked so far. */
  checking?: number | null;
  /** Files picked from a site link (a release, a shared folder), saved
   *  together in `path`; each is listed with how far it is. */
  bundle?: boolean;
  /** [start, position, end] per segment */
  segments: [number, number, number][];
  /** This download's own speed limit, bytes per second; 0 = none. */
  speed_limit: number;
  /** For a download inside a batch: the batch's id. It is shown in the
   *  batch's details, never as a row of its own. */
  parent?: number | null;
  /** For a batch (many links saved together in the folder `path`): how its
   *  downloads stand. Its status, sizes and speed are theirs together. */
  batch?: BatchInfo | null;
  /** A download of a batch cancelled on its own; its status is "failed". */
  cancelled?: boolean;
  /** When it ended, in seconds since 1970. */
  finished_at?: number | null;
  /** For a finished download: seconds it spent running (not queued or paused). */
  took?: number | null;
  /** Which connection it uses. */
  proxy?: ProxyChoice;
  /** Its file's checksum and how it compares, once one is wanted. */
  checksum?: Checksum | null;
  /** Its own request options (private values left out). */
  request?: RequestOptions | null;
  /** The saved request profile it uses. */
  request_profile?: number | null;
  /** For an FTP or SFTP download: how it signs in (no password). */
  remote?: RemoteOptions | null;
  /** An SFTP server key that was refused, to trust or not. */
  server_key?: ServerKey | null;
}

/** How an FTP or SFTP download signs in (crates/engine/src/remote.rs). */
export interface RemoteOptions {
  username: string;
  /** In only: the password, or the key file's passphrase. */
  password?: string | null;
  has_password?: boolean;
  key_file: string | null;
  active_mode: boolean;
  connect_timeout: number | null;
}

/** An SFTP server key that was refused. */
export interface ServerKey {
  host: string;
  port: number;
  fingerprint: string;
  key: string;
  /** It differs from the key trusted before. */
  changed: boolean;
}

export type AuthKind = "none" | "basic" | "digest" | "bearer";

/** A browser extension asking to connect (src-tauri/src/extension.rs). */
export interface PairRequest {
  id: string;
  origin: string;
  browser: string;
}

/** A connected browser extension. */
export interface PairedExtension {
  id: string;
  origin: string;
  browser: string;
  created_at: number;
  last_seen: number | null;
}

/** How a download's requests are made (crates/engine/src/request.rs).
 *  Private values go in (password, cookies, header values) and only flags
 *  saying they are kept come back. */
export interface RequestOptions {
  user_agent: string | null;
  referer: string | null;
  headers: [string, string][];
  auth: AuthKind;
  username: string;
  read_timeout: number | null;
  connect_timeout: number | null;
  password?: string | null;
  cookies?: string | null;
  has_password?: boolean;
  has_cookies?: boolean;
  private_headers?: string[];
}

export interface RequestProfile {
  id: number;
  name: string;
  options: RequestOptions;
}

export interface RequestBook {
  next_id: number;
  default: number | null;
  items: RequestProfile[];
}

export type HashAlgo = "md5" | "sha1" | "sha256" | "sha512";
export const HASH_ALGOS: readonly HashAlgo[] = ["sha256", "sha1", "md5", "sha512"];
export type CheckState = "not_verified" | "calculating" | "verified" | "failed" | "error";

/** A finished file's checksum (crates/engine/src/checksum.rs). */
export interface Checksum {
  algorithm: HashAlgo;
  expected: string | null;
  actual: string | null;
  state: CheckState;
  error: string | null;
  /** While it is worked out: bytes read so far. */
  done: number;
}

/** How a batch's downloads stand (crates/engine/src/batch.rs). */
export interface BatchInfo {
  count: number;
  completed: number;
  /** Connecting or downloading now. */
  downloading: number;
  queued: number;
  paused: number;
  /** Failed; cancelled ones are counted apart. */
  failed: number;
  cancelled: number;
  /** 0 to 1: each download counts the same; failed and cancelled ones are left out. */
  progress: number;
  /** A download running now, by its file name. */
  current: string | null;
}

/** A text file read for a batch download. */
export interface TextFile {
  name: string;
  text: string;
  bytes: number;
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
  /** For a file of a torrent that is not finished: how much of it is in;
   *  `size` is then its full size. */
  downloaded?: number | null;
  /** For a torrent's file on an engine of its own: its peers and speeds. */
  net?: FileNet | null;
}

/** How one file's own torrent engine is doing. */
export interface FileNet {
  /** Peers connected; of those, the ones with the whole torrent; peers heard of. */
  peers: number;
  seeders: number;
  found: number;
  /** Bytes per second in, and out to other peers. */
  down: number;
  up: number;
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
  /** This download's own speed limit, bytes per second; leave out for none. */
  speed_limit?: number;
  /** For a torrent: indexes of the files to download; leave out for all. */
  torrent_files?: number[];
  /** Which connection it uses; leave out for the default. */
  proxy?: ProxyChoice;
  /** The checksum the file should have, as pasted; checked once it is in. */
  checksum?: string;
  /** Its own request options, private values included. */
  request?: RequestOptions;
  /** A saved request profile to use instead; neither = the default profile. */
  request_profile?: number;
  /** For an FTP or SFTP link: how to sign in. */
  remote?: RemoteOptions;
}

/** How the app talks to a proxy server. */
export type ProxyKind = "http" | "https" | "socks4" | "socks4a" | "socks5" | "socks5h";
export const PROXY_KINDS: readonly ProxyKind[] = ["http", "https", "socks4", "socks4a", "socks5", "socks5h"];

/** A saved proxy. Its password is kept by the system, not here. */
export interface ProxyProfile {
  /** 0 for one not saved yet. */
  id: number;
  name: string;
  kind: ProxyKind;
  host: string;
  port: number;
  username: string;
  has_password: boolean;
  /** Sites reached directly, one rule each. */
  bypass: string[];
}

/** Which connection a download uses. */
export type ProxyChoice = "default" | "direct" | { profile: number };

/** The saved proxies and which one is the default (null: direct). */
export interface ProxyBook {
  next_id: number;
  default: number | null;
  items: ProxyProfile[];
}

/** A change to a proxy's saved password. */
export type PasswordChange = "keep" | "clear" | { set: string };

/** What testing a proxy found. */
export interface ProxyTest {
  ok: boolean;
  url: string;
  status: number | null;
  millis: number;
  error: string | null;
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

/** One file a site link holds (a release's asset, a file in a shared folder). */
export interface SiteFile {
  /** Folders, then the file name. */
  path: string[];
  url: string;
  size: number | null;
  headers: [string, string][];
  late: string | null;
  meta: string | null;
}

/** What a site link holds, to pick from. */
export interface SiteFiles {
  site: string;
  title: string;
  files: SiteFile[];
  /** More files than could be listed. */
  partial: boolean;
}

/** How the app handles a link of a site it knows. */
export interface SiteKind {
  site: string;
  /** "files": the user picks files from the list; "blocked": it cannot be downloaded. */
  support: "files" | "blocked";
  why: "store" | "paid" | "encrypted" | "search" | "site_only" | null;
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
  /** The largest picture's width and height, where known. */
  width?: number | null;
  height?: number | null;
  /** Who posted it, as the site shows their name. */
  uploader?: string | null;
  /** Their own page (a profile or channel), where known. */
  uploader_url?: string | null;
  /** The post's own words (its caption), where the site gives them. */
  caption?: string | null;
  /** How many have liked, commented on and watched it, where the site tells. */
  likes?: number | null;
  comments?: number | null;
  views?: number | null;
}

/** A link to Instagram (other than a profile, which Home reads as a
 *  profile first): read by the engine's own Instagram reader. */
export function isInstagramLink(text: string): boolean {
  try {
    const host = new URL(text.trim()).hostname.toLowerCase();
    return host === "instagram.com" || host.endsWith(".instagram.com") || host === "instagr.am";
  } catch {
    return false;
  }
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
  /** Download each file of a torrent of several files on its own, side by side. */
  torrent_files_apart: boolean;
  /** When the app opens, start a schedule whose start came while it was
   *  closed and whose stop time has not come yet. */
  start_scheduled_on_launch: boolean;
  /** What happens once every download has finished (src-tauri/src/after.rs). */
  after_all: AfterAll;
  /** Seconds of countdown, with Cancel, before it is done (5 to 600). */
  after_countdown: number;
  /** Done even when a download failed or was cancelled. */
  after_failures: boolean;
  /** Sorting finished files into folders by kind (crates/engine/src/organize.rs). */
  organize: Organize;
  /** Install yt-dlp and gallery-dl when missing and keep the app's own copies up to date. */
  auto_update_tools: boolean;
  /** Work out this kind of checksum for every finished file; null = only when one is given. */
  auto_checksum: HashAlgo | null;
}

/** A helper program the app can install and update (crates/engine/src/deps.rs). */
export interface ToolInfo {
  name: string;
  /** managed: installed by the app; system: installed by the user, left alone. */
  source: "managed" | "system" | "missing";
  path: string | null;
  version: string | null;
  /** The newest release seen at the last check. */
  latest: string | null;
  update_available: boolean;
  /** The project publishes a build for this computer. */
  can_install: boolean;
  installed_at: number | null;
  last_checked: number | null;
  /** The version kept to go back to. */
  previous: string | null;
  notes_url: string | null;
  busy: { phase: "checking" | "downloading" | "verifying" | "installing"; done: number; total: number | null } | null;
  error: string | null;
}

/** One kind of file and the folder it goes to. */
export interface FileCategory {
  name: string;
  /** A folder inside the download folder ("Videos"), or a full path. */
  folder: string;
  /** Endings, lower case, without the dot. */
  extensions: string[];
}

/** How finished downloads are sorted into folders by kind. */
export interface Organize {
  enabled: boolean;
  /** The first that lists a file's ending wins. */
  categories: FileCategory[];
  /** Where every other file goes; empty keeps it in the download folder. */
  other: string;
}

export type AfterAll = "nothing" | "notify" | "close" | "exit" | "sleep" | "hibernate" | "shutdown";

/** What a new link for a download would do (crates/engine/src/refresh.rs). */
export interface LinkCheck {
  /** It carries on from what is on disk. */
  resume: boolean;
  problem: "no_ranges" | "size_differs" | "version_differs" | "content_differs" | null;
  /** Bytes on disk, kept when it resumes, lost when it starts over. */
  downloaded: number;
  old_total: number | null;
  new_total: number | null;
}

/** A direct file download, the kind that can take a new link. */
export function takesNewLink(d: Download): boolean {
  // only a web link can be swapped for a fresh one (not FTP or SFTP)
  return !d.torrent && !d.media && !d.profile && !d.bundle && !d.batch && !remoteKind(d.url) && (d.status === "failed" || d.status === "paused");
}

/** What the backend says while it is about to act after all downloads finished. */
export interface AfterAllNotice {
  action: AfterAll;
  /** Seconds left; null once the countdown is over or stopped. */
  left: number | null;
  reason: "done" | "cancelled" | "busy" | null;
}

/* ---------- schedules (crates/engine/src/schedule.rs) ---------- */

/** When a schedule starts: once on a day ("2026-11-28"), or every week on
 *  the days set in `days` (bit 0 Monday ... bit 6 Sunday). */
export type Repeat = { kind: "once"; date: string } | { kind: "weekly"; days: number };

/** What the user sets for a schedule. Times are minutes after midnight, on
 *  this computer's clock; a stop at or before the start is on the next day. */
export interface ScheduleSpec {
  name: string;
  repeat: Repeat;
  start: number;
  stop: number | null;
  /** The downloads it starts, in order. */
  downloads: number[];
  /** Where its downloads' files go once finished, in place of their kind's folder. */
  folder?: string | null;
}

export type ScheduleStatus = "waiting" | "active" | "paused" | "done" | "missed";

export interface ScheduleEvent {
  kind: "started" | "stopped" | "missed";
  /** Seconds since 1970. */
  at: number;
}

export interface Schedule extends ScheduleSpec {
  id: number;
  /** False while the user has paused it. */
  enabled: boolean;
  created_at: number;
  status: ScheduleStatus;
  /** The next start, in seconds since 1970. */
  next_start: number | null;
  /** When the running window ends, or the next one would. */
  next_stop: number | null;
  last: ScheduleEvent | null;
}

/** How the app sits on the desktop (src-tauri/src/shell.rs). */
export interface AppPrefs {
  /** Start when the user signs in to the computer. */
  start_with_system: boolean;
  /** Closing the window leaves the app running in the tray. */
  close_to_tray: boolean;
  /** A system notification when a download finishes or fails. */
  notifications: boolean;
  /** Draw the window with the graphics card; read when the app starts. */
  hardware_acceleration: boolean;
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
  /** Its name on the site, without the "@". */
  username?: string | null;
  /** The links it lists: its website, its other accounts. */
  links?: string[];
  /** Likes its posts have had together, where the site shows that. */
  likes?: number | null;
  /** The counts are as the page writes them, rounded ("32K"). */
  rounded?: boolean;
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
  /** Whether a link is one of a site the app reads files from, or one it cannot download from. */
  siteKind: (url: string) => invoke<SiteKind | null>("site_kind", { url }),
  /** The files a site link holds (a release, a shared folder, a data set). */
  siteFiles: (url: string) => invoke<SiteFiles>("site_files", { url }),
  /** Downloads files picked from a site link, together in a folder named `title`. */
  addSiteFiles: (url: string, title: string, files: SiteFile[], dir?: string, speedLimit?: number) =>
    invoke<number>("add_site_files", { url, title, files, dir: dir ?? null, speedLimit: speedLimit || null }),
  /** What a new link for a download would do; nothing is changed. */
  checkNewLink: (id: number, url: string, headers?: [string, string][]) =>
    invoke<LinkCheck>("check_new_link", { id, url, headers: headers ?? null }),
  /** Gives a download a new link: it carries on only for the same file, or starts over with `restart`. */
  useNewLink: (id: number, url: string, headers: [string, string][] | undefined, restart: boolean) =>
    invoke<LinkCheck>("use_new_link", { id, url, headers: headers ?? null, restart }),
  /** A download's own speed limit, bytes per second; 0 = none. Takes effect while it runs. */
  setSpeedLimit: (id: number, rate: number) => invoke<void>("set_download_speed_limit", { id, rate }),
  /** The saved proxies and the default connection. */
  proxies: () => invoke<ProxyBook>("list_proxies"),
  /** Saves a new proxy (id 0) or changes one; answers with the list as it now is. */
  saveProxy: (profile: ProxyProfile, password: PasswordChange) => invoke<ProxyBook>("save_proxy", { profile, password }),
  /** Refused while an unfinished download uses it. */
  deleteProxy: (id: number) => invoke<ProxyBook>("delete_proxy", { id }),
  /** null: downloads on the default connection connect directly. */
  setDefaultProxy: (id: number | null) => invoke<ProxyBook>("set_default_proxy", { id }),
  /** Tries a proxy as filled in, saved or not; "keep" uses its saved password. */
  testProxy: (profile: ProxyProfile, password: PasswordChange, url?: string) =>
    invoke<ProxyTest>("test_proxy", { profile, password, url: url ?? null }),
  /** Trusts the SFTP server key a download was refused for, and tries it again. */
  trustServerKey: (id: number) => invoke<void>("trust_server_key", { id }),
  /** Picks any one file; null when cancelled. */
  pickAnyFile: async (title: string): Promise<string | null> => {
    const picked = await openDialog({ multiple: false, directory: false, title });
    return typeof picked === "string" ? picked : null;
  },
  /** Checks a finished file against a pasted checksum, or (none) works its own out to see. */
  verifyChecksum: (id: number, expected: string | null, algorithm: HashAlgo | null) =>
    invoke<void>("verify_checksum", { id, expected, algorithm }),
  /** Checks a batch's files (or one file) against a sums file's text; answers how many it named. */
  verifyWithSums: (id: number, sums: string) => invoke<number>("verify_with_sums", { id, sums }),
  /** The saved request profiles and the default. */
  requestProfiles: () => invoke<RequestBook>("list_request_profiles"),
  /** Saves a new profile (id 0) or changes one; "keep" leaves a saved secret as it is. */
  saveRequestProfile: (id: number, name: string, options: RequestOptions, password: PasswordChange, cookies: PasswordChange) =>
    invoke<RequestBook>("save_request_profile", { id, name, options, password, cookies }),
  deleteRequestProfile: (id: number) => invoke<RequestBook>("delete_request_profile", { id }),
  setDefaultRequestProfile: (id: number | null) => invoke<RequestBook>("set_default_request_profile", { id }),
  /** Changes a download's connection; a running one restarts with it and carries on. */
  setDownloadProxy: (id: number, choice: ProxyChoice) => invoke<void>("set_download_proxy", { id, choice }),
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
  /** Opens a web page (a link a profile lists) in the user's browser. */
  openWebLink: (url: string) => invoke<void>("open_web_link", { url }),
  /** The same with its numbers (posts, followers, joined), asked as a visitor who is not signed in. */
  profileStats: (url: string) => invoke<ProfileFace>("profile_stats", { url }),
  /** Calls back with true while files are dragged over the window, false once they leave or drop. */
  onFileHover: (cb: (over: boolean) => void) =>
    getCurrentWebview().onDragDropEvent((e) => cb(e.payload.type === "enter" || e.payload.type === "over")),
  /** Calls back with the paths of files dropped onto the window. */
  onFileDrop: (cb: (paths: string[]) => void) =>
    getCurrentWebview().onDragDropEvent((e) => {
      if (e.payload.type === "drop") cb(e.payload.paths);
    }),
  /** Moves a finished download into another folder; resolves with where it is now. */
  moveDownload: (id: number, dir: string) => invoke<string>("move_download", { id, dir }),
  /** Downloads many links as one batch, saved in a folder named `name` inside `dir`. */
  addBatch: (name: string, urls: string[], media: MediaOptions, dir?: string, speedLimit?: number) =>
    invoke<number>("add_batch", { name, urls, media, dir: dir ?? null, speedLimit: speedLimit || null }),
  /** Puts a batch's failed and cancelled downloads back in the queue. */
  retryFailed: (id: number) => invoke<void>("retry_failed_downloads", { id }),
  /** Cancels one download of a batch; it stays listed and can be retried. */
  cancelItem: (id: number) => invoke<void>("cancel_download", { id }),
  /** Picks text files holding links; empty when cancelled. */
  pickTextFiles: async (): Promise<string[]> => {
    const picked = await openDialog({
      multiple: true,
      filters: [
        {
          name: t.batch.fileFilter,
          extensions: ["txt", "json", "jsonl", "csv", "tsv", "xml", "html", "htm", "md", "log", "m3u", "m3u8", "xspf", "yaml", "yml", "ini", "rtf", "list", "url"],
        },
        { name: t.batch.allFiles, extensions: ["*"] },
      ],
    });
    return Array.isArray(picked) ? picked : typeof picked === "string" ? [picked] : [];
  },
  /** Reads a file as text; fails for a file that is not text. */
  readTextFile: (path: string) => invoke<TextFile>("read_text_file", { path }),
  /** Asks where to save, then saves the text there; resolves with the path, or null when cancelled. */
  saveTextFile: async (defaultName: string, contents: string, kind: "txt" | "csv"): Promise<string | null> => {
    const path = await saveDialog({
      defaultPath: defaultName,
      filters: [kind === "csv" ? { name: "CSV", extensions: ["csv"] } : { name: "Text", extensions: ["txt"] }],
    });
    if (!path) return null;
    await invoke<void>("write_text_file", { path, contents });
    return path;
  },
  pause: (id: number) => invoke<void>("pause_download", { id }),
  resume: (id: number) => invoke<void>("resume_download", { id }),
  remove: (id: number, deleteFile: boolean) =>
    invoke<void>("remove_download", { id, deleteFile }),
  open: (id: number) => invoke<void>("open_file", { id }),
  reveal: (id: number) => invoke<void>("reveal_file", { id }),
  /** Opens one finished file of a profile, playlist or torrent, by its place in the list. */
  openItem: (id: number, index: number) => invoke<void>("open_item", { id, index }),
  /** Shows one finished file of a profile, playlist or torrent in its folder. */
  revealItem: (id: number, index: number) => invoke<void>("reveal_item", { id, index }),
  /** Shows where a download is: its file once finished, or else its folder. */
  showFolder: (id: number) => invoke<void>("show_download_folder", { id }),
  openDownloadDir: () => invoke<void>("open_download_dir"),
  tools: () => invoke<ToolStatus>("tool_status"),
  /** yt-dlp and gallery-dl: which copy is used, its version and the newest release. */
  toolsInfo: () => invoke<ToolInfo[]>("tools_info"),
  /** Looks for new releases now. */
  checkTools: () => invoke<ToolInfo[]>("check_tools"),
  /** Installs or updates one tool, checked against its published sums; answers when done. */
  installTool: (name: string) => invoke<ToolInfo>("install_tool", { name }),
  /** Goes back to the version used before the last update. */
  rollbackTool: (name: string) => invoke<ToolInfo>("rollback_tool", { name }),
  /** Opens a tool's release notes on its project's own site. */
  openReleaseNotes: (name: string) => invoke<void>("open_release_notes", { name }),
  onTools: (cb: (tools: ToolInfo[]) => void) => listen<ToolInfo[]>("tools", (e) => cb(e.payload)),
  getSettings: () => invoke<Settings>("get_settings"),
  schedules: () => invoke<Schedule[]>("list_schedules"),
  /** Makes a schedule (`id` null) or changes one; resolves with its id. */
  saveSchedule: (id: number | null, spec: ScheduleSpec) => invoke<number>("save_schedule", { id, spec }),
  /** Pauses a schedule, or lets it run again. */
  setScheduleEnabled: (id: number, enabled: boolean) => invoke<void>("set_schedule_enabled", { id, enabled }),
  deleteSchedule: (id: number) => invoke<void>("delete_schedule", { id }),
  /** Stops the countdown before the action after all downloads. */
  afterAllCancel: () => invoke<void>("after_all_cancel"),
  /** Does that action now, without waiting for the countdown. */
  afterAllNow: () => invoke<void>("after_all_now"),
  onAfterAll: (cb: (notice: AfterAllNotice) => void) => listen<AfterAllNotice>("after-all", (e) => cb(e.payload)),
  onAfterAllFailed: (cb: (why: string) => void) => listen<string>("after-all-failed", (e) => cb(e.payload)),
  onSchedules: (cb: (schedules: Schedule[]) => void) =>
    listen<Schedule[]>("schedules", (e) => cb(e.payload)),
  getAppPrefs: () => invoke<AppPrefs>("get_app_prefs"),
  setAppPrefs: (prefs: AppPrefs) => invoke<void>("set_app_prefs", { prefs }),
  /** The tray menu's two lines, in the interface's language. */
  setTrayText: (show: string, quit: string) => invoke<void>("set_tray_text", { show, quit }),
  /** A system notification; asks for permission the first time. */
  notify: async (title: string, body: string): Promise<void> => {
    let allowed = await isPermissionGranted();
    if (!allowed) allowed = (await requestPermission()) === "granted";
    if (allowed) sendNotification({ title, body });
  },
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
  /** Calls back with a page link sent from the browser extension, for the app to look up. */
  onExtensionLink: (cb: (link: string) => void) => listen<string>("extension-link", (e) => cb(e.payload)),
  /** Calls back when a page on the Browser page starts a file download, which the app takes over. */
  onBrowserDownload: (cb: (url: string) => void) => listen<string>("browser-download", (e) => cb(e.payload)),
  onBrowserDownloadFailed: (cb: (why: string) => void) => listen<string>("browser-download-failed", (e) => cb(e.payload)),
  /** Calls back with how many downloads the browser extension just started. */
  onExtensionAdded: (cb: (count: number) => void) => listen<number>("extension-added", (e) => cb(e.payload)),
  /** A browser extension asks to connect; the user allows it or not. */
  onExtensionPair: (cb: (ask: PairRequest) => void) => listen<PairRequest>("extension-pair", (e) => cb(e.payload)),
  answerExtensionPair: (id: string, allow: boolean) => invoke<PairedExtension[]>("answer_extension_pair", { id, allow }),
  pairedExtensions: () => invoke<PairedExtension[]>("paired_extensions"),
  revokeExtension: (id: string) => invoke<PairedExtension[]>("revoke_extension", { id }),
  /** Picks a cookies file exported from a browser; null when cancelled. */
  pickCookiesFile: async (): Promise<string | null> => {
    const picked = await openDialog({
      multiple: false,
      filters: [{ name: t.common.fileFilters.cookies, extensions: ["txt"] }],
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
        { name: t.common.fileFilters.linkLists, extensions: ["torrent", "txt", "csv", "list", "md", "html"] },
        { name: t.common.fileFilters.torrents, extensions: ["torrent"] },
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

export function formatBytes(n: number): string {
  const units = t.common.units;
  let i = 0;
  while (n >= 1024 && i < units.length - 1) {
    n /= 1024;
    i++;
  }
  return isolate(`${i === 0 ? num(Math.round(n)) : num(n, n >= 100 ? 0 : 1)} ${units[i]}`);
}

export function formatSpeed(n: number): string {
  return isolate(t.common.perSecond(formatBytes(n)));
}

/** In right-to-left text, keeps a size such as "12 MB" in one piece, read
 *  left to right, instead of letting its number and unit trade places. */
const FSI = String.fromCharCode(0x2068); // first strong isolate
const PDI = String.fromCharCode(0x2069); // pop directional isolate

function isolate(text: string): string {
  return isRtl() ? FSI + text + PDI : text;
}

export function formatEta(seconds: number): string {
  if (!isFinite(seconds) || seconds < 0) return "";
  // Years, days, hours, minutes and seconds, starting at the largest one
  // there is: "45s left", "3h 0m 12s left", "1y 20d 4h 5m 6s left".
  let rest = Math.ceil(seconds);
  const parts: string[] = [];
  for (const [unit, size] of ETA_UNITS) {
    const count = Math.floor(rest / size);
    rest -= count * size;
    if (count > 0 || parts.length > 0 || unit === "s") parts.push(t.common.eta[unit](count));
  }
  return t.common.eta.left(parts.join(t.common.eta.join));
}

/** How long a download took: "45s", "3m 12s", "1h 0m 5s". */
export function formatTook(seconds: number): string {
  let rest = Math.max(0, Math.round(seconds));
  const parts: string[] = [];
  for (const [unit, size] of ETA_UNITS) {
    const count = Math.floor(rest / size);
    rest -= count * size;
    if (count > 0 || parts.length > 0 || unit === "s") parts.push(t.common.eta[unit](count));
  }
  return parts.join(t.common.eta.join);
}

const ETA_UNITS: ["y" | "d" | "h" | "m" | "s", number][] = [
  ["y", 365 * 86400],
  ["d", 86400],
  ["h", 3600],
  ["m", 60],
  ["s", 1],
];

export function formatDate(unixSeconds: number): string {
  if (!unixSeconds) return t.common.unknownDate;
  return dateText(new Date(unixSeconds * 1000), {
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
  // A batch: by its bytes while every file's size is known, else file by file.
  if (d.batch && !d.total) return Math.min(100, Math.floor(d.batch.progress * 100));
  if (!d.total) return null;
  return Math.min(100, Math.floor((d.downloaded / d.total) * 100));
}

/** How far the check of what is already downloaded has got, 0 to 100, or
 *  null when nothing is being checked. */
export function checkPercent(d: Download): number | null {
  if (d.checking == null) return null;
  return d.total ? Math.min(100, Math.floor((d.checking / d.total) * 100)) : 0;
}

/** "Checking downloaded data · 42%" while that happens, else null. */
export function checkingText(d: Download): string | null {
  const pct = checkPercent(d);
  return pct == null ? null : t.common.checking(pct);
}

export function etaOf(d: Download): string {
  if (d.status !== "downloading" || !d.total || d.speed <= 0) return "";
  return formatEta((d.total - d.downloaded) / d.speed);
}

/* ---------- status ---------- */

/** Each status in the language switched on now (read when used). */
export const STATUS_LABEL: Readonly<Record<Status, string>> = {
  get queued() {
    return t.common.status.queued;
  },
  get connecting() {
    return t.common.status.connecting;
  },
  get downloading() {
    return t.common.status.downloading;
  },
  get paused() {
    return t.common.status.paused;
  },
  get completed() {
    return t.common.status.completed;
  },
  get failed() {
    return t.common.status.failed;
  },
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
  const ago = t.common.deletedAgo;
  return days <= 0 ? ago.today : days === 1 ? ago.yesterday : ago.days(days);
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

/** The categories in order; each `label` is read in the language switched on now. */
export const CATEGORIES: readonly { readonly id: Category; readonly label: string }[] = (
  ["video", "audio", "image", "document", "archive", "software", "other"] as const
).map((id) => ({
  id,
  get label() {
    return t.common.category[id];
  },
}));

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
  if (d.torrent || d.profile || d.bundle || d.batch) return "other";
  const dot = d.filename.lastIndexOf(".");
  const ext = dot >= 0 ? d.filename.slice(dot + 1).toLowerCase() : "";
  return BY_EXT.get(ext) ?? (d.media ? "video" : "other");
}

/* ---------- media quality ---------- */

/** The qualities on offer; each `label` is read in the language switched on now. */
export const QUALITIES: readonly { readonly value: string; readonly label: string; readonly options: MediaOptions }[] = [
  {
    value: "best",
    get label() {
      return t.common.quality.best;
    },
    options: { max_height: null, audio_only: false },
  },
  ...[2160, 1440, 1080, 720, 480, 360].map((h) => ({
    value: String(h),
    get label() {
      return h === 2160 ? t.common.quality.fourK(String(h)) : t.common.quality.height(String(h));
    },
    options: { max_height: h, audio_only: false },
  })),
  {
    value: "audio",
    get label() {
      return t.common.quality.audio;
    },
    options: { max_height: null, audio_only: true },
  },
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
  return /^(https?:\/\/|s?ftps?:\/\/|ftpes:\/\/|magnet:\?)\S+$/i.test(text.trim()) || isLocalTorrentPath(text);
}

/** An FTP, FTPS or SFTP link, which signs in its own way. */
export function remoteKind(text: string): "ftp" | "sftp" | null {
  const scheme = /^([a-z]+):\/\//i.exec(text.trim())?.[1].toLowerCase();
  return scheme === "sftp" ? "sftp" : scheme === "ftp" || scheme === "ftps" || scheme === "ftpes" ? "ftp" : null;
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
