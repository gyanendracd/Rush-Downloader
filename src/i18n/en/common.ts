/* English text shared across the interface: statuses, categories, sizes, times.
 * Used by src/api.ts and anywhere else. */
import { num, pct, plural } from "../format";

export const common = {
  status: {
    queued: "Queued",
    connecting: "Connecting",
    downloading: "Downloading",
    paused: "Paused",
    completed: "Completed",
    failed: "Failed",
  },
  category: {
    video: "Videos",
    audio: "Music",
    image: "Images",
    document: "Documents",
    archive: "Archives",
    software: "Software",
    other: "Other",
  },
  quality: {
    best: "Best quality",
    /** 2160 */
    fourK: (h: string) => `${h}p (4K)`,
    height: (h: string) => `${h}p`,
    audio: "Audio only (MP3)",
  },
  /** Byte units, smallest first. */
  units: ["B", "KB", "MB", "GB", "TB"],
  /** "12 MB/s" */
  perSecond: (size: string) => `${size}/s`,
  /** Parts of a time left: "3h 0m 12s left". */
  eta: {
    y: (n: number) => `${num(n)}y`,
    d: (n: number) => `${num(n)}d`,
    h: (n: number) => `${num(n)}h`,
    m: (n: number) => `${num(n)}m`,
    s: (n: number) => `${num(n)}s`,
    /** Parts are joined with this. */
    join: " ",
    left: (time: string) => `${time} left`,
  },
  unknownDate: "Unknown",
  checking: (percent: number) => `Checking downloaded data · ${pct(percent)}`,
  deletedAgo: {
    today: "today",
    yesterday: "yesterday",
    days: (n: number) => plural(n, { one: "# day ago", other: "# days ago" }),
  },
  fileFilters: {
    cookies: "Cookies file",
    linkLists: "Torrents and link lists",
    torrents: "Torrent files",
  },
  /** Words on the file-type badge of a file icon (src/DocIcon.tsx) when the
   *  name has no extension to show. Keep them very short: 5 letters at most. */
  badge: {
    video: "VIDEO",
    audio: "AUDIO",
    image: "IMAGE",
    document: "DOC",
    archive: "ZIP",
    software: "APP",
    other: "FILE",
    /** A whole profile. */
    profile: "ALL",
    /** A torrent. */
    torrent: "P2P",
  },
  /** Language picker. */
  language: "Language",
  changeLanguage: "Change language",
  languageHint: "The language of the app's menus and messages.",
};
