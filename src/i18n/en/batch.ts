/* English text of Batch Download: the button on Home, its dialog
 * (src/Batch.tsx), and a batch's row and details in Downloads.
 * Other languages show this English text until they translate it. */
import { num, plural } from "../format";

export const batch = {
  button: "Batch Download",
  buttonHint: "Extract and download multiple URLs together",
  command: "Batch download",

  title: "Batch Download",
  steps: {
    input: "Add links",
    results: "Review links (optional)",
  },

  /* step 1: paste or add files */
  pasteLabel: "Paste text with links",
  pastePlaceholder:
    "Paste anything that holds links: a list, a chat message, JSON, HTML, a page's source…\nEvery http and https link is found, wherever it is in the text.",
  liveCount: (n: number) => (n === 0 ? "No links in this text yet" : plural(n, { one: "# link in this text", other: "# links in this text" })),
  filesLabel: "Or add files",
  drop: "Drop text files here",
  dropOr: "or",
  browse: "Browse files",
  fileFilter: "Text files",
  allFiles: "All files",
  dropHint: "TXT, CSV, JSON, HTML, XML, Markdown, M3U, logs and any other text file",
  dropNow: "Drop to add",
  reading: "Reading…",
  removeFile: (name: string) => `Remove ${name}`,
  fileRejected: "Not a text file",
  extract: "Extract links",
  downloadAll: "Download all",
  extracting: (done: number, total: number) => `Finding links… ${num(done)} of ${num(total)}`,
  stop: "Stop",
  nothingToExtract: "Paste some text or add a file first",
  noneFound: "No web links were found. Links have to start with http:// or https://.",

  /* step 2: the links found */
  complete: "URL Extraction Complete",
  stats: {
    files: "Files processed",
    discovered: "URLs discovered",
    unique: "Unique URLs",
    duplicates: "Duplicates removed",
    invalid: "Invalid URLs",
  },
  skipped: (names: string) => `Skipped (not text): ${names}`,
  search: "Search links",
  searchEmpty: "No link matches the search",
  selectAll: "Select all",
  deselectAll: "Deselect all",
  copySelected: "Copy selected",
  removeSelected: "Remove selected",
  clear: "Clear list",
  copyOne: "Copy link",
  removeOne: "Remove from list",
  selectOne: (url: string) => `Select ${url}`,
  pasted: "Pasted",
  shownOf: (shown: number, all: number) => `Showing ${num(shown)} of ${num(all)}`,
  showMore: (n: number) => `Show ${num(n)} more`,
  exportTxt: "Download Extracted URLs",
  exportCsv: "CSV",
  exportCsvTitle: "Download the list as CSV (Index, URL, Source File)",
  exportScope: (selected: number, all: number) =>
    selected > 0 ? `${num(selected)} selected` : `All ${num(all)}`,
  exported: (n: number, file: string) => `Saved ${plural(n, { one: "# link", other: "# links" })} to ${file}`,
  copied: (n: number) => `${plural(n, { one: "# link", other: "# links" })}`,
  emptyList: "The list is empty. Go back to add links.",
  back: "Back",
  next: (selected: number, all: number) =>
    selected > 0 ? `Download ${num(selected)} selected` : `Download all ${num(all)}`,

  /* step 3: name and place */
  nameLabel: "Batch name",
  namePlaceholder: "Batch name, e.g. YouTube Collection November",
  nameRequired: "Give the batch a name. It becomes the folder its files are saved in.",
  location: "Download location",
  finalFolder: "Final folder",
  folderNote: "If a folder with this name is already there, a number is added, so nothing is overwritten.",
  quality: "Quality for video links",
  qualityHint: "Direct file links download as they are.",
  start: "Start batch download",
  started: (n: number, name: string) => `${name}: ${plural(n, { one: "# link", other: "# links" })} added`,

  /* the batch in Downloads */
  files: (n: number) => `${plural(n, { one: "# file", other: "# files" })}`,
  counts: {
    completed: (n: number) => `${num(n)} completed`,
    downloading: (n: number) => `${num(n)} downloading`,
    queued: (n: number) => `${num(n)} queued`,
    paused: (n: number) => `${num(n)} paused`,
    failed: (n: number) => `${num(n)} failed`,
    cancelled: (n: number) => `${num(n)} cancelled`,
  },
  now: (name: string) => `Now: ${name}`,
  preparing: "Preparing",
  waiting: "Waiting",
  completedWithErrors: "Completed with errors",
  cancelled: "Cancelled",

  /* details */
  details: "Details",
  detailTitle: (name: string) => `Batch ${name}`,
  overall: "Overall progress",
  downloaded: "Downloaded",
  speed: "Speed",
  folder: "Batch folder",
  created: "Created",
  finished: "Finished",
  links: "Links",
  pauseAll: "Pause All",
  resumeAll: "Resume All",
  retryFailed: "Retry Failed",
  cancelBatch: "Cancel Batch",
  openFolder: "Open Batch Folder",
  filter: {
    all: "All",
    active: "Active",
    done: "Done",
    problems: "Failed",
  },
  item: {
    pause: "Pause",
    resume: "Resume",
    retry: "Retry",
    cancel: "Cancel",
    openFile: "Open file",
    openFolder: "Show in folder",
    unknownSize: "Size not known yet",
  },
  cancelConfirm: {
    title: "Cancel this batch?",
    detail:
      "Every download of the batch that has not finished stops and its partial data is deleted. Files that finished stay in the batch folder.",
  },
};
