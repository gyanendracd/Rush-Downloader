/* English text of dialogs. See src/i18n/README for how to add keys. */
import { num, plural } from "../format";

export const dialogs = {
  /** Shared buttons. */
  cancel: "Cancel",
  close: "Close",
  download: "Download",
  browse: "Browse",
  saveTo: "Save to",

  add: {
    title: "Add download",
    scanTitle: "Add from a web page",
    findMedia: "Find media",
    scanPage: "Scan page for media",
    scanPageHint: "List the video, audio and stream links found on this page",
    link: "Link",
    pageLink: "Page link",
    invalidLink: "Enter a full link: one starting with http:// or https://, or a magnet link",
    scanNote:
      "Paste the address of a page from your browser. Every video, audio and stream link in it is listed so you can pick what to download.",
    quality: "Quality for video pages",
    qualityHint: "Applies to site links such as YouTube. Direct file links ignore it.",
    moreOptions: "More options",
    fileName: "File name",
    fileNamePlaceholder: "Use the name the server gives",
    connections: "Connections",
    connectionsHint: "1 to 32. More helps only when the server allows it.",
  },

  scan: {
    title: "Media found on this page",
    downloadAll: (n: number) => `Download all (${num(n)})`,
    searching: "Searching the page",
    nothingFound:
      "No video or audio links are in this page's code. Some sites only load them after you press play, or keep them behind a login or another page.",
    added: "Added",
    kind: {
      video: "Video",
      stream: "Stream",
      audio: "Audio",
      embed: "Player",
    },
  },

  torrent: {
    title: "Add torrent",
    added: "Torrent added",
    collapse: (name: string) => `Collapse ${name}`,
    expand: (name: string) => `Expand ${name}`,
    everythingIn: (name: string) => `Everything in ${name}`,
    /** "3 of 5 files · 1.2 GB of 2 GB" */
    summary: (selected: number, total: number, chosenSize: string, totalSize: string) =>
      plural(total, {
        one: `${num(selected)} of # file · ${chosenSize} of ${totalSize}`,
        other: `${num(selected)} of # files · ${chosenSize} of ${totalSize}`,
      }),
    reading:
      "Reading the torrent. For a magnet link this can take up to a minute while people sharing it are found.",
    readingLabel: "Reading the torrent",
    folderNote: (name: string) => `A folder named "${name}" is created inside it for this torrent.`,
    files: "Files to download",
    selectAll: "Select all",
    selectNone: "Select none",
    filesLabel: "Files in this torrent",
    chooseOne: "Choose at least one file.",
    /** A folder with more files than are listed: a click lists `batch` more. */
    showMore: (batch: number, rest: number) =>
      plural(rest, {
        one: `Show ${num(batch)} more · # file not shown`,
        other: `Show ${num(batch)} more · # files not shown`,
      }),
  },

  site: {
    title: `Choose files`,
    reading: `Reading the list of files…`,
    readingLabel: `Reading the list of files`,
    folderNote: (name: string) => `Several files go into a folder named "${name}" inside it.`,
    partial: `This link holds more files than can be listed; only the first ones are shown.`,
    filesLabel: `Files at this link`,
    added: (n: number) => plural(n, { one: `Download started`, other: `# files are downloading` }),
    /** Why a site's links cannot be downloaded. */
    blocked: {
      store: (site: string) => `${site} games only download in ${site}'s own app, with its copy protection. RushDownload can't download them.`,
      paid: (site: string) => `${site} courses are only for signed-in, paying students. RushDownload can't download them.`,
      encrypted: (site: string) => `${site} locks files with a key only your browser gets. RushDownload can't open them.`,
      search: (site: string) => `${site} only points to other sites. Open a result and download from that site.`,
      siteOnly: (site: string) => `${site} only hands files to its own web page. Open the link in your browser and download it there.`,
    },
  },

  palette: {
    label: "Search commands and downloads",
    pressEnter: "Press Enter to download this link.",
    noMatch: "Nothing matches that.",
    downloads: "Downloads",
  },
};
