/* Batch Download: the dialog that finds every link in pasted text or files,
 * lists them for review and export, and starts them as one batch; and the
 * batch's details, listing each of its downloads with its own controls.
 *
 * Finding links is urlExtraction.ts (in a worker for large input); the
 * downloads themselves are the engine's (crates/engine/src/batch.rs). */
import { useEffect, useMemo, useRef, useState, type DragEvent } from "react";
import {
  Archive,
  Check,
  Copy,
  File as FileGlyph,
  Film,
  Globe,
  Image as ImageGlyph,
  Music,
  Download as DownloadGlyph,
  FileText,
  FolderOpen,
  Layers,
  Pause,
  Play,
  RotateCw,
  Search,
  SquareArrowOutUpRight,
  Trash2,
  Upload,
  X,
} from "lucide-react";
import {
  api,
  formatBytes,
  formatDate,
  formatEta,
  formatSpeed,
  isTauri,
  percentOf,
  QUALITIES,
  qualityOptions,
  STATUS_LABEL,
  type Download,
} from "./api";
import { pct, t } from "./i18n";
import { IconBtn, Modal, Select, useApp } from "./ui";
import { SumsButton } from "./Checksum";
import { extractUrls, toCsv, toTxt, type ExtractedUrl, type ExtractionResult, type ExtractionSource } from "./urlExtraction";
import type { WorkerReply } from "./urlExtraction.worker";

/* ---------- running the link finder ---------- */

/** Inputs up to this size are read at once; larger ones in a worker. */
const INLINE_CHARS = 300_000;

/** Finds the links in `sources` without holding up the window; `cancel` stops it. */
function runExtraction(
  sources: ExtractionSource[],
  onProgress: (done: number, total: number) => void,
): { result: Promise<ExtractionResult>; cancel: () => void } {
  const size = sources.reduce((sum, s) => sum + s.text.length, 0);
  let worker: Worker | null = null;
  if (size > INLINE_CHARS) {
    try {
      worker = new Worker(new URL("./urlExtraction.worker.ts", import.meta.url), { type: "module" });
    } catch {
      worker = null;
    }
  }
  if (!worker) {
    let stopped = false;
    // A tick later, so the "finding links" state is drawn first.
    const result = new Promise<ExtractionResult>((resolve, reject) =>
      window.setTimeout(() => (stopped ? reject(new Error("stopped")) : resolve(extractUrls(sources))), 0),
    );
    return { result, cancel: () => (stopped = true) };
  }
  const w = worker;
  let fail: (e: Error) => void = () => {};
  const result = new Promise<ExtractionResult>((resolve, reject) => {
    fail = reject;
    w.onmessage = (e: MessageEvent<WorkerReply>) => {
      const reply = e.data;
      if (reply.kind === "progress") onProgress(reply.done, reply.total);
      else {
        w.terminate();
        if (reply.kind === "done") resolve(reply.result);
        else reject(new Error(reply.message));
      }
    };
    w.onerror = (e) => {
      w.terminate();
      reject(new Error(e.message));
    };
    w.postMessage({ sources });
  });
  return {
    result,
    cancel: () => {
      w.terminate();
      fail(new Error("stopped"));
    },
  };
}

/* ---------- files ---------- */

interface InputFile {
  key: string;
  name: string;
  bytes: number | null;
  /** Its text, once read. */
  text: string | null;
  /** Why it could not be used. */
  error: string | null;
}

let fileKeys = 0;

/** A file dropped in a browser (development, or where the window lets the
 *  page have drops): read here, binary files turned away by their bytes. */
async function readBrowserFile(file: File): Promise<InputFile> {
  const key = `b${++fileKeys}`;
  const head = new Uint8Array(await file.slice(0, 8192).arrayBuffer());
  const utf16 = (head[0] === 0xff && head[1] === 0xfe) || (head[0] === 0xfe && head[1] === 0xff);
  if (!utf16 && head.includes(0)) return { key, name: file.name, bytes: file.size, text: null, error: t.batch.fileRejected };
  const text = utf16
    ? new TextDecoder(head[0] === 0xff ? "utf-16le" : "utf-16be").decode(await file.arrayBuffer())
    : await file.text();
  return { key, name: file.name, bytes: file.size, text, error: null };
}

/** What a folder name becomes on disk: the engine's own rule (probe.rs). */
export function folderName(name: string): string {
  return name
    .replace(/[<>:"/\\|?*\u0000-\u001f]/g, "_")
    .trim()
    .replace(/[. ]+$/, "");
}

/** Joins a folder and a name with the separator the folder already uses. */
function joinPath(dir: string, name: string): string {
  if (!dir) return name;
  const sep = dir.includes("\\") ? "\\" : "/";
  return dir.endsWith(sep) ? dir + name : dir + sep + name;
}

/** More rows than this are drawn only when asked for. */
const ROWS_SHOWN = 300;
const ROWS_STEP = 1000;

type Step = "input" | "results";
const STEPS: Step[] = ["input", "results"];

/* ---------- thumbnails ---------- */

const YOUTUBE_ID = /^(?:[\w-]+\.)?(?:youtube\.com|youtube-nocookie\.com|youtu\.be)$/i;

/** A picture for a link, read from the link alone (nothing is fetched to
 *  find it): a YouTube video's own thumbnail, or an image link itself. */
export function thumbOf(link: string): string | null {
  let u: URL;
  try {
    u = new URL(link);
  } catch {
    return null;
  }
  if (YOUTUBE_ID.test(u.hostname)) {
    const parts = u.pathname.split("/").filter(Boolean);
    const id =
      u.hostname.toLowerCase().endsWith("youtu.be")
        ? parts[0]
        : u.searchParams.get("v") ?? (["live", "shorts", "embed", "v"].includes(parts[0]) ? parts[1] : null);
    return id && /^[\w-]{6,20}$/.test(id) ? `https://i.ytimg.com/vi/${id}/mqdefault.jpg` : null;
  }
  return /\.(jpe?g|png|gif|webp|avif|bmp)$/i.test(u.pathname) ? link : null;
}

/** What kind of thing a link is, by its ending, for its mark. */
function kindIcon(link: string) {
  let path = link;
  try {
    path = new URL(link).pathname;
  } catch {
    // as written
  }
  const ext = /\.([a-z0-9]{2,5})$/i.exec(path)?.[1].toLowerCase() ?? "";
  if (/^(mp4|mkv|webm|mov|m4v|avi|flv|wmv|ts|m3u8)$/.test(ext)) return Film;
  if (/^(mp3|m4a|flac|wav|ogg|opus|aac)$/.test(ext)) return Music;
  if (/^(jpe?g|png|gif|webp|avif|bmp|svg)$/.test(ext)) return ImageGlyph;
  if (/^(zip|rar|7z|tar|gz|xz|iso)$/.test(ext)) return Archive;
  if (ext && !/^(html?|php|aspx?|jsp)$/.test(ext)) return FileGlyph;
  return Globe;
}

/** A link's picture where one is known, else a mark of its kind. Loaded
 *  only as the row comes into view. */
export function LinkThumb({ url, picture }: { url: string; picture?: string | null }) {
  const src = picture || thumbOf(url);
  const [broken, setBroken] = useState<string | null>(null);
  if (src && broken !== src) {
    return (
      <img className="batch-thumb" src={src} alt="" loading="lazy" decoding="async" referrerPolicy="no-referrer" onError={() => setBroken(src)} />
    );
  }
  const Icon = kindIcon(url);
  return (
    <span className="batch-thumb batch-thumb-empty" aria-hidden="true">
      <Icon size={15} strokeWidth={2} />
    </span>
  );
}

/* ---------- the dialog ---------- */

export function BatchDialog(props: {
  quality: string;
  onClose: () => void;
  onStarted: () => void;
  /** Hands this dialog the paths of files dropped onto the window, while it is open. */
  takeDrops: (take: ((paths: string[]) => void) | null) => void;
}) {
  const { settings, actions } = useApp();
  const [step, setStep] = useState<Step>("input");
  const [text, setText] = useState("");
  const [files, setFiles] = useState<InputFile[]>([]);
  const [reading, setReading] = useState(0);
  const [dragging, setDragging] = useState(false);
  const [running, setRunning] = useState<{ done: number; total: number } | null>(null);
  const stopRun = useRef<(() => void) | null>(null);
  /** The message at the foot of the popup; `n` counts them, so the same
   *  message again starts its life over. */
  const [alert, setAlert] = useState<{ text: string; n: number } | null>(null);
  const setError = (text: string) => setAlert((had) => (text ? { text, n: (had?.n ?? 0) + 1 } : null));
  // Shown for three seconds, then it fades out (`hint-life` in styles.css).
  useEffect(() => {
    if (!alert) return;
    const timer = window.setTimeout(() => setAlert(null), 4200);
    return () => window.clearTimeout(timer);
  }, [alert]);
  /** What the last extraction found, before the user edits the list. */
  const [found, setFound] = useState<ExtractionResult | null>(null);
  const [filesUsed, setFilesUsed] = useState(0);
  const [list, setList] = useState<ExtractedUrl[]>([]);
  const [picked, setPicked] = useState<Set<string>>(new Set());
  const [query, setQuery] = useState("");
  const [shown, setShown] = useState(ROWS_SHOWN);
  const [name, setName] = useState("");
  const [nameMissing, setNameMissing] = useState(false);
  const [dir, setDir] = useState("");
  const [quality, setQuality] = useState(props.quality);
  const [busy, setBusy] = useState(false);
  const input = useRef<HTMLInputElement>(null);
  const nameField = useRef<HTMLInputElement>(null);
  const goButton = useRef<HTMLButtonElement>(null);

  /** Shakes the download button: what it needs is missing. From the start
   *  on every click. */
  function shake() {
    const el = goButton.current;
    if (!el) return;
    el.classList.remove("is-shaking");
    void el.offsetWidth;
    el.classList.add("is-shaking");
  }

  const saveTo = dir || settings?.download_dir || "";
  const folder = folderName(name);

  // Live count of the links in the pasted text, while it is small.
  const liveCount = useMemo(
    () => (text.length > 0 && text.length <= INLINE_CHARS ? extractUrls([{ text, name: null }]).urls.length : null),
    [text],
  );

  async function addPaths(paths: string[]) {
    setError("");
    setReading((n) => n + paths.length);
    for (const path of paths) {
      const fallback = path.split(/[\\/]/).pop() ?? path;
      const key = `p${++fileKeys}`;
      try {
        const file = await api.readTextFile(path);
        setFiles((list) => [...list, { key, name: file.name, bytes: file.bytes, text: file.text, error: null }]);
      } catch (e) {
        const why = String(e).replace(`${fallback}: `, "");
        setFiles((list) => [...list, { key, name: fallback, bytes: null, text: null, error: why }]);
      } finally {
        setReading((n) => n - 1);
      }
    }
  }
  const addPathsRef = useRef(addPaths);
  addPathsRef.current = addPaths;

  // In the app, the window itself takes dropped files (the page never sees
  // them): the drop area lights up while files are held over the window.
  useEffect(() => {
    if (!isTauri || step !== "input") return;
    const stop = api.onFileHover(setDragging);
    return () => {
      stop.then((f) => f());
      setDragging(false);
    };
  }, [step]);

  // Files dropped on the window come here while the dialog is open.
  useEffect(() => {
    props.takeDrops((paths) => {
      if (step === "input") void addPathsRef.current(paths);
    });
    return () => props.takeDrops(null);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [step]);

  async function addBrowserFiles(list: FileList | File[]) {
    const all = Array.from(list);
    setReading((n) => n + all.length);
    for (const file of all) {
      const read = await readBrowserFile(file).catch(
        (e) => ({ key: `b${++fileKeys}`, name: file.name, bytes: file.size, text: null, error: String(e) }) as InputFile,
      );
      setFiles((had) => [...had, read]);
      setReading((n) => n - 1);
    }
  }

  async function browse() {
    if (!isTauri) {
      input.current?.click();
      return;
    }
    try {
      const paths = await api.pickTextFiles();
      if (paths.length) await addPaths(paths);
    } catch (e) {
      setError(String(e));
    }
  }

  function onDrop(e: DragEvent) {
    e.preventDefault();
    setDragging(false);
    if (e.dataTransfer.files.length) void addBrowserFiles(e.dataTransfer.files);
  }

  const usable = files.filter((f) => f.text != null);
  const canExtract = (text.trim() !== "" || usable.length > 0) && reading === 0;

  /** Finds the links, then shows them for review, or with "start"
   *  downloads all of them at once (Download all). */
  async function extract(then: "results" | "start" = "results") {
    if (!canExtract || running) {
      if (!running) {
        setError(t.batch.nothingToExtract);
        if (then === "start") shake();
      }
      return;
    }
    if (then === "start" && !folder) {
      setNameMissing(true);
      nameField.current?.focus();
      shake();
      return;
    }
    setError("");
    const sources: ExtractionSource[] = [
      ...(text.trim() ? [{ text, name: null }] : []),
      ...usable.map((f) => ({ text: f.text!, name: f.name })),
    ];
    const run = runExtraction(sources, (done, total) => setRunning({ done, total }));
    stopRun.current = run.cancel;
    setRunning({ done: 0, total: sources.length });
    try {
      const result = await run.result;
      if (result.urls.length === 0) {
        setError(t.batch.noneFound);
        return;
      }
      setFound(result);
      setFilesUsed(usable.length);
      setList(result.urls);
      setPicked(new Set());
      setQuery("");
      setShown(ROWS_SHOWN);
      if (then === "start") void start(result.urls);
      else setStep("results");
    } catch (e) {
      if (String(e) !== "Error: stopped") setError(String(e));
    } finally {
      stopRun.current = null;
      setRunning(null);
    }
  }

  /* ----- the list ----- */
  const q = query.trim().toLowerCase();
  const visible = useMemo(
    () => (q ? list.filter((u) => u.url.toLowerCase().includes(q) || (u.source ?? "").toLowerCase().includes(q)) : list),
    [list, q],
  );
  const index = useMemo(() => new Map(list.map((u, i) => [u.url, i + 1])), [list]);
  /** What the export and the download act on: the picked links, else all. */
  const chosen = picked.size > 0 ? list.filter((u) => picked.has(u.url)) : list;

  function setPick(urls: string[], on: boolean) {
    setPicked((had) => {
      const next = new Set(had);
      for (const u of urls) {
        if (on) next.add(u);
        else next.delete(u);
      }
      return next;
    });
  }
  function removeUrls(urls: Set<string>) {
    setList((had) => had.filter((u) => !urls.has(u.url)));
    setPicked((had) => new Set([...had].filter((u) => !urls.has(u))));
  }
  function copy(urls: string[]) {
    actions.copy(urls.join("\n"), t.batch.copied(urls.length));
  }

  async function exportList(kind: "txt" | "csv") {
    const rows = chosen;
    if (rows.length === 0) return;
    const contents = kind === "txt" ? toTxt(rows.map((r) => r.url)) : toCsv(rows);
    const fileName = kind === "txt" ? "extracted-urls.txt" : "extracted-urls.csv";
    try {
      if (isTauri) {
        const saved = await api.saveTextFile(fileName, contents, kind);
        if (saved) actions.toast(t.batch.exported(rows.length, saved.split(/[\\/]/).pop() ?? saved));
        return;
      }
      // A plain browser (development): the browser's own download.
      const href = URL.createObjectURL(new Blob([contents], { type: kind === "txt" ? "text/plain" : "text/csv" }));
      const a = document.createElement("a");
      a.href = href;
      a.download = fileName;
      a.click();
      window.setTimeout(() => URL.revokeObjectURL(href), 1000);
      actions.toast(t.batch.exported(rows.length, fileName));
    } catch (e) {
      actions.toast(String(e), "error");
    }
  }

  /* ----- starting ----- */

  async function start(links: ExtractedUrl[] = chosen) {
    if (busy) return;
    if (!folder) {
      setNameMissing(true);
      nameField.current?.focus();
      shake();
      return;
    }
    if (links.length === 0) {
      shake();
      return;
    }
    setBusy(true);
    try {
      const urls = links.map((u) => u.url);
      await api.addBatch(name.trim(), urls, qualityOptions(quality), dir || undefined);
      actions.toast(t.batch.started(urls.length, name.trim()));
      props.onClose();
      props.onStarted();
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  }

  const footer =
    step === "input" ? (
      <>
        {running ? (
          <span className="tree-summary" role="status">
            {t.batch.extracting(running.done, running.total)}
          </span>
        ) : (
          <span className="tree-summary" />
        )}
        <span className="spacer" />
        <button type="button" className="btn" onClick={running ? () => stopRun.current?.() : props.onClose}>
          {running ? t.batch.stop : t.dialogs.cancel}
        </button>
        <button
          ref={goButton}
          type="button"
          className="btn btn-primary batch-go"
          aria-disabled={!canExtract || !folder || undefined}
          disabled={!!running || busy}
          onClick={() => extract("start")}
          onAnimationEnd={(e) => e.currentTarget.classList.remove("is-shaking")}
        >
          <DownloadGlyph size={15} strokeWidth={2.2} aria-hidden="true" /> {t.batch.downloadAll}
        </button>
      </>
    ) : (
      <>
        <button type="button" className="btn" onClick={() => setStep("input")}>
          {t.batch.back}
        </button>
        <span className="spacer" />
        <button
          ref={goButton}
          type="button"
          className="btn btn-primary batch-go"
          aria-disabled={chosen.length === 0 || !folder || undefined}
          disabled={busy}
          onClick={() => start()}
          onAnimationEnd={(e) => e.currentTarget.classList.remove("is-shaking")}
        >
          <DownloadGlyph size={15} strokeWidth={2.2} aria-hidden="true" /> {t.batch.next(picked.size, list.length)}
        </button>
      </>
    );

  return (
    <Modal wide title={t.batch.title} onClose={props.onClose} footer={footer}>
      <div className="batch-top">
      <ol className="batch-steps" aria-label={t.batch.title}>
        {STEPS.map((s, i) => {
          // Review is optional: open it any time there are links to look at.
          const reachable = s === "input" || list.length > 0 || canExtract;
          return (
            <li key={s} className={s === step ? "is-on" : undefined}>
              <button
                type="button"
                disabled={!reachable || !!running}
                aria-current={s === step ? "step" : undefined}
                onClick={() => {
                  if (s === step) return;
                  if (s === "results" && list.length === 0) void extract();
                  else setStep(s);
                }}
              >
                <span aria-hidden="true">{i + 1}</span>
                {t.batch.steps[s]}
              </button>
            </li>
          );
        })}
      </ol>
        {step === "input" && (
          <button type="button" className="btn btn-small batch-extract" disabled={!canExtract || !!running} onClick={() => extract()}>
            <Search size={14} strokeWidth={2.2} aria-hidden="true" /> {t.batch.extract}
          </button>
        )}
      </div>

      {step === "input" && (
        <div className="form batch-input">
          <label className="field">
            <span>{t.batch.pasteLabel}</span>
            <textarea
              className="batch-text"
              value={text}
              onChange={(e) => setText(e.target.value)}
              placeholder={t.batch.pastePlaceholder}
              spellCheck={false}
              data-autofocus
              rows={6}
            />
            {liveCount != null && <small className={liveCount > 0 ? "batch-live is-found" : "batch-live"}>{t.batch.liveCount(liveCount)}</small>}
          </label>
          <div className="field">
            <span>{t.batch.filesLabel}</span>
            <div
              className={`batch-drop${dragging ? " is-dragging" : ""}`}
              onDragOver={(e) => {
                e.preventDefault();
                setDragging(true);
              }}
              onDragLeave={() => setDragging(false)}
              onDrop={onDrop}
            >
              <Upload size={18} strokeWidth={2} aria-hidden="true" />
              <span>
                {dragging ? t.batch.dropNow : t.batch.drop} {!dragging && <>{t.batch.dropOr} </>}
              </span>
              {!dragging && (
                <button type="button" className="btn btn-small" onClick={browse}>
                  {t.batch.browse}
                </button>
              )}
              <small>{t.batch.dropHint}</small>
              <input
                ref={input}
                type="file"
                multiple
                hidden
                onChange={(e) => {
                  if (e.target.files?.length) void addBrowserFiles(e.target.files);
                  e.target.value = "";
                }}
              />
            </div>
            {(files.length > 0 || reading > 0) && (
              <ul className="batch-files">
                {files.map((f) => (
                  <li key={f.key} className={f.error ? "is-rejected" : undefined}>
                    <FileText size={15} strokeWidth={2} aria-hidden="true" />
                    <span className="batch-file-name" title={f.name}>{f.name}</span>
                    <span className="batch-file-meta">{f.error ?? (f.bytes != null ? formatBytes(f.bytes) : "")}</span>
                    <IconBtn icon={X} label={t.batch.removeFile(f.name)} onClick={() => setFiles((list) => list.filter((x) => x.key !== f.key))} />
                  </li>
                ))}
                {reading > 0 && <li className="is-reading">{t.batch.reading}</li>}
              </ul>
            )}
          </div>
          <div className="batch-where">
            <input
              ref={nameField}
              type="text"
              className="batch-name"
              value={name}
              placeholder={t.batch.namePlaceholder}
              aria-label={t.batch.nameLabel}
              aria-invalid={nameMissing && !folder}
              onChange={(e) => {
                setName(e.target.value);
                setNameMissing(false);
              }}
            />
            <button
              type="button"
              className="btn batch-location"
              title={saveTo}
              onClick={async () => {
                const pickedDir = await api.pickFolder(saveTo);
                if (pickedDir) setDir(pickedDir);
              }}
            >
              <FolderOpen size={15} strokeWidth={2} aria-hidden="true" /> {t.batch.location}
            </button>
            <Select
              className="batch-quality"
              aria-label={t.batch.quality}
              value={quality}
              onChange={setQuality}
              options={QUALITIES.map((q) => ({ value: q.value, label: q.label }))}
            />
            {nameMissing && !folder ? (
              <small className="field-error batch-where-note">{t.batch.nameRequired}</small>
            ) : (
              <small className="batch-where-note" title={t.batch.folderNote}>
                {t.batch.finalFolder}: <span className="selectable">{joinPath(saveTo, folder || "…")}</span>
              </small>
            )}
          </div>
        </div>
      )}

      {step === "results" && found && (
        <div className="batch-results">
          <div className="batch-summary">
            <div className="batch-summary-head">
              <Check size={16} strokeWidth={2.5} aria-hidden="true" />
              <strong>{t.batch.complete}</strong>
              <span className="spacer" />
              <span className="batch-export">
                <small>{t.batch.exportScope(picked.size, list.length)}</small>
                <button type="button" className="btn btn-small btn-primary" onClick={() => exportList("txt")} disabled={chosen.length === 0}>
                  <DownloadGlyph size={14} strokeWidth={2.2} aria-hidden="true" /> {t.batch.exportTxt}
                </button>
                <button type="button" className="btn btn-small" title={t.batch.exportCsvTitle} onClick={() => exportList("csv")} disabled={chosen.length === 0}>
                  {t.batch.exportCsv}
                </button>
              </span>
            </div>
            <dl className="batch-stats">
              {(
                [
                  ["files", filesUsed],
                  ["discovered", found.discovered],
                  ["unique", found.urls.length],
                  ["duplicates", found.duplicates],
                  ["invalid", found.invalid],
                ] as const
              ).map(([key, value]) => (
                <div key={key}>
                  <dt>{t.batch.stats[key]}</dt>
                  <dd>{value.toLocaleString()}</dd>
                </div>
              ))}
            </dl>
            {files.some((f) => f.error) && (
              <small className="field-error">{t.batch.skipped(files.filter((f) => f.error).map((f) => f.name).join(", "))}</small>
            )}
          </div>

          <div className="batch-toolbar">
            <label className="search">
              <Search size={15} strokeWidth={2} aria-hidden="true" />
              <input
                type="text"
                value={query}
                onChange={(e) => {
                  setQuery(e.target.value);
                  setShown(ROWS_SHOWN);
                }}
                placeholder={t.batch.search}
                aria-label={t.batch.search}
                spellCheck={false}
              />
            </label>
            <span className="spacer" />
            <button type="button" className="link" onClick={() => setPick(visible.map((u) => u.url), true)}>{t.batch.selectAll}</button>
            <button type="button" className="link" disabled={picked.size === 0} onClick={() => setPicked(new Set())}>{t.batch.deselectAll}</button>
            <button type="button" className="link" disabled={picked.size === 0} onClick={() => copy(list.filter((u) => picked.has(u.url)).map((u) => u.url))}>
              {t.batch.copySelected}
            </button>
            <button type="button" className="link" disabled={picked.size === 0} onClick={() => removeUrls(new Set(picked))}>{t.batch.removeSelected}</button>
            <button type="button" className="link is-danger" disabled={list.length === 0} onClick={() => removeUrls(new Set(list.map((u) => u.url)))}>
              {t.batch.clear}
            </button>
          </div>

          {list.length === 0 ? (
            <p className="note">{t.batch.emptyList}</p>
          ) : visible.length === 0 ? (
            <p className="note">{t.batch.searchEmpty}</p>
          ) : (
            <ul className="batch-urls" aria-label={t.batch.links}>
              {visible.slice(0, shown).map((u) => (
                <li key={u.url} className={picked.has(u.url) ? "is-picked" : undefined}>
                  <input type="checkbox" checked={picked.has(u.url)} aria-label={t.batch.selectOne(u.url)} onChange={(e) => setPick([u.url], e.target.checked)} />
                  <span className="batch-url-index">{index.get(u.url)}</span>
                  <LinkThumb url={u.url} />
                  <span className="batch-url" title={u.url}>{u.url}</span>
                  <span className="batch-url-source" title={u.sources.join(", ")}>{u.source ?? t.batch.pasted}</span>
                  <span className="batch-url-actions">
                    <IconBtn icon={Copy} label={t.batch.copyOne} onClick={() => copy([u.url])} />
                    <IconBtn icon={X} label={t.batch.removeOne} onClick={() => removeUrls(new Set([u.url]))} />
                  </span>
                </li>
              ))}
            </ul>
          )}
          {visible.length > shown && (
            <p className="batch-more">
              {t.batch.shownOf(shown, visible.length)}{" "}
              <button type="button" className="link" onClick={() => setShown((n) => n + ROWS_STEP)}>
                {t.batch.showMore(Math.min(ROWS_STEP, visible.length - shown))}
              </button>
            </p>
          )}
        </div>
      )}

      {/* Alerts at the foot, as on Home: a layer that takes no room, so
          nothing in the popup moves when one appears or goes. */}
      <div className="home-alerts batch-alerts">
        <div className="home-alerts-stack">
          {alert && (
            <p key={alert.n} className="note note-error paste-hint" role="alert">
              {alert.text}
            </p>
          )}
        </div>
      </div>
    </Modal>
  );
}

/* ---------- a batch in Downloads ---------- */

/** "12 files · 5 completed · 2 downloading · 5 queued": what is not zero. */
export function batchCounts(d: Download): string {
  const b = d.batch;
  if (!b) return "";
  const c = t.batch.counts;
  return [
    t.batch.files(b.count),
    b.completed ? c.completed(b.completed) : null,
    b.downloading ? c.downloading(b.downloading) : null,
    b.queued ? c.queued(b.queued) : null,
    b.paused ? c.paused(b.paused) : null,
    b.failed ? c.failed(b.failed) : null,
    b.cancelled ? c.cancelled(b.cancelled) : null,
  ]
    .filter(Boolean)
    .join(" · ");
}

/** The word for where a batch stands, finer than its status. */
export function batchState(d: Download): { word: string; tone: "ok" | "err" | "warn" | null } {
  const b = d.batch!;
  if (d.status === "completed") return d.error ? { word: t.batch.completedWithErrors, tone: "warn" } : { word: t.common.status.completed, tone: "ok" };
  if (d.status === "failed") return { word: b.cancelled === b.count ? t.batch.cancelled : t.common.status.failed, tone: "err" };
  if (d.status === "queued") return { word: b.completed + b.failed + b.cancelled > 0 || d.downloaded > 0 ? t.batch.waiting : t.batch.preparing, tone: null };
  return { word: STATUS_LABEL[d.status], tone: null };
}

/** A batch's line in the list: where it stands, its files, how far. */
export function BatchMeta({ d }: { d: Download }) {
  const { word, tone } = batchState(d);
  const percent = percentOf(d);
  const amount = d.total ? `${formatBytes(d.downloaded)} / ${formatBytes(d.total)}` : d.downloaded > 0 ? formatBytes(d.downloaded) : null;
  const facts = [
    batchCounts(d),
    d.status !== "completed" && percent != null ? pct(percent) : null,
    amount,
    d.status === "downloading" ? formatSpeed(d.speed) : null,
  ].filter(Boolean);
  return (
    <>
      <span className={tone ? `tone-${tone}` : undefined}>{word}</span> · {facts.join(" · ")}
    </>
  );
}

/** A batch's tile: a stack of pages. */
export function BatchTile({ large }: { large?: boolean }) {
  return (
    <span className={`file-icon batch-tile${large ? " file-icon-lg" : ""}`} aria-hidden="true">
      <Layers size={large ? 26 : 18} strokeWidth={2} />
    </span>
  );
}

/* ---------- a batch's details ---------- */

type ItemFilter = "all" | "active" | "done" | "problems";

const ITEM_FILTER: Record<ItemFilter, (d: Download) => boolean> = {
  all: () => true,
  active: (d) => d.status === "downloading" || d.status === "connecting" || d.status === "queued" || d.status === "paused",
  done: (d) => d.status === "completed",
  problems: (d) => d.status === "failed",
};

/** A download of a batch: name, link, size, progress, speed, status, its error, its own controls. */
function BatchItem({ d, place }: { d: Download; place: number }) {
  const { actions } = useApp();
  const percent = percentOf(d);
  const fail = (e: unknown) => actions.toast(String(e), "error");
  const running = d.status === "downloading" || d.status === "connecting";
  const state = d.cancelled ? t.batch.cancelled : STATUS_LABEL[d.status];
  const size =
    d.status === "completed"
      ? d.total != null
        ? formatBytes(d.total)
        : ""
      : d.total
        ? `${formatBytes(d.downloaded)} / ${formatBytes(d.total)}`
        : d.downloaded > 0
          ? formatBytes(d.downloaded)
          : d.status === "failed"
            ? ""
            : t.batch.item.unknownSize;
  return (
    <li className={`batch-item is-${d.cancelled ? "cancelled" : d.status}`}>
      <span className="batch-item-index">{place}</span>
      <LinkThumb url={d.url} picture={d.thumbnail} />
      <div className="batch-item-main">
        <div className="batch-item-top">
          <span className="batch-item-name" title={d.filename}>{d.filename}</span>
          <span className={`batch-item-state status-${d.status}`}>
            {state}
            {percent != null && d.status !== "completed" && d.status !== "failed" ? ` · ${pct(percent)}` : ""}
          </span>
        </div>
        <div className="batch-item-line">
          <span className="batch-item-url selectable" title={d.url}>{d.url}</span>
          <span className="batch-item-size">
            {size}
            {running && d.speed > 0 ? ` · ${formatSpeed(d.speed)}` : ""}
          </span>
        </div>
        {(running || d.status === "paused" || (d.status === "queued" && d.downloaded > 0)) && (
          <div className={`bar bar-${d.status}${percent == null && running ? " bar-busy" : ""}`}>
            {percent != null && <div className="fill" style={{ width: `${percent}%` }} />}
          </div>
        )}
        {d.status === "failed" && d.error && !d.cancelled && <p className="batch-item-error">{d.error}</p>}
      </div>
      <div className="batch-item-actions">
        {(running || d.status === "queued") && <IconBtn icon={Pause} label={t.batch.item.pause} onClick={() => api.pause(d.id).catch(fail)} />}
        {d.status === "paused" && <IconBtn icon={Play} label={t.batch.item.resume} onClick={() => api.resume(d.id).catch(fail)} />}
        {d.status === "failed" && <IconBtn icon={RotateCw} label={t.batch.item.retry} onClick={() => api.resume(d.id).catch(fail)} />}
        {d.status !== "completed" && d.status !== "failed" && (
          <IconBtn icon={X} label={t.batch.item.cancel} onClick={() => api.cancelItem(d.id).catch(fail)} />
        )}
        {d.status === "completed" && (
          <>
            <IconBtn icon={SquareArrowOutUpRight} label={t.batch.item.openFile} onClick={() => actions.open(d)} />
            <IconBtn icon={FolderOpen} label={t.batch.item.openFolder} onClick={() => actions.reveal(d)} />
          </>
        )}
      </div>
    </li>
  );
}

/** The details of a batch, opened from its row. */
export function BatchDetail({ d, onClose }: { d: Download; onClose: () => void }) {
  const { batchItems, actions } = useApp();
  const items = batchItems.get(d.id) ?? [];
  const [filter, setFilter] = useState<ItemFilter>("all");
  const [query, setQuery] = useState("");
  const [shown, setShown] = useState(ROWS_SHOWN);
  const b = d.batch!;
  const { word, tone } = batchState(d);
  const percent = percentOf(d);
  const fail = (e: unknown) => actions.toast(String(e), "error");
  const eta = d.status === "downloading" && d.total && d.speed > 0 ? formatEta((d.total - d.downloaded) / d.speed) : "";
  const q = query.trim().toLowerCase();
  const listed = items.filter(
    (item) => ITEM_FILTER[filter](item) && (!q || item.filename.toLowerCase().includes(q) || item.url.toLowerCase().includes(q)),
  );
  const order = new Map(items.map((item, i) => [item.id, i + 1]));
  const canPauseAll = b.downloading + b.queued > 0;
  const canResumeAll = b.paused > 0;
  const canRetry = b.failed + b.cancelled > 0;
  const finished = d.status === "completed" || d.status === "failed";

  return (
    <Modal
      wide
      bare
      title={t.batch.detailTitle(d.filename)}
      onClose={onClose}
      footer={
        <div className="batch-parent-actions">
          <button type="button" className="btn" disabled={!canPauseAll} onClick={() => api.pause(d.id).catch(fail)}>
            <Pause size={14} strokeWidth={0} fill="currentColor" aria-hidden="true" /> {t.batch.pauseAll}
          </button>
          <button type="button" className="btn" disabled={!canResumeAll} onClick={() => api.resume(d.id).catch(fail)}>
            <Play size={14} strokeWidth={0} fill="currentColor" aria-hidden="true" /> {t.batch.resumeAll}
          </button>
          <button type="button" className="btn" disabled={!canRetry} onClick={() => api.retryFailed(d.id).catch(fail)}>
            <RotateCw size={14} strokeWidth={2.2} aria-hidden="true" /> {t.batch.retryFailed}
          </button>
          <button type="button" className="btn" onClick={() => api.showFolder(d.id).catch(fail)}>
            <FolderOpen size={15} strokeWidth={2} aria-hidden="true" /> {t.batch.openFolder}
          </button>
          {(d.batch?.completed ?? 0) > 0 && <SumsButton id={d.id} />}
          <span className="spacer" />
          <button type="button" className="btn btn-danger" onClick={() => actions.remove([d.id])}>
            {finished ? <Trash2 size={15} strokeWidth={2} aria-hidden="true" /> : <X size={15} strokeWidth={2} aria-hidden="true" />}{" "}
            {finished ? t.detail.buttons.remove : t.batch.cancelBatch}
          </button>
        </div>
      }
    >
      <div className="detail-head">
        <BatchTile large />
        <div className="detail-title">
          <h2 title={d.filename}>{d.filename}</h2>
          <p className={`detail-headline${tone ? ` tone-${tone}` : ""}`}>
            {word} · {batchCounts(d)}
          </p>
          <div className="detail-progress">
            <div className={`bar bar-tall bar-${d.status}`} role="progressbar" aria-label={t.batch.overall} aria-valuemin={0} aria-valuemax={100} aria-valuenow={percent ?? 0}>
              <div className="fill" style={{ width: `${percent ?? 0}%` }} />
            </div>
            <span>{[percent != null ? pct(percent) : "", eta].filter(Boolean).join(" · ")}</span>
          </div>
        </div>
      </div>

      {d.error && d.status !== "completed" && (
        <div className="callout callout-error">
          <p>{d.error}</p>
        </div>
      )}

      <dl className="facts batch-facts">
        <div className="fact">
          <dt>{t.batch.downloaded}</dt>
          <dd>{d.total ? `${formatBytes(d.downloaded)} / ${formatBytes(d.total)}` : formatBytes(d.downloaded)}</dd>
        </div>
        <div className="fact">
          <dt>{t.batch.speed}</dt>
          <dd>{d.status === "downloading" ? formatSpeed(d.speed) : "—"}</dd>
        </div>
        <div className="fact">
          <dt>{t.batch.folder}</dt>
          <dd className="selectable break">{d.path}</dd>
        </div>
        <div className="fact">
          <dt>{d.finished_at ? t.batch.finished : t.batch.created}</dt>
          <dd>{formatDate(d.finished_at ?? d.created_at)}</dd>
        </div>
      </dl>
      {d.status === "downloading" && b.current && <p className="batch-now">{t.batch.now(b.current)}</p>}

      <div className="batch-toolbar">
        <div className="batch-filter" role="tablist" aria-label={t.batch.links}>
          {(Object.keys(ITEM_FILTER) as ItemFilter[]).map((f) => (
            <button
              key={f}
              type="button"
              role="tab"
              aria-selected={filter === f}
              className={filter === f ? "is-on" : undefined}
              onClick={() => {
                setFilter(f);
                setShown(ROWS_SHOWN);
              }}
            >
              {t.batch.filter[f]} <span>{items.filter(ITEM_FILTER[f]).length}</span>
            </button>
          ))}
        </div>
        <span className="spacer" />
        {items.length > 8 && (
          <label className="search">
            <Search size={15} strokeWidth={2} aria-hidden="true" />
            <input type="text" value={query} onChange={(e) => setQuery(e.target.value)} placeholder={t.batch.search} aria-label={t.batch.search} spellCheck={false} />
          </label>
        )}
      </div>
      <ol className="batch-items" aria-label={t.batch.links}>
        {listed.slice(0, shown).map((item) => (
          <BatchItem key={item.id} d={item} place={order.get(item.id) ?? 0} />
        ))}
      </ol>
      {listed.length > shown && (
        <p className="batch-more">
          {t.batch.shownOf(shown, listed.length)}{" "}
          <button type="button" className="link" onClick={() => setShown((n) => n + ROWS_STEP)}>
            {t.batch.showMore(Math.min(ROWS_STEP, listed.length - shown))}
          </button>
        </p>
      )}
    </Modal>
  );
}
