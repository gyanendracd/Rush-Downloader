/* English text of file sorting: its settings (src/Organize.tsx), the folder a
 * schedule saves to, and moving a finished download.
 * Other languages show this English text until they translate it. */

export const organize = {
  title: "Organize files by type",
  hint: "Finished downloads saved to the download folder go into a folder for their kind of file. A folder you pick for a download, a batch or a schedule is kept. Profiles, playlists and torrents keep their own folders.",
  enabled: "Sort finished files into folders",
  enabledHint: "Applies to downloads that finish from now on. A file of the same name is never overwritten: a number is added.",
  category: "Category",
  folder: "Folder",
  types: "File types",
  categoryName: (name: string) => `Name of ${name || "category"}`,
  folderOf: (name: string) => `Folder for ${name || "category"}`,
  browseFor: (name: string) => `Choose the folder for ${name || "category"}`,
  addType: "Add…",
  addTypeLabel: (name: string) => `Add a file type to ${name || "category"}`,
  removeType: (ext: string) => `Remove .${ext}`,
  removeCategory: (name: string) => `Remove ${name || "this category"}`,
  addCategory: "Add category",
  newCategory: "New category",
  other: "Everything else",
  otherHint: "Leave the folder empty to keep other files in the download folder.",
  folderHint: "A name is a folder inside the download folder; the folder button picks any folder.",
  reset: "Restore defaults",

  schedule: {
    folder: "Save finished files to",
    usual: "Their usual folder",
    choose: "Choose folder",
    clear: "Use their usual folder",
    hint: "Files of this schedule's downloads go here when they finish, in place of the folder for their kind.",
  },

  moveTo: "Move to…",
  moved: (place: string) => `Moved to ${place}`,
};
