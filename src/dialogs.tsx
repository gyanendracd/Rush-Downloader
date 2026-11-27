import { useEffect, useMemo, useRef, useState, type FormEvent, type ReactNode } from "react";
import { ChevronDown, ChevronRight, File as FileGlyph, Folder, Search, type LucideIcon } from "lucide-react";
import {
  api,
  displayName,
  formatBytes,
  isTorrentSource,
  looksLikeUrl,
  QUALITIES,
  qualityOptions,
  type Download,
  type Found,
  type MediaOptions,
  type TorrentFile,
  type TorrentPreview,
} from "./api";
import { FileIcon, Modal, StatusText, useApp, Select } from "./ui";
import { t } from "./i18n";

/* ---------- add download ---------- */

export function AddDialog(props: {
  mode?: "download" | "scan";
  initialUrl: string;
  quality: string;
  onQuality: (q: string) => void;
  onClose: () => void;
}) {
  const { settings, actions } = useApp();
  const [url, setUrl] = useState(props.initialUrl);
  const [filename, setFilename] = useState("");
  const [dir, setDir] = useState("");
  const [connections, setConnections] = useState("");
  const [advanced, setAdvanced] = useState(false);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const valid = looksLikeUrl(url);
  const scanMode = props.mode === "scan";

  async function submit(e: FormEvent) {
    e.preventDefault();
    if (!valid || busy) return;
    if (scanMode) {
      props.onClose();
      actions.scan(url.trim());
      return;
    }
    if (isTorrentSource(url)) {
      // Torrents get their own dialog, with folder and file selection.
      props.onClose();
      actions.openTorrent(url.trim());
      return;
    }
    const duplicate = await actions.checkDuplicate(url);
    if (duplicate === "show") props.onClose();
    if (duplicate !== "go") return;
    setBusy(true);
    try {
      const n = Number(connections);
      await api.add({
        url: url.trim(),
        media: qualityOptions(props.quality),
        filename: filename.trim() || undefined,
        dir: dir || undefined,
        connections: connections && Number.isFinite(n) ? Math.min(32, Math.max(1, Math.round(n))) : undefined,
      });
      props.onClose();
    } catch (err) {
      setError(String(err));
      setBusy(false);
    }
  }

  async function browse() {
    const picked = await api.pickFolder(dir || settings?.download_dir);
    if (picked) setDir(picked);
  }

  return (
    <Modal
      title={scanMode ? t.dialogs.add.scanTitle : t.dialogs.add.title}
      onClose={props.onClose}
      footer={
        scanMode ? (
          <>
            <span className="spacer" />
            <button type="button" className="btn" onClick={props.onClose}>{t.dialogs.cancel}</button>
            <button type="submit" form="add-form" className="btn btn-primary" disabled={!valid}>
              {t.dialogs.add.findMedia}
            </button>
          </>
        ) : (
        <>
          <button
            type="button"
            className="btn"
            disabled={!valid}
            title={t.dialogs.add.scanPageHint}
            onClick={() => {
              props.onClose();
              actions.scan(url.trim());
            }}
          >
            {t.dialogs.add.scanPage}
          </button>
          <span className="spacer" />
          <button type="button" className="btn" onClick={props.onClose}>{t.dialogs.cancel}</button>
          <button type="submit" form="add-form" className="btn btn-primary" disabled={!valid || busy}>
            {t.dialogs.download}
          </button>
        </>
        )
      }
    >
      <form id="add-form" className="form" onSubmit={submit}>
        <label className="field">
          <span>{scanMode ? t.dialogs.add.pageLink : t.dialogs.add.link}</span>
          <input
            data-autofocus
            type="text"
            value={url}
            onChange={(e) => {
              setUrl(e.target.value);
              setError("");
            }}
            placeholder="https://"
            spellCheck={false}
            aria-invalid={url.trim() !== "" && !valid}
          />
          {url.trim() !== "" && !valid && (
            <small className="field-error">
              {t.dialogs.add.invalidLink}
            </small>
          )}
          {error && <small className="field-error">{error}</small>}
        </label>

        {scanMode && (
          <small className="form-note">
            {t.dialogs.add.scanNote}
          </small>
        )}
        {!scanMode && (
        <>
        <label className="field">
          <span>{t.dialogs.add.quality}</span>
          <Select
            aria-label={t.dialogs.add.quality}
            value={props.quality}
            onChange={props.onQuality}
            options={QUALITIES.map((q) => ({ value: q.value, label: q.label }))}
          />
          <small>{t.dialogs.add.qualityHint}</small>
        </label>

        <button
          type="button"
          className="disclosure"
          aria-expanded={advanced}
          onClick={() => setAdvanced((v) => !v)}
        >
          {advanced ? <ChevronDown size={15} aria-hidden="true" /> : <ChevronRight size={15} aria-hidden="true" />}
          {t.dialogs.add.moreOptions}
        </button>

        {advanced && (
          <>
            <label className="field">
              <span>{t.dialogs.add.fileName}</span>
              <input
                type="text"
                value={filename}
                onChange={(e) => setFilename(e.target.value)}
                placeholder={t.dialogs.add.fileNamePlaceholder}
                spellCheck={false}
              />
            </label>
            <div className="field">
              <span id="add-dir-label">{t.dialogs.saveTo}</span>
              <div className="field-row">
                <input
                  type="text"
                  readOnly
                  aria-labelledby="add-dir-label"
                  value={dir || settings?.download_dir || ""}
                />
                <button type="button" className="btn" onClick={browse}>{t.dialogs.browse}</button>
              </div>
            </div>
            <label className="field field-narrow">
              <span>{t.dialogs.add.connections}</span>
              <input
                type="number"
                min={1}
                max={32}
                value={connections}
                onChange={(e) => setConnections(e.target.value)}
                placeholder={String(settings?.connections ?? 16)}
              />
              <small>{t.dialogs.add.connectionsHint}</small>
            </label>
          </>
        )}
        </>
        )}
      </form>
    </Modal>
  );
}

/* ---------- scan a page for media ---------- */

export function ScanDialog(props: { url: string; media: MediaOptions; onClose: () => void }) {
  const [found, setFound] = useState<Found[] | null>(null);
  const [error, setError] = useState("");
  const [added, setAdded] = useState<Set<string>>(new Set());

  useEffect(() => {
    let cancelled = false;
    api
      .scan(props.url)
      .then((items) => !cancelled && setFound(items))
      .catch((e) => !cancelled && setError(String(e)));
    return () => {
      cancelled = true;
    };
  }, [props.url]);

  async function download(item: Found) {
    try {
      await api.add({
        url: item.url,
        media: props.media,
        headers: [["Referer", item.referer]],
        via_site: item.kind === "stream" || item.kind === "embed",
      });
      setAdded((prev) => new Set(prev).add(item.url));
    } catch (e) {
      setError(String(e));
    }
  }

  const pending = found?.filter((f) => !added.has(f.url)) ?? [];

  return (
    <Modal
      wide
      title={t.dialogs.scan.title}
      subtitle={props.url}
      onClose={props.onClose}
      footer={
        <>
          <span className="spacer" />
          <button type="button" className="btn" onClick={props.onClose}>{t.dialogs.close}</button>
          {found && found.length > 1 && (
            <button
              type="button"
              className="btn btn-primary"
              disabled={pending.length === 0}
              onClick={() => pending.forEach(download)}
            >
              {t.dialogs.scan.downloadAll(pending.length)}
            </button>
          )}
        </>
      }
    >
      {error && <p className="note note-error">{error}</p>}
      {!found && !error && (
        <ul className="found" aria-busy="true" aria-label={t.dialogs.scan.searching}>
          {[0, 1, 2].map((i) => (
            <li key={i} className="skeleton-row">
              <span className="skeleton skeleton-chip" />
              <span className="skeleton skeleton-line" />
            </li>
          ))}
        </ul>
      )}
      {found && found.length === 0 && (
        <p className="note">
          {t.dialogs.scan.nothingFound}
        </p>
      )}
      {found && found.length > 0 && (
        <ul className="found">
          {found.map((item) => (
            <li key={item.url}>
              <span className={`kind kind-${item.kind}`}>{t.dialogs.scan.kind[item.kind]}</span>
              <div className="found-main">
                <div className="found-name" title={item.url}>{item.name}</div>
                <div className="found-meta">
                  {item.size != null ? formatBytes(item.size) : new URL(item.url).host}
                </div>
              </div>
              {added.has(item.url) ? (
                <span className="found-added">{t.dialogs.scan.added}</span>
              ) : (
                <button type="button" className="btn" onClick={() => download(item)}>{t.dialogs.download}</button>
              )}
            </li>
          ))}
        </ul>
      )}
    </Modal>
  );
}

/* ---------- torrent: choose folder and files ---------- */

interface TreeFolder {
  key: string;
  name: string;
  folders: Map<string, TreeFolder>;
  files: TorrentFile[];
  /** Every file under it, at any depth, and their size: worked out once. */
  all: TorrentFile[];
  size: number;
}

/** A torrent with more files than this opens with its folders closed. */
const MANY_FILES = 200;
/** Files listed at first in an open folder, and how many more each click adds.
 *  A data set can hold tens of thousands; a row for each at once would hold
 *  up the window (and every click in it) for a long time. */
const FILES_SHOWN = 200;
const FILES_STEP = 500;

function buildTree(files: TorrentFile[]): TreeFolder {
  const root: TreeFolder = { key: "", name: "", folders: new Map(), files: [], all: [], size: 0 };
  for (const file of files) {
    let node = root;
    for (const part of file.path.slice(0, -1)) {
      let next = node.folders.get(part);
      if (!next) {
        next = { key: `${node.key}/${part}`, name: part, folders: new Map(), files: [], all: [], size: 0 };
        node.folders.set(part, next);
      }
      node = next;
    }
    node.files.push(file);
  }
  gather(root);
  return root;
}

/** Fills in each folder's `all` and `size`, from the bottom up. */
function gather(folder: TreeFolder): void {
  const all = [...folder.files];
  for (const child of folder.folders.values()) {
    gather(child);
    for (const f of child.all) all.push(f);
  }
  folder.all = all;
  folder.size = all.reduce((sum, f) => sum + f.size, 0);
}

/** Every folder's key, for closing them all. */
function folderKeys(folder: TreeFolder, into: string[] = []): string[] {
  for (const child of folder.folders.values()) {
    into.push(child.key);
    folderKeys(child, into);
  }
  return into;
}

/** Checkbox that can also show "some but not all". */
function Check(props: { state: "all" | "some" | "none"; label: string; onChange: (on: boolean) => void }) {
  return (
    <input
      type="checkbox"
      aria-label={props.label}
      checked={props.state === "all"}
      ref={(el) => {
        if (el) el.indeterminate = props.state === "some";
      }}
      onChange={(e) => props.onChange(e.target.checked)}
    />
  );
}

export function TorrentDialog(props: { source: string; onClose: () => void; onAdded: () => void }) {
  const { settings, actions } = useApp();
  const [preview, setPreview] = useState<TorrentPreview | null>(null);
  const [error, setError] = useState("");
  const [dir, setDir] = useState("");
  const [selected, setSelected] = useState<Set<number>>(new Set());
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  /** How many files each open folder lists, where more than the first few. */
  const [shown, setShown] = useState<Map<string, number>>(new Map());
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let cancelled = false;
    api
      .torrentPreview(props.source)
      .then((p) => {
        if (cancelled) return;
        setPreview(p);
        setSelected(new Set(p.files.map((f) => f.index)));
        // A large torrent opens folded: its folders closed, to be opened one at a time.
        if (p.files.length > MANY_FILES) setCollapsed(new Set(folderKeys(buildTree(p.files))));
      })
      .catch((e) => !cancelled && setError(String(e)));
    return () => {
      cancelled = true;
    };
  }, [props.source]);

  const tree = useMemo(() => (preview ? buildTree(preview.files) : null), [preview]);
  const chosenSize = preview
    ? preview.files.reduce((sum, f) => sum + (selected.has(f.index) ? f.size : 0), 0)
    : 0;
  const saveTo = dir || settings?.download_dir || "";

  function setMany(files: TorrentFile[], on: boolean) {
    setSelected((prev) => {
      const next = new Set(prev);
      for (const f of files) {
        if (on) next.add(f.index);
        else next.delete(f.index);
      }
      return next;
    });
  }

  async function browse() {
    const picked = await api.pickFolder(saveTo);
    if (picked) setDir(picked);
  }

  async function download() {
    if (!preview || selected.size === 0 || busy) return;
    setBusy(true);
    try {
      await api.add({
        url: props.source,
        media: { max_height: null, audio_only: false },
        dir: dir || undefined,
        // Leaving the list out means "everything".
        torrent_files:
          selected.size === preview.files.length ? undefined : [...selected].sort((a, b) => a - b),
      });
      actions.toast(t.dialogs.torrent.added);
      props.onClose();
      props.onAdded();
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  }

  function renderFolder(folder: TreeFolder, depth: number): ReactNode[] {
    const rows: ReactNode[] = [];
    for (const child of folder.folders.values()) {
      const inside = child.all;
      let picked = 0;
      for (const f of inside) if (selected.has(f.index)) picked++;
      const open = !collapsed.has(child.key);
      rows.push(
        <li key={`d${child.key}`} className="tree-row tree-folder" style={{ paddingLeft: 8 + depth * 20 }}>
          <button
            type="button"
            className="tree-toggle"
            aria-label={open ? t.dialogs.torrent.collapse(child.name) : t.dialogs.torrent.expand(child.name)}
            aria-expanded={open}
            onClick={() =>
              setCollapsed((prev) => {
                const next = new Set(prev);
                if (open) next.add(child.key);
                else next.delete(child.key);
                return next;
              })
            }
          >
            {open ? <ChevronDown size={15} aria-hidden="true" /> : <ChevronRight size={15} aria-hidden="true" />}
          </button>
          <Check
            label={t.dialogs.torrent.everythingIn(child.name)}
            state={picked === 0 ? "none" : picked === inside.length ? "all" : "some"}
            onChange={(on) => setMany(inside, on)}
          />
          <Folder size={16} strokeWidth={2} aria-hidden="true" className="tree-icon" />
          <span className="tree-name" title={child.name}>{child.name}</span>
          <span className="tree-size">{formatBytes(child.size)}</span>
        </li>,
      );
      if (open) rows.push(...renderFolder(child, depth + 1));
    }
    const limit = shown.get(folder.key) ?? FILES_SHOWN;
    for (const file of folder.files.slice(0, limit)) {
      const name = file.path[file.path.length - 1] ?? "";
      rows.push(
        <li key={`f${file.index}`} className="tree-row" style={{ paddingLeft: 8 + depth * 20 + (depth > 0 ? 22 : 0) }}>
          <Check
            label={name}
            state={selected.has(file.index) ? "all" : "none"}
            onChange={(on) => setMany([file], on)}
          />
          <FileGlyph size={16} strokeWidth={2} aria-hidden="true" className="tree-icon" />
          <span className="tree-name" title={file.path.join("/")}>{name}</span>
          <span className="tree-size">{formatBytes(file.size)}</span>
        </li>,
      );
    }
    const rest = folder.files.length - limit;
    if (rest > 0) {
      rows.push(
        <li key={`m${folder.key}`} className="tree-row tree-more" style={{ paddingLeft: 8 + depth * 20 + (depth > 0 ? 22 : 0) }}>
          <button
            type="button"
            className="link"
            onClick={() => setShown((prev) => new Map(prev).set(folder.key, limit + FILES_STEP))}
          >
            {t.dialogs.torrent.showMore(Math.min(rest, FILES_STEP), rest)}
          </button>
        </li>,
      );
    }
    return rows;
  }

  return (
    <Modal
      wide
      title={t.dialogs.torrent.title}
      subtitle={preview ? undefined : props.source}
      onClose={props.onClose}
      footer={
        <>
          <span className="tree-summary">
            {preview
              ? t.dialogs.torrent.summary(selected.size, preview.files.length, formatBytes(chosenSize), formatBytes(preview.total))
              : ""}
          </span>
          <span className="spacer" />
          <button type="button" className="btn" onClick={props.onClose}>{t.dialogs.cancel}</button>
          <button
            type="button"
            className="btn btn-primary"
            data-autofocus
            disabled={!preview || selected.size === 0 || busy}
            onClick={download}
          >
            {t.dialogs.download}
          </button>
        </>
      }
    >
      {error && <p className="note note-error">{error}</p>}
      {!preview && !error && (
        <>
          <p className="note">
            {t.dialogs.torrent.reading}
          </p>
          <ul className="found" aria-busy="true" aria-label={t.dialogs.torrent.readingLabel}>
            {[0, 1, 2].map((i) => (
              <li key={i} className="skeleton-row">
                <span className="skeleton skeleton-chip" />
                <span className="skeleton skeleton-line" />
              </li>
            ))}
          </ul>
        </>
      )}
      {preview && tree && (
        <div className="form">
          <div className="torrent-name" title={preview.name}>{preview.name}</div>
          <div className="field">
            <span id="torrent-dir">{t.dialogs.saveTo}</span>
            <div className="field-row">
              <input type="text" readOnly aria-labelledby="torrent-dir" value={saveTo} />
              <button type="button" className="btn" onClick={browse}>{t.dialogs.browse}</button>
            </div>
            <small>{t.dialogs.torrent.folderNote(preview.name)}</small>
          </div>
          <div className="field">
            <div className="tree-head">
              <span>{t.dialogs.torrent.files}</span>
              <span className="spacer" />
              <button type="button" className="link" onClick={() => setMany(preview.files, true)}>{t.dialogs.torrent.selectAll}</button>
              <button type="button" className="link" onClick={() => setMany(preview.files, false)}>{t.dialogs.torrent.selectNone}</button>
            </div>
            <ul className="tree" aria-label={t.dialogs.torrent.filesLabel}>{renderFolder(tree, 0)}</ul>
            {selected.size === 0 && <small className="field-error">{t.dialogs.torrent.chooseOne}</small>}
          </div>
        </div>
      )}
    </Modal>
  );
}

/* ---------- command palette ---------- */

export interface Command {
  id: string;
  label: string;
  group: string;
  icon: LucideIcon;
  shortcut?: string;
  run: () => void;
}

export function Palette(props: { commands: Command[]; onClose: () => void }) {
  const { downloads, actions } = useApp();
  const [query, setQuery] = useState("");
  const [index, setIndex] = useState(0);
  const listRef = useRef<HTMLDivElement>(null);

  type Entry =
    | { kind: "command"; command: Command }
    | { kind: "download"; download: Download };

  const entries = useMemo<Entry[]>(() => {
    const q = query.trim().toLowerCase();
    const commands = props.commands
      .filter((c) => !q || c.label.toLowerCase().includes(q))
      .map((command) => ({ kind: "command" as const, command }));
    const matches = q
      ? downloads
          .filter((d) => d.filename.toLowerCase().includes(q))
          .slice(0, 8)
          .map((download) => ({ kind: "download" as const, download }))
      : [];
    return [...commands, ...matches];
  }, [query, props.commands, downloads]);

  useEffect(() => setIndex(0), [query]);
  useEffect(() => {
    listRef.current?.querySelector(".is-current")?.scrollIntoView({ block: "nearest" });
  }, [index]);

  function run(entry: Entry | undefined) {
    if (!entry) return;
    props.onClose();
    if (entry.kind === "command") entry.command.run();
    else actions.inspect(entry.download.id);
  }

  let lastGroup = "";

  return (
    <div className="overlay overlay-top" onMouseDown={props.onClose}>
      <div
        className="palette"
        role="dialog"
        aria-modal="true"
        aria-label={t.dialogs.palette.label}
        onMouseDown={(e) => e.stopPropagation()}
      >
        <div className="palette-input">
          <Search size={16} strokeWidth={1.75} aria-hidden="true" />
          <input
            autoFocus
            type="text"
            value={query}
            placeholder={t.dialogs.palette.label}
            aria-label={t.dialogs.palette.label}
            spellCheck={false}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Escape") {
                e.stopPropagation();
                props.onClose();
              } else if (e.key === "ArrowDown") {
                e.preventDefault();
                setIndex((i) => Math.min(entries.length - 1, i + 1));
              } else if (e.key === "ArrowUp") {
                e.preventDefault();
                setIndex((i) => Math.max(0, i - 1));
              } else if (e.key === "Enter") {
                e.preventDefault();
                // A pasted link goes straight to the add dialog.
                if (looksLikeUrl(query)) {
                  props.onClose();
                  actions.openAdd(query.trim());
                } else {
                  run(entries[index]);
                }
              }
            }}
          />
        </div>
        <div className="palette-list" ref={listRef} role="listbox">
          {entries.length === 0 && (
            <p className="note">
              {looksLikeUrl(query) ? t.dialogs.palette.pressEnter : t.dialogs.palette.noMatch}
            </p>
          )}
          {entries.map((entry, i) => {
            const group = entry.kind === "command" ? entry.command.group : t.dialogs.palette.downloads;
            const heading = group !== lastGroup ? group : null;
            lastGroup = group;
            return (
              <div key={entry.kind === "command" ? entry.command.id : `d${entry.download.id}`}>
                {heading && <div className="palette-group">{heading}</div>}
                <button
                  type="button"
                  role="option"
                  aria-selected={i === index}
                  className={`palette-item${i === index ? " is-current" : ""}`}
                  onMouseMove={() => setIndex(i)}
                  onClick={() => run(entry)}
                >
                  {entry.kind === "command" ? (
                    <>
                      <entry.command.icon size={16} strokeWidth={1.75} aria-hidden="true" />
                      <span className="palette-label">{entry.command.label}</span>
                      {entry.command.shortcut && <kbd>{entry.command.shortcut}</kbd>}
                    </>
                  ) : (
                    <>
                      <FileIcon d={entry.download} />
                      <span className="palette-label">{displayName(entry.download)}</span>
                      <StatusText d={entry.download} />
                    </>
                  )}
                </button>
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}
