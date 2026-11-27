/* English text of history. See src/i18n/README for how to add keys. */
import { plural } from "../format";

export const history = {
  title: "History",
  /** "12 links" */
  links: (n: number) => plural(n, { one: "# link", other: "# links" }),
  /** "12 links · 3 deleted" */
  linksDeleted: (links: string, deleted: number) =>
    `${links} · ${plural(deleted, { one: "# deleted", other: "# deleted" })}`,
  search: {
    placeholder: "Search history...",
    label: "Search history",
    clear: "Clear search",
  },
  empty: {
    title: "No history yet",
    text: "Every link you download is kept here, even after its file is deleted.",
  },
  table: "Download history",
  columns: {
    name: "Name",
    link: "Link",
    size: "Size",
    added: "Added",
    status: "Status",
  },
  noMatch: (query: string) => `Nothing in History matches “${query}”.`,
  outcome: {
    deletedHint: "Its file is no longer where it was saved",
    /** "Deleted · 3 days ago" */
    deleted: (ago: string) => `Deleted · ${ago}`,
  },
  /** A profile or playlist group. */
  group: {
    playlist: "Playlist",
    profile: "Profile",
    /** "Playlist · 40 files" */
    kindCount: (kind: string, files: string) => `${kind} · ${files}`,
    hideItems: "Hide its items",
    showItems: "Show its items",
  },
  items: {
    label: (name: string) => `What ${name} holds`,
    goneUnknown:
      "Its files are gone, and their names were not kept: it was downloaded before History began keeping them.",
    nothingYet: "Nothing has been fetched for it yet.",
    more: (n: number) => plural(n, { one: "and # more", other: "and # more" }),
    state: {
      done: "Downloaded",
      downloading: "Downloading",
      waiting: "Waiting",
      missed: "Not available",
      deleted: "Deleted",
    },
  },
};
