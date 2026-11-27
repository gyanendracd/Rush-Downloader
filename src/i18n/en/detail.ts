/* English text of detail. See src/i18n/README for how to add keys. */
import { num, plural } from "../format";

export const detail = {
  /** Dialog title, read by screen readers. */
  title: (name: string) => `Details for ${name}`,
  tabs: {
    progress: "Progress",
    details: "Details",
    options: "Options",
  },
  /** The line under the file name. */
  headline: {
    completed: (size: string) => `Completed · ${size}`,
    running: (amount: string, speed: string) => `${amount} · ${speed}`,
    /** "Checking downloaded data · 40% · 72.5 MB / 1.2 GB before" */
    checking: (checking: string, amount: string) => `${checking} · ${amount} before`,
    status: (status: string, amount: string) => `${status} · ${amount}`,
  },
  buttons: {
    open: "Open",
    showInFolder: "Show in folder",
    remove: "Remove",
    pause: "Pause",
    resume: "Resume",
    retry: "Retry",
    cancel: "Cancel",
  },
  progress: {
    whyFailed: "Why it failed",
    speedChart: "Download speed over the last minute",
    saveLocation: "Save Location",
    fileSize: "File Size",
    downloaded: "Downloaded",
    timeRemaining: "Time Remaining",
    unknown: "Unknown",
    done: "Done",
  },
  details: {
    link: "Link",
    savedAs: "Saved As",
    added: "Added",
    source: "Source",
    sourceProfile: "Profile or playlist",
    sourceTorrent: "Torrent",
    sourceSite: `Files from a site link`,
    sourceMedia: "Video page",
    sourceDirect: "Direct file link",
    pauseResume: "Pause and Resume",
    supported: "Supported",
    notKnownYet: "Not known yet",
    notSupported: "Not supported by this server",
    connections: "Connections",
    connectionsTorrent: "Shared between peers by the torrent engine",
    connectionsMedia: "Handled by the site backend",
    /** "3 of 8" connections in use. */
    connectionsActive: (active: number, all: number) => `${num(active)} of ${num(all)}`,
    connectionsUpTo: (n: number) => `Up to ${num(n)}`,
    segments: "Progress of each connection",
  },
  options: {
    openFile: "Open file",
    showInFolder: "Show in folder",
    copyFilePath: "Copy file path",
    /** What was copied, as named in the "copied" message. */
    filePath: "File path",
    copyLink: "Copy link",
    link: "Link",
    findMedia: "Find media on this page",
    deleteFile: "Delete file from disk",
  },
  /** The file a profile or playlist download is fetching now. */
  now: {
    label: "File downloading now",
    heading: "Downloading now",
    /** "Downloading now · 3 of 40" */
    headingAt: (place: string) => `Downloading now · ${place}`,
    /** Number in the list, when the list's length is not known. */
    number: (index: number) => `No. ${num(index)}`,
    amount: (done: string, total: string) => `${done} / ${total}`,
    progressOf: (name: string) => `${name} progress`,
  },
  /** "3 of 40": a place in a list. */
  placeOf: (index: number, count: number) => `${num(index)} of ${num(count)}`,
  /** The items of a profile or playlist download. */
  items: {
    label: "What this download holds",
    gettingList: "Getting the list",
    /** A torrent file's own engine: "12 of 40 peers". */
    peersOf: (live: number, found: number) =>
      plural(found, { one: `${num(live)} of # peer`, other: `${num(live)} of # peers` }),
    seeders: (n: number) => plural(n, { one: "# seeder", other: "# seeders" }),
    count: (n: number) => plural(n, { one: "# item", other: "# items" }),
    done: (n: number) => `${num(n)} done`,
    doneMark: "Done",
    more: (n: number) => `and ${num(n)} more`,
    state: {
      done: "Downloaded",
      downloading: "Downloading",
      waiting: "Waiting",
      missed: "Not available",
      deleted: "Deleted",
    },
  },
  /** The card shown beside a pointed-at item. */
  card: {
    status: "Status",
    position: "Position",
    length: "Length",
    channel: "Channel",
    downloaded: "Downloaded",
    size: "Size",
    file: "File",
    link: "Link",
    /** "Downloading · 42%" */
    stateWithPct: (state: string, percent: string) => `${state} · ${percent}`,
  },
};
