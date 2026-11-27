/* English text of app: src/App.tsx (title bar, sidebar, toasts, questions, command palette). */
import { pct, plural } from "../format";

export const app = {
  /** Sidebar. */
  nav: {
    label: "Main",
    home: "Home",
    downloads: "Downloads",
    history: "History",
    browser: "Browser",
    signin: "Sign in",
    settings: "Settings",
    darkMode: "Dark Mode",
    about: "About",
  },
  /** Window buttons in the title bar. */
  /** The tray icon's menu. */
  tray: {
    show: "Show RushDownload",
    quit: "Quit",
  },
  /** Titles of the system notifications; the text is the file's name. */
  notify: {
    done: "Download finished",
    failed: "Download failed",
  },
  window: {
    minimize: "Minimize",
    maximize: "Maximize",
    close: "Close",
  },
  /** Asked before a second copy of a link already in the list starts. */
  duplicate: {
    status: {
      completed: "Completed",
      failed: "Failed",
      paused: "Paused",
      pausedAt: (percent: number) => `Paused, ${pct(percent)}`,
      waiting: "Waiting to start",
      downloading: "Downloading",
      downloadingAt: (percent: number) => `Downloading, ${pct(percent)}`,
    },
    title: {
      doneFile: "You already downloaded this file",
      doneTorrent: "You already downloaded this torrent",
      failedFile: "This file is already in your downloads",
      failedTorrent: "This torrent is already in your downloads",
      runningFile: "This file is already downloading",
      runningTorrent: "This torrent is already downloading",
    },
    detailFile: "Downloading a copy saves a second, separate file. The first one stays as it is.",
    detailTorrent:
      "Downloading a copy saves it again in a second, separate folder. The first one stays as it is.",
    ok: "Download a copy",
    show: "Show it",
    cancel: "Cancel",
  },
  /** Asked before unfinished downloads are removed. */
  cancelDownload: {
    titleOne: "Cancel this download?",
    titleMany: "Cancel these downloads?",
    unfinished: (n: number) => plural(n, { one: "# unfinished download", other: "# unfinished downloads" }),
    detailProfile: "Photos and videos already downloaded stay in the folder.",
    detailFile: "The part already downloaded will be deleted.",
    ok: "Cancel download",
    cancel: "Keep downloading",
  },
  /** Asked before a file is deleted from disk. */
  deleteFile: {
    title: "Delete this file from disk?",
    detail: "This cannot be undone.",
    ok: "Delete file",
    cancel: "Keep file",
  },
  toast: {
    /** `what` is the name of the thing copied, e.g. "Link". */
    copied: (what: string) => `${what} copied`,
    copyFailed: (what: string) => `Could not copy the ${what.toLowerCase()}`,
    pickOnPage: "This video has to be picked on its page. Choose a quality there",
    clipboardLink: "Copied link is ready on Home",
    extensionLink: "Link from your browser is ready on Home",
    extensionAdded: (n: number) =>
      plural(n, { one: "Download started from your browser", other: "# downloads started from your browser" }),
    onlyTorrents: "Only .torrent files can be dropped here",
    browserDownload: `The page's download is now in your downloads`,
  },
  /** Right-click menu for several selected downloads. */
  menu: {
    pause: (n: number) => plural(n, { one: "Pause # download", other: "Pause # downloads" }),
    resume: (n: number) => plural(n, { one: "Resume # download", other: "Resume # downloads" }),
    remove: (n: number) => plural(n, { one: "Remove # from list", other: "Remove # from list" }),
  },
  /** Command palette (Ctrl K). */
  commands: {
    groupActions: "Actions",
    groupGoTo: "Go to",
    add: "Add download",
    scan: "Find media on a web page",
    pauseAll: "Pause all downloads",
    resumeAll: "Resume all downloads",
    folder: "Open download folder",
    lightTheme: "Switch to light theme",
    darkTheme: "Switch to dark theme",
    home: "Home",
    downloads: "Downloads",
    settings: "Settings",
    about: "About RushDownload",
  },
};
