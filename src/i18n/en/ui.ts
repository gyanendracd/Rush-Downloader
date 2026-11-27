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
  },
  dismiss: "Dismiss",
  close: "Close",
};
