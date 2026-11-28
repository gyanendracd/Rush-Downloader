/* Settings › Downloads › Organize files by type: which kinds of files go to
 * which folders once finished (crates/engine/src/organize.rs does the
 * sorting). Every change is saved at once, as elsewhere in Settings. */
import { useState, type KeyboardEvent } from "react";
import { FolderOpen, Plus, RotateCcw, X } from "lucide-react";
import { api, type FileCategory, type Organize } from "./api";
import { t } from "./i18n";
import { IconBtn } from "./ui";

/** The categories a new install starts with (the engine's own defaults). */
export const DEFAULT_ORGANIZE: Organize = {
  enabled: true,
  categories: [
    { name: "Videos", folder: "Videos", extensions: ["mp4", "mkv", "avi", "mov", "webm"] },
    { name: "Music", folder: "Music", extensions: ["mp3", "wav", "flac", "m4a", "aac"] },
    { name: "Documents", folder: "Documents", extensions: ["pdf", "docx", "doc", "xlsx", "pptx", "txt"] },
    { name: "Images", folder: "Images", extensions: ["jpg", "jpeg", "png", "gif", "webp", "svg"] },
    { name: "Archives", folder: "Archives", extensions: ["zip", "rar", "7z", "tar", "gz"] },
    { name: "Software", folder: "Software", extensions: ["exe", "msi", "dmg", "deb", "rpm", "apk"] },
  ],
  other: "Other",
};

/** File types typed in: "epub, .MOBI azw" → ["epub", "mobi", "azw"]. */
function typedTypes(text: string): string[] {
  return text
    .split(/[\s,;]+/)
    .map((e) => e.trim().replace(/^\.+/, "").toLowerCase())
    .filter((e) => e && !/[\\/]/.test(e));
}

/** One category: its name, folder and file types, each changed in place. */
function CategoryRow(props: {
  c: FileCategory;
  downloadDir: string;
  onChange: (c: FileCategory) => void;
  onRemove: () => void;
}) {
  const { c } = props;
  const [typing, setTyping] = useState("");
  const add = () => {
    const more = typedTypes(typing).filter((e) => !c.extensions.includes(e));
    if (more.length) props.onChange({ ...c, extensions: [...c.extensions, ...more] });
    setTyping("");
  };
  return (
    <li className="org-row">
      <input
        type="text"
        className="org-name"
        value={c.name}
        aria-label={t.organize.categoryName(c.name)}
        onChange={(e) => props.onChange({ ...c, name: e.target.value })}
      />
      <FolderField
        value={c.folder}
        label={t.organize.folderOf(c.name)}
        browseLabel={t.organize.browseFor(c.name)}
        downloadDir={props.downloadDir}
        onChange={(folder) => props.onChange({ ...c, folder })}
      />
      <div className="org-types">
        {c.extensions.map((ext) => (
          <span key={ext} className="org-type">
            .{ext}
            <button
              type="button"
              aria-label={t.organize.removeType(ext)}
              title={t.organize.removeType(ext)}
              onClick={() => props.onChange({ ...c, extensions: c.extensions.filter((e) => e !== ext) })}
            >
              <X size={11} strokeWidth={2.5} aria-hidden="true" />
            </button>
          </span>
        ))}
        <input
          type="text"
          className="org-type-add"
          value={typing}
          placeholder={t.organize.addType}
          aria-label={t.organize.addTypeLabel(c.name)}
          onChange={(e) => setTyping(e.target.value)}
          onBlur={add}
          onKeyDown={(e: KeyboardEvent) => {
            if (e.key === "Enter" || e.key === "," || e.key === " ") {
              e.preventDefault();
              add();
            } else if (e.key === "Backspace" && !typing && c.extensions.length) {
              props.onChange({ ...c, extensions: c.extensions.slice(0, -1) });
            }
          }}
        />
      </div>
      <IconBtn icon={X} label={t.organize.removeCategory(c.name)} onClick={props.onRemove} />
    </li>
  );
}

/** A folder as set ("Videos", or a full path) with a button to pick one. */
function FolderField(props: { value: string; label: string; browseLabel: string; downloadDir: string; onChange: (v: string) => void }) {
  return (
    <div className="org-folder">
      <input type="text" value={props.value} aria-label={props.label} title={props.value} onChange={(e) => props.onChange(e.target.value)} />
      <IconBtn
        icon={FolderOpen}
        label={props.browseLabel}
        onClick={async () => {
          const picked = await api.pickFolder(props.downloadDir);
          if (picked) props.onChange(picked);
        }}
      />
    </div>
  );
}

export function OrganizeSettings(props: { value: Organize; downloadDir: string; onChange: (o: Organize) => void }) {
  const o = props.value;
  const set = (next: Partial<Organize>) => props.onChange({ ...o, ...next });
  const setCategory = (i: number, c: FileCategory) => set({ categories: o.categories.map((x, j) => (j === i ? c : x)) });
  return (
    <div className={`org${o.enabled ? "" : " is-off"}`}>
      <div className="org-head" aria-hidden="true">
        <span>{t.organize.category}</span>
        <span>{t.organize.folder}</span>
        <span>{t.organize.types}</span>
      </div>
      <ul className="org-list">
        {o.categories.map((c, i) => (
          <CategoryRow
            key={i}
            c={c}
            downloadDir={props.downloadDir}
            onChange={(next) => setCategory(i, next)}
            onRemove={() => set({ categories: o.categories.filter((_, j) => j !== i) })}
          />
        ))}
        <li className="org-row org-other">
          <span className="org-name">{t.organize.other}</span>
          <FolderField
            value={o.other}
            label={t.organize.folderOf(t.organize.other)}
            browseLabel={t.organize.browseFor(t.organize.other)}
            downloadDir={props.downloadDir}
            onChange={(other) => set({ other })}
          />
          <small>{t.organize.otherHint}</small>
        </li>
      </ul>
      <div className="org-foot">
        <button
          type="button"
          className="btn btn-small"
          onClick={() => set({ categories: [...o.categories, { name: t.organize.newCategory, folder: t.organize.newCategory, extensions: [] }] })}
        >
          <Plus size={14} strokeWidth={2.2} aria-hidden="true" /> {t.organize.addCategory}
        </button>
        <button type="button" className="btn btn-small" onClick={() => props.onChange({ ...DEFAULT_ORGANIZE, enabled: o.enabled })}>
          <RotateCcw size={13} strokeWidth={2.2} aria-hidden="true" /> {t.organize.reset}
        </button>
        <small>{t.organize.folderHint}</small>
      </div>
    </div>
  );
}
