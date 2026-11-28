/* The English text of the whole interface: the source every other language
 * translates. Each section belongs to one file of src/. */
import { app } from "./app";
import { batch } from "./batch";
import { common } from "./common";
import { detail } from "./detail";
import { dialogs } from "./dialogs";
import { history } from "./history";
import { organize } from "./organize";
import { scheduler } from "./scheduler";
import { ui } from "./ui";
import { views } from "./views";

export const en = { common, app, views, ui, detail, history, dialogs, scheduler, batch, organize };

/** The shape every language must fill in: the same keys, strings where
 *  English has strings, functions taking the same arguments where English
 *  has functions. A missing or extra key fails the build. */
export type Dict = typeof en;

/** What another language fills in: everything, except sections added after
 *  it was translated, which show in English until they are (see
 *  `withEnglish` in ../index.ts). */
export type LocaleDict = Omit<Dict, "batch" | "organize"> & Partial<Pick<Dict, "batch" | "organize">>;
