/* The English text of the whole interface: the source every other language
 * translates. Each section belongs to one file of src/. */
import { app } from "./app";
import { common } from "./common";
import { detail } from "./detail";
import { dialogs } from "./dialogs";
import { history } from "./history";
import { ui } from "./ui";
import { views } from "./views";

export const en = { common, app, views, ui, detail, history, dialogs };

/** The shape every language must fill in: the same keys, strings where
 *  English has strings, functions taking the same arguments where English
 *  has functions. A missing or extra key fails the build. */
export type Dict = typeof en;
