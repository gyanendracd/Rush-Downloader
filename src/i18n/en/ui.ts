/* English text of ui (src/ui.tsx: shared components). See src/i18n/README for how to add keys. */
import { plural } from "../format";

export const ui = {
  /** Status word while what is on disk is being checked. */
  checking: "Checking",
  connect: {
    connecting: "Connecting",
    failed: "Connection failed, trying again",
    stillFailing: "Still failing to connect, trying again",
  },
  checkBar: "Checking downloaded data",
  progress: {
    /** Label of a download's progress bar. */
    label: (name: string) => `${name} progress`,
    /** Progress drawn as one bar for each connection. */
    parts: (name: string, n: number) =>
      plural(n, { one: `${name} progress, # part`, other: `${name} progress, # parts` }),
  },
  files: (n: number) => plural(n, { one: "# file", other: "# files" }),
  /** "3 files · 72.5 MB" for a profile being saved. */
  profileAmount: (files: string, size: string) => `${files} · ${size}`,
  meta: {
    queuedSize: (size: string) => `Queued · ${size}`,
    queuedWaiting: "Queued · waiting for a free slot",
    paused: (amount: string) => `Paused · ${amount}`,
    deleted: "Deleted",
    /** Size of a file that has since been deleted. */
    was: (size: string) => `was ${size}`,
    unknownError: "Unknown error",
  },
  spark: {
    empty: "Speed appears here while the download is running.",
    now: "now",
    secondsAgo: (n: number) => plural(n, { one: "about #s ago", other: "about #s ago" }),
  },
  menu: {
    pause: "Pause",
    resume: "Resume",
    retry: "Retry",
    openOnBrowser: "Open on Browser page",
    findMedia: "Find media on this page",
    downloadAgain: "Download again",
    openFile: "Open file",
    showInFolder: "Show in folder",
    copyLink: "Copy link",
    copyPath: "Copy file path",
    removeFromHistory: "Remove from history",
    removeFromList: "Remove from list",
    cancelAndRemove: "Cancel and remove",
    deleteFile: "Delete file from disk",
  },
  /** What was copied, for the "… copied" message. */
  copyWhat: {
    link: "Link",
    path: "File path",
  },
  row: {
    deleted: "Deleted",
    deletedAgain: "Deleted. Download again",
    openFile: "Open file",
    completedOpen: "Completed. Open file",
    retry: "Retry",
    failedRetry: "Failed. Retry",
    resume: "Resume",
    pause: "Pause",
    more: "More actions",
    /** `time`: how long it took ("3m 12s"). */
    took: (time: string) => `Downloaded in ${time}`,
  },
  dismiss: "Dismiss",
  close: "Close",
  /** One download's own speed limit. Speeds ("2 MB/s") are written by the app. */
  limit: {
    menu: "Speed limit…",
    title: "Speed limit",
    enable: "Limit this download's speed",
    amount: "Maximum speed",
    unit: "Unit",
    hint: "The global limit in Settings applies too; the lower of the two wins.",
    none: "No limit",
    change: "Change",
    save: "Save",
    /** `speed`: the limit, written out ("2 MB/s"). */
    set: (speed: string) => `Limited to ${speed}`,
    cleared: "Speed limit removed",
    torrentNote: "A torrent slows to the new limit over about 15 seconds, as blocks already asked of its peers arrive.",
    programNote: "It restarts to take the new limit, and carries on from where it was.",
  },
  /** A new link for a download whose link stopped working. Sizes ("1.2 GB") are written by the app. */
  refresh: {
    menu: "Refresh link…",
    title: "Refresh link",
    link: "New link",
    /** `url`: the link it has now. */
    old: (url: string) => `Now: ${url}`,
    headers: "Request headers",
    headersHint: "Optional. One per line, as Name: value, for a link that needs a sign-in token. Left empty, the ones it had are kept.",
    check: "Check link",
    checking: "Checking…",
    /** `kept`: how much is downloaded already. */
    same: (kept: string) => `It is the same file. The download carries on from ${kept}.`,
    fresh: "Nothing is downloaded yet: it starts with this link.",
    use: "Use this link",
    problems: {
      no_ranges: "This server sends the file only whole, so the download cannot carry on from where it was.",
      size_differs: "The file at this link is a different size.",
      version_differs: "The file at this link is a different version: its ETag or date differs.",
      content_differs: "Parts of the file at this link do not match what is already downloaded.",
    },
    /** `lost`: how much is downloaded already. */
    restartNote: (lost: string) => `Starting over deletes the ${lost} downloaded so far.`,
    restart: "Start over with this link",
    done: "Link updated",
    /** `line`: the line that is not a header. */
    badHeader: (line: string) => `Not a header (Name: value): ${line}`,
  },
};
