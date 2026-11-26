import { useCallback, useEffect, useMemo, useRef, useState, type MouseEvent as ReactMouseEvent } from "react";
import {
  Download as DownloadGlyph,
  FolderOpen,
  Globe,
  House,
  Info,
  KeyRound,
  Minus,
  Moon,
  Pause,
  Play,
  Plus,
  ScanSearch,
  Settings as SettingsGlyph,
  Square,
  Sun,
  X,
  type LucideIcon,
} from "lucide-react";
import {
  api,
  canPause,
  canResume,
  isRunning,
  isTauri,
  isTorrentSource,
  loadPref,
  looksLikeUrl,
  QUALITIES,
  qualityOptions,
  savePref,
  type Download,
  type Settings,
} from "./api";
import { DetailDialog } from "./Detail";
import { AddDialog, Palette, ScanDialog, TorrentDialog, type Command } from "./dialogs";
import {
  AppContext,
  downloadMenu,
  Logo,
  Menu,
  Modal,
  Toasts,
  type Actions,
  type MenuItem,
  type ProgressStyle,
  type Toast,
} from "./ui";

/** A question asked in the app's own dialog: yes, no, or an optional third answer. */
interface Question {
  title: string;
  message: string;
  detail?: string;
  /** State of the thing asked about, shown as a small badge. */
  status?: string;
  statusTone?: "active" | "done" | "failed";
  okLabel: string;
  cancelLabel: string;
  /** A third, quieter answer between the two. */
  altLabel?: string;
  /** "danger" (the default) for destructive answers, "primary" for ordinary ones. */
  tone?: "danger" | "primary";
  resolve: (answer: boolean | "alt") => void;
}
import {
  AboutDialog,
  BrowserView,
  DownloadsView,
  HomeView,
  NAVS,
  SettingsView,
  SignInView,
  type Density,
  type Nav,
  type Theme,
} from "./views";
import "./styles.css";

const HISTORY_LENGTH = 60;
const MAIN_NAV: { nav: Nav; label: string; icon: LucideIcon }[] = [
  { nav: "home", label: "Home", icon: House },
  { nav: "downloads", label: "Downloads", icon: DownloadGlyph },
  { nav: "browser", label: "Browser", icon: Globe },
  { nav: "signin", label: "Sign in", icon: KeyRound },
  { nav: "settings", label: "Settings", icon: SettingsGlyph },
];

function isTyping(target: EventTarget | null) {
  const el = target as HTMLElement | null;
  return !!el && (["INPUT", "TEXTAREA", "SELECT"].includes(el.tagName) || el.isContentEditable);
}

export default function App() {
  const [downloads, setDownloads] = useState<Download[]>([]);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [nav, setNavState] = useState<Nav>(() => loadPref<Nav>("nav", "home", NAVS));
  const [selection, setSelection] = useState<Set<number>>(new Set());
  const [inspected, setInspected] = useState<number | null>(null);
  const [menu, setMenu] = useState<{ items: MenuItem[]; x: number; y: number } | null>(null);
  const [add, setAdd] = useState<{ url: string; mode: "download" | "scan" } | null>(null);
  const [scanUrl, setScanUrl] = useState<string | null>(null);
  const [torrentSource, setTorrentSource] = useState<string | null>(null);
  const qualityRef = useRef("best");
  const [aboutOpen, setAboutOpen] = useState(false);
  const [question, setQuestion] = useState<Question | null>(null);
  /** The link last copied anywhere on the computer, handed to the Home box. */
  const [copied, setCopied] = useState<{ text: string; n: number } | null>(null);
  const [clipWatch, setClipWatch] = useState(() => loadPref("clipwatch", "on", ["on", "off"]) === "on");
  /** A profile or playlist asks for its folder each time, when the user wants that. */
  const [askFolder, setAskFolder] = useState(() => loadPref("askfolder", "off", ["on", "off"]) === "on");
  const askFolderRef = useRef(askFolder);
  askFolderRef.current = askFolder;
  const saveDirRef = useRef<string | undefined>(undefined);
  /** What the app itself last put on the clipboard, which is not a new link. */
  const ownCopy = useRef("");
  /** Downloads started from the link box that are still finding their video. */
  const fromLinkBox = useRef<Set<number>>(new Set());
  /** A page to open as soon as the Browser page shows. */
  const [browseStart, setBrowseStart] = useState<{ url: string; fetch: boolean } | null>(null);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [toasts, setToasts] = useState<Toast[]>([]);
  const [theme, setTheme] = useState<Theme>(() =>
    loadPref<Theme>("theme", "light", ["system", "light", "dark"]),
  );
  const [progressStyle, setProgressStyle] = useState<ProgressStyle>(() =>
    loadPref<ProgressStyle>("progress", "single", ["single", "threads"]),
  );
  const [density, setDensity] = useState<Density>(() =>
    loadPref<Density>("density", "comfortable", ["comfortable", "compact"]),
  );
  const [quality, setQualityState] = useState(() =>
    loadPref("quality", "best", QUALITIES.map((q) => q.value)),
  );
  const [systemDark, setSystemDark] = useState(
    () => window.matchMedia("(prefers-color-scheme: dark)").matches,
  );

  qualityRef.current = quality;
  saveDirRef.current = settings?.download_dir;
  const history = useRef(new Map<number, number[]>());
  const anchor = useRef<number | null>(null);
  const toastId = useRef(0);
  const downloadsRef = useRef(downloads);
  downloadsRef.current = downloads;

  /* ---------- data ---------- */

  const receive = useCallback((list: Download[]) => {
    const h = history.current;
    const alive = new Set(list.map((d) => d.id));
    for (const id of h.keys()) if (!alive.has(id)) h.delete(id);
    for (const d of list) {
      if (!isRunning(d)) continue;
      const samples = h.get(d.id) ?? [];
      samples.push(d.speed);
      if (samples.length > HISTORY_LENGTH) samples.shift();
      h.set(d.id, samples);
    }
    setDownloads(list);
  }, []);

  useEffect(() => {
    api.list().then(receive);
    api.getSettings().then(setSettings);
    const unlisten = api.onChange(receive);
    return () => {
      unlisten.then((f) => f());
    };
  }, [receive]);

  /* ---------- appearance ---------- */

  useEffect(() => {
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const onChange = (e: MediaQueryListEvent) => setSystemDark(e.matches);
    mq.addEventListener("change", onChange);
    return () => mq.removeEventListener("change", onChange);
  }, []);

  const dark = theme === "dark" || (theme === "system" && systemDark);
  useEffect(() => {
    document.documentElement.dataset.theme = dark ? "dark" : "light";
    document.documentElement.dataset.density = density;
  }, [dark, density]);

  const chooseTheme = useCallback((t: Theme) => {
    setTheme(t);
    savePref("theme", t);
  }, []);

  const setQuality = useCallback((q: string) => {
    setQualityState(q);
    savePref("quality", q);
  }, []);

  /* ---------- actions ---------- */

  const toast = useCallback((message: string, tone: "error" | "info" = "info") => {
    const id = ++toastId.current;
    setToasts((list) => [...list.slice(-3), { id, message, tone }]);
    window.setTimeout(
      () => setToasts((list) => list.filter((t) => t.id !== id)),
      tone === "error" ? 8000 : 3000,
    );
  }, []);

  const setNav = useCallback((next: Nav) => {
    setNavState(next);
    savePref("nav", next);
    setSelection(new Set());
    anchor.current = null;
  }, []);

  const ask = useCallback(
    (q: Omit<Question, "resolve">) =>
      new Promise<boolean | "alt">((resolve) => setQuestion({ ...q, resolve })),
    [],
  );
  const answer = useCallback((ok: boolean | "alt") => {
    setQuestion((q) => {
      q?.resolve(ok);
      return null;
    });
  }, []);

  const actions = useMemo<Actions>(() => {
    const fail = (e: unknown) => toast(String(e), "error");
    const inspect = (id: number) => {
      setSelection(new Set([id]));
      anchor.current = id;
      setInspected(id);
    };
    // Tells the user when the same link is already in the list (running,
    // paused, finished or failed) before a second copy starts.
    const checkDuplicate = async (link: string): Promise<"go" | "show" | "cancel"> => {
      const url = link.trim();
      const twins = downloadsRef.current.filter((d) => d.url === url);
      // The one still on its way matters most, then a failed one, then a finished one.
      const rank = (d: Download) => (d.status === "completed" ? 2 : d.status === "failed" ? 1 : 0);
      const twin = [...twins].sort((a, b) => rank(a) - rank(b))[0];
      if (!twin) return "go";
      const percent =
        twin.total && twin.total > 0 ? `, ${Math.floor((twin.downloaded / twin.total) * 100)}%` : "";
      const status =
        twin.status === "completed"
          ? "Completed"
          : twin.status === "failed"
            ? "Failed"
            : twin.status === "paused"
              ? `Paused${percent}`
              : twin.status === "queued"
                ? "Waiting to start"
                : `Downloading${percent}`;
      const statusTone =
        twin.status === "completed" ? "done" : twin.status === "failed" ? "failed" : "active";
      const torrent = isTorrentSource(url);
      const what = torrent ? "torrent" : "file";
      const title =
        twin.status === "completed"
          ? `You already downloaded this ${what}`
          : twin.status === "failed"
            ? `This ${what} is already in your downloads`
            : `This ${what} is already downloading`;
      const reply = await ask({
        title,
        message: twin.filename,
        status,
        statusTone,
        detail: torrent
          ? "Downloading a copy saves it again in a second, separate folder. The first one stays as it is."
          : "Downloading a copy saves a second, separate file. The first one stays as it is.",
        okLabel: "Download a copy",
        altLabel: "Show it",
        cancelLabel: "Cancel",
        tone: "primary",
      });
      if (reply === "alt") {
        inspect(twin.id);
        return "show";
      }
      return reply ? "go" : "cancel";
    };
    const openTorrent = async (source: string) => {
      if ((await checkDuplicate(source)) === "go") setTorrentSource(source.trim());
    };
    return {
      toast,
      checkDuplicate,
      pause: (ids) => ids.forEach((id) => api.pause(id).catch(fail)),
      resume: (ids) => ids.forEach((id) => api.resume(id).catch(fail)),
      remove: async (ids) => {
        const unfinished = downloadsRef.current.filter(
          (d) => ids.includes(d.id) && d.status !== "completed",
        );
        if (unfinished.length > 0) {
          const what =
            unfinished.length === 1
              ? unfinished[0].filename
              : `${unfinished.length} unfinished downloads`;
          const ok = await ask({
            title: unfinished.length === 1 ? "Cancel this download?" : "Cancel these downloads?",
            message: what,
            detail: unfinished.every((d) => d.profile)
              ? "Photos and videos already downloaded stay in the folder."
              : "The part already downloaded will be deleted.",
            okLabel: "Cancel download",
            cancelLabel: "Keep downloading",
          });
          if (!ok) return;
        }
        ids.forEach((id) => api.remove(id, false).catch(fail));
        setSelection((sel) => new Set([...sel].filter((id) => !ids.includes(id))));
        setInspected((id) => (id != null && ids.includes(id) ? null : id));
      },
      deleteFile: async (d) => {
        const ok = await ask({
          title: "Delete this file from disk?",
          message: d.filename,
          detail: "This cannot be undone.",
          okLabel: "Delete file",
          cancelLabel: "Keep file",
        });
        if (!ok) return;
        api.remove(d.id, true).catch(fail);
        setInspected((id) => (id === d.id ? null : id));
      },
      open: (d) => api.open(d.id).catch(fail),
      reveal: (d) => api.reveal(d.id).catch(fail),
      copy: (text, what) => {
        ownCopy.current = text.trim();
        return navigator.clipboard
          .writeText(text)
          .then(() => toast(`${what} copied`))
          .catch(() => toast(`Could not copy the ${what.toLowerCase()}`, "error"));
      },
      scan: (url) => setScanUrl(url),
      inspect,
      openAdd: (url, mode) => setAdd({ url: url ?? "", mode: mode ?? "download" }),
      openTorrent,
      browse: (url, fetch = false) => {
        setBrowseStart({ url, fetch });
        setInspected(null);
        setNavState("browser");
        savePref("nav", "browser");
      },
      submitLink: async (text, media) => {
        const link = text.trim();
        if (!link) return false;
        if (isTorrentSource(link)) {
          await openTorrent(link);
          return true;
        }
        const duplicate = await checkDuplicate(link);
        if (duplicate !== "go") return duplicate === "show";
        try {
          // A profile or playlist is a whole folder: the user may want to say where it goes.
          let dir: string | undefined;
          if (askFolderRef.current && (await api.profileOf(link).catch(() => null))) {
            const picked = await api.pickFolder(saveDirRef.current);
            // Closing the folder window without choosing leaves the link in the box.
            if (!picked) return false;
            dir = picked;
          }
          const id = await api.add({ url: link, media: media ?? qualityOptions(qualityRef.current), dir });
          fromLinkBox.current.add(id);
          setNavState("downloads");
          savePref("nav", "downloads");
          return true;
        } catch (e) {
          fail(e);
          return false;
        }
      },
    };
  }, [toast, ask]);

  // Some sites cannot be downloaded from the link alone: the video has to be
  // caught while it plays. A link pasted in the link box that turns out to be
  // one of those opens on the Browser page by itself, instead of sitting in
  // the list as a failure.
  useEffect(() => {
    for (const id of [...fromLinkBox.current]) {
      const d = downloads.find((item) => item.id === id);
      if (!d || d.status === "completed" || d.downloaded > 0) {
        fromLinkBox.current.delete(id);
      } else if (d.status === "failed") {
        fromLinkBox.current.delete(id);
        if (d.error?.includes("Browser page") && /^https?:/i.test(d.url)) {
          api.remove(id, false).catch(() => {});
          // The app opens the page and starts the player so the video's
          // qualities show up; which one to download is the user's choice.
          actions.browse(d.url, true);
          toast("This video has to be picked on its page. Choose a quality there");
        }
      }
    }
  }, [downloads, actions, toast]);

  const pauseAll = useCallback(
    () => actions.pause(downloadsRef.current.filter(canPause).map((d) => d.id)),
    [actions],
  );
  const resumeAll = useCallback(
    () => actions.resume(downloadsRef.current.filter((d) => d.status === "paused").map((d) => d.id)),
    [actions],
  );

  const onSelect = useCallback((id: number, e: ReactMouseEvent, visible: number[]) => {
    if (e.shiftKey && anchor.current != null && visible.includes(anchor.current)) {
      const a = visible.indexOf(anchor.current);
      const b = visible.indexOf(id);
      setSelection(new Set(visible.slice(Math.min(a, b), Math.max(a, b) + 1)));
      return;
    }
    anchor.current = id;
    if (e.ctrlKey || e.metaKey) {
      setSelection((sel) => {
        const next = new Set(sel);
        if (next.has(id)) next.delete(id);
        else next.add(id);
        return next;
      });
      return;
    }
    // A plain click opens the detailed view.
    setSelection(new Set([id]));
    setInspected(id);
  }, []);

  const onMenu = useCallback(
    (d: Download, x: number, y: number) => {
      const many = selection.size > 1 && selection.has(d.id);
      if (!many) {
        setMenu({ items: downloadMenu(d, actions), x, y });
        return;
      }
      const chosen = downloadsRef.current.filter((item) => selection.has(item.id));
      const items: MenuItem[] = [];
      if (chosen.some(canPause)) {
        items.push({
          label: `Pause ${chosen.filter(canPause).length} downloads`,
          icon: Pause,
          onSelect: () => actions.pause(chosen.filter(canPause).map((c) => c.id)),
        });
      }
      if (chosen.some(canResume)) {
        items.push({
          label: `Resume ${chosen.filter(canResume).length} downloads`,
          icon: Play,
          onSelect: () => actions.resume(chosen.filter(canResume).map((c) => c.id)),
        });
      }
      if (items.length > 0) items.push("separator");
      items.push({
        label: `Remove ${chosen.length} from list`,
        icon: X,
        onSelect: () => actions.remove(chosen.map((c) => c.id)),
      });
      setMenu({ items, x, y });
    },
    [selection, actions],
  );

  /* ---------- keyboard and paste ---------- */

  const inspectedDownload = downloads.find((d) => d.id === inspected);
  const overlayOpen =
    add != null ||
    scanUrl != null ||
    torrentSource != null ||
    paletteOpen ||
    aboutOpen ||
    menu != null ||
    question != null ||
    inspectedDownload != null;

  // A link copied anywhere lands in the Home box, the way a download manager
  // picks links up. Open dialogs are left alone; the link waits on Home.
  const overlayNow = useRef(overlayOpen);
  overlayNow.current = overlayOpen;
  useEffect(() => {
    api.setClipboardWatch(clipWatch).catch(() => {});
    if (!clipWatch) return;
    const unlisten = api.onClipboardLink((link) => {
      const text = link.trim();
      if (!text || text === ownCopy.current) return;
      setCopied((last) => ({ text, n: (last?.n ?? 0) + 1 }));
      if (!overlayNow.current) setNav("home");
      toast("Copied link is ready on Home");
    });
    return () => {
      unlisten.then((f) => f());
    };
  }, [clipWatch, setNav, toast]);

  useEffect(() => {
    const onPaste = (e: ClipboardEvent) => {
      if (isTyping(e.target) || overlayOpen) return;
      const text = e.clipboardData?.getData("text") ?? "";
      if (isTorrentSource(text)) {
        e.preventDefault();
        actions.openTorrent(text);
      } else if (looksLikeUrl(text)) {
        e.preventDefault();
        setAdd({ url: text.trim(), mode: "download" });
      }
    };
    window.addEventListener("paste", onPaste);
    return () => window.removeEventListener("paste", onPaste);
  }, [overlayOpen, actions]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const ctrl = e.ctrlKey || e.metaKey;
      const key = e.key.toLowerCase();
      if (ctrl && key === "k") {
        e.preventDefault();
        setPaletteOpen((v) => !v);
        return;
      }
      if (overlayOpen) return;

      if (ctrl && e.shiftKey && key === "p") {
        e.preventDefault();
        pauseAll();
      } else if (ctrl && e.shiftKey && key === "r") {
        e.preventDefault();
        resumeAll();
      } else if (ctrl && key === "n") {
        e.preventDefault();
        setAdd({ url: "", mode: "download" });
      } else if (ctrl && key === ",") {
        e.preventDefault();
        setNav("settings");
      } else if (ctrl && (key === "1" || key === "2")) {
        e.preventDefault();
        setNav(key === "1" ? "home" : "downloads");
      } else if (isTyping(e.target)) {
        return;
      } else if (ctrl && key === "a" && nav === "downloads") {
        e.preventDefault();
        setSelection(new Set(downloadsRef.current.map((d) => d.id)));
      } else if (e.key === "Escape") {
        setSelection(new Set());
      } else if (selection.size > 0 && e.key === "Delete") {
        e.preventDefault();
        actions.remove([...selection]);
      } else if (selection.size > 0 && e.key === " " && (e.target as HTMLElement).tagName !== "BUTTON") {
        e.preventDefault();
        const chosen = downloadsRef.current.filter((d) => selection.has(d.id));
        if (chosen.some(canPause)) actions.pause(chosen.filter(canPause).map((d) => d.id));
        else actions.resume(chosen.filter(canResume).map((d) => d.id));
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [overlayOpen, nav, selection, actions, pauseAll, resumeAll, setNav]);

  // Dropping a .torrent file onto the window opens it.
  useEffect(() => {
    const unlisten = api.onFileDrop((paths) => {
      const torrent = paths.find((p) => /\.torrent$/i.test(p));
      if (torrent) actions.openTorrent(torrent);
      else if (paths.length > 0) toast("Only .torrent files can be dropped here", "error");
    });
    return () => {
      unlisten.then((f) => f());
    };
  }, [toast, actions]);

  /* ---------- commands ---------- */

  const commands = useMemo<Command[]>(
    () => [
      { id: "add", group: "Actions", label: "Add download", icon: Plus, shortcut: "Ctrl N", run: () => setAdd({ url: "", mode: "download" }) },
      { id: "scan", group: "Actions", label: "Find media on a web page", icon: ScanSearch, run: () => setAdd({ url: "", mode: "scan" }) },
      { id: "pause-all", group: "Actions", label: "Pause all downloads", icon: Pause, shortcut: "Ctrl Shift P", run: pauseAll },
      { id: "resume-all", group: "Actions", label: "Resume all downloads", icon: Play, shortcut: "Ctrl Shift R", run: resumeAll },
      {
        id: "folder",
        group: "Actions",
        label: "Open download folder",
        icon: FolderOpen,
        run: () => api.openDownloadDir().catch((e) => toast(String(e), "error")),
      },
      {
        id: "theme",
        group: "Actions",
        label: dark ? "Switch to light theme" : "Switch to dark theme",
        icon: dark ? Sun : Moon,
        run: () => chooseTheme(dark ? "light" : "dark"),
      },
      { id: "nav-home", group: "Go to", label: "Home", icon: House, shortcut: "Ctrl 1", run: () => setNav("home") },
      { id: "nav-downloads", group: "Go to", label: "Downloads", icon: DownloadGlyph, shortcut: "Ctrl 2", run: () => setNav("downloads") },
      { id: "nav-settings", group: "Go to", label: "Settings", icon: SettingsGlyph, shortcut: "Ctrl ,", run: () => setNav("settings") },
      { id: "about", group: "Go to", label: "About RushDownload", icon: Info, run: () => setAboutOpen(true) },
    ],
    [dark, pauseAll, resumeAll, chooseTheme, setNav, toast],
  );

  const context = { downloads, settings, history: history.current, actions, progressStyle };
  const rowHandlers = { selection, onSelect, onMenu };

  return (
    <AppContext.Provider value={context}>
      <div className="app">
        <nav className="sidebar" aria-label="Main">
          <div className="brand" data-tauri-drag-region>
            <Logo size={24} />
            <span>RushDownload</span>
          </div>

          <div className="nav-group">
            {MAIN_NAV.map((item) => (
              <button
                key={item.nav}
                type="button"
                className={`nav-item${nav === item.nav ? " is-active" : ""}`}
                aria-current={nav === item.nav ? "page" : undefined}
                aria-label={item.label}
                title={item.label}
                onClick={() => setNav(item.nav)}
              >
                <item.icon size={18} strokeWidth={2} aria-hidden="true" />
                <span>{item.label}</span>
              </button>
            ))}
          </div>

          <div className="nav-group nav-bottom">
            <button
              type="button"
              className="nav-item"
              role="switch"
              aria-checked={dark}
              aria-label="Dark Mode"
              title="Dark Mode"
              onClick={() => chooseTheme(dark ? "light" : "dark")}
            >
              <Moon size={18} strokeWidth={2} aria-hidden="true" />
              <span>Dark Mode</span>
              <span className="switch" aria-hidden="true" />
            </button>
            <button
              type="button"
              className="nav-item"
              aria-label="About"
              title="About"
              onClick={() => setAboutOpen(true)}
            >
              <Info size={18} strokeWidth={2} aria-hidden="true" />
              <span>About</span>
            </button>
          </div>
        </nav>

        <main className="workspace">
          <div className="titlebar" data-tauri-drag-region>
            {isTauri && (
              <div className="window-controls">
                <button type="button" aria-label="Minimize" title="Minimize" onClick={() => api.windowMinimize()}>
                  <Minus size={16} strokeWidth={2} aria-hidden="true" />
                </button>
                <button type="button" aria-label="Maximize" title="Maximize" onClick={() => api.windowToggleMaximize()}>
                  <Square size={13} strokeWidth={2} aria-hidden="true" />
                </button>
                <button type="button" className="is-close" aria-label="Close" title="Close" onClick={() => api.windowClose()}>
                  <X size={17} strokeWidth={2} aria-hidden="true" />
                </button>
              </div>
            )}
          </div>

          {nav === "home" && <HomeView quality={quality} go={setNav} copied={copied} />}
          {nav === "downloads" && (
            <DownloadsView {...rowHandlers} clearSelection={() => setSelection(new Set())} go={setNav} />
          )}
          {nav === "browser" && (
            <BrowserView quality={quality} go={setNav} start={browseStart} onStarted={() => setBrowseStart(null)} />
          )}
          {nav === "signin" && <SignInView />}
          {nav === "settings" && (
            <SettingsView
              theme={theme}
              onTheme={chooseTheme}
              density={density}
              onDensity={(d) => {
                setDensity(d);
                savePref("density", d);
              }}
              clipWatch={clipWatch}
              onClipWatch={(on) => {
                setClipWatch(on);
                savePref("clipwatch", on ? "on" : "off");
              }}
              askFolder={askFolder}
              onAskFolder={(on) => {
                setAskFolder(on);
                savePref("askfolder", on ? "on" : "off");
              }}
              progressStyle={progressStyle}
              onProgressStyle={(p) => {
                setProgressStyle(p);
                savePref("progress", p);
              }}
              quality={quality}
              onQuality={setQuality}
              onSettings={(s) => {
                setSettings(s);
                api.setSettings(s).catch((e) => toast(String(e), "error"));
              }}
            />
          )}
        </main>

        {inspectedDownload && (
          <DetailDialog
            d={inspectedDownload}
            onClose={() => {
              setInspected(null);
              // Looking at details is not a selection: leave nothing selected
              // behind, or a later Space or Delete would act on this download.
              setSelection(new Set());
              anchor.current = null;
            }}
          />
        )}
        {add && (
          <AddDialog
            mode={add.mode}
            initialUrl={add.url}
            quality={quality}
            onQuality={setQuality}
            onClose={() => setAdd(null)}
          />
        )}
        {torrentSource && (
          <TorrentDialog
            source={torrentSource}
            onClose={() => setTorrentSource(null)}
            onAdded={() => setNav("downloads")}
          />
        )}
        {scanUrl && (
          <ScanDialog url={scanUrl} media={qualityOptions(quality)} onClose={() => setScanUrl(null)} />
        )}
        {aboutOpen && <AboutDialog onClose={() => setAboutOpen(false)} />}
        {paletteOpen && <Palette commands={commands} onClose={() => setPaletteOpen(false)} />}
        {menu && <Menu {...menu} onClose={() => setMenu(null)} />}
        {question && (
          <Modal
            title={question.title}
            onClose={() => answer(false)}
            footer={
              <>
                <span className="spacer" />
                <button type="button" className="btn" data-autofocus onClick={() => answer(false)}>
                  {question.cancelLabel}
                </button>
                {question.altLabel && (
                  <button type="button" className="btn" onClick={() => answer("alt")}>
                    {question.altLabel}
                  </button>
                )}
                <button
                  type="button"
                  className={`btn ${question.tone === "primary" ? "btn-primary" : "btn-danger-solid"}`}
                  onClick={() => answer(true)}
                >
                  {question.okLabel}
                </button>
              </>
            }
          >
            <p className="confirm-name">{question.message}</p>
            {question.status && (
              <p className={`confirm-status is-${question.statusTone ?? "active"}`}>
                <span aria-hidden="true" />
                {question.status}
              </p>
            )}
            {question.detail && <p className="confirm-detail">{question.detail}</p>}
          </Modal>
        )}
        <Toasts toasts={toasts} dismiss={(id) => setToasts((list) => list.filter((t) => t.id !== id))} />
      </div>
    </AppContext.Provider>
  );
}
