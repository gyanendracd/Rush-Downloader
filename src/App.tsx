import { useCallback, useEffect, useMemo, useRef, useState, type MouseEvent as ReactMouseEvent } from "react";
import {
  CalendarClock,
  Download as DownloadGlyph,
  FolderOpen,
  Globe,
  History as HistoryGlyph,
  House,
  Info,
  KeyRound,
  Layers,
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
  displayName,
  isRunning,
  isTauri,
  isTorrentSource,
  loadPref,
  looksLikeUrl,
  QUALITIES,
  qualityOptions,
  savePref,
  type AfterAllNotice,
  type AppPrefs,
  type Download,
  type Schedule,
  type Settings,
} from "./api";
import { ScheduleDialog, SchedulerView } from "./Scheduler";
import { DetailDialog } from "./Detail";
import { PairPrompt } from "./Bridge";
import { BatchDetail, BatchDialog } from "./Batch";
import { Ambient } from "./Ambient";
import { AddDialog, Palette, ScanDialog, siteBlockedText, TorrentDialog, type Command } from "./dialogs";
import {
  AppContext,
  downloadMenu,
  Logo,
  Menu,
  Modal,
  RefreshLinkDialog,
  SpeedLimitDialog,
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
  durationText,
  DownloadsView,
  HomeView,
  NAVS,
  SettingsView,
  SignInView,
  TransferSummary,
  type Density,
  type Nav,
  type SettingsSection,
  type Theme,
} from "./views";
import { HistoryView } from "./History";
import { locale, t } from "./i18n";
import { useLocale } from "./i18n/react";
import "./styles.css";

const HISTORY_LENGTH = 60;
/** How long a page takes to fade out before the next one is shown; the stylesheet uses the same time. */
const PAGE_FADE_OUT = 170;
/** The sidebar's pages; each label is read from `t.app.nav` when drawn. */
const MAIN_NAV: { nav: Nav; icon: LucideIcon }[] = [
  { nav: "home", icon: House },
  { nav: "downloads", icon: DownloadGlyph },
  { nav: "scheduler", icon: CalendarClock },
  { nav: "history", icon: HistoryGlyph },
  { nav: "browser", icon: Globe },
  { nav: "signin", icon: KeyRound },
  { nav: "settings", icon: SettingsGlyph },
];

function isTyping(target: EventTarget | null) {
  const el = target as HTMLElement | null;
  return !!el && (["INPUT", "TEXTAREA", "SELECT"].includes(el.tagName) || el.isContentEditable);
}

export default function App() {
  // Renders the whole interface again when the language changes in Settings.
  useLocale();
  /** The list as it shows: every download except those inside a batch. */
  const [downloads, setDownloads] = useState<Download[]>([]);
  /** The downloads inside each batch, by the batch's id, in order. */
  const [batchItems, setBatchItems] = useState<Map<number, Download[]>>(new Map());
  /** The Batch Download dialog is open. */
  const [batchOpen, setBatchOpen] = useState(false);
  /** Where files dropped onto the window go while a dialog wants them. */
  const dropTo = useRef<((paths: string[]) => void) | null>(null);
  const [settings, setSettings] = useState<Settings | null>(null);
  /** Starting with the system, the tray, notifications, graphics. */
  const [appPrefs, setAppPrefs] = useState<AppPrefs | null>(null);
  const appPrefsRef = useRef<AppPrefs | null>(null);
  appPrefsRef.current = appPrefs;
  /** Each download's status at the last update, to notice one finishing. */
  const lastStatus = useRef<Map<number, Download["status"]> | null>(null);
  /** Each download's checksum outcome last seen, to tell when one fails. */
  const lastCheck = useRef<Map<number, string> | null>(null);
  const [nav, setNavState] = useState<Nav>(() => loadPref<Nav>("nav", "home", NAVS));
  /** The page on screen. It trails `nav` by a moment: the old page fades
   *  out first, then this changes and the new page fades in. */
  const [shownNav, setShownNav] = useState<Nav>(nav);
  /** The tab of Settings that shows; kept here so it is still open on the way back. */
  const [settingsSection, setSettingsSection] = useState<SettingsSection>("general");
  const leaving = nav !== shownNav;
  useEffect(() => {
    if (nav === shownNav) return;
    // With reduced motion there is no fade to wait for.
    const wait = window.matchMedia("(prefers-reduced-motion: reduce)").matches ? 0 : PAGE_FADE_OUT;
    const timer = window.setTimeout(() => setShownNav(nav), wait);
    return () => window.clearTimeout(timer);
  }, [nav, shownNav]);
  const [selection, setSelection] = useState<Set<number>>(new Set());
  const [inspected, setInspected] = useState<number | null>(null);
  const [menu, setMenu] = useState<{ items: MenuItem[]; x: number; y: number } | null>(null);
  const [add, setAdd] = useState<{ url: string; mode: "download" | "scan" } | null>(null);
  const [scanUrl, setScanUrl] = useState<string | null>(null);
  const [torrentSource, setTorrentSource] = useState<string | null>(null);
  /** A share link or project page whose files are being picked. */
  const [siteSource, setSiteSource] = useState<string | null>(null);
  const qualityRef = useRef("best");
  const [aboutOpen, setAboutOpen] = useState(false);
  /** Schedules, as the engine keeps them; the dialog making or changing one. */
  const [schedules, setSchedules] = useState<Schedule[]>([]);
  const [scheduling, setScheduling] = useState<{ schedule?: Schedule; picked?: number[] } | null>(null);
  /** The download being given a new link. */
  const [relinking, setRelinking] = useState<Download | null>(null);
  /** The download whose own speed limit is being set. */
  const [limiting, setLimiting] = useState<Download | null>(null);
  /** The countdown before the action after all downloads, while it runs. */
  const [afterAll, setAfterAll] = useState<AfterAllNotice | null>(null);
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
  /** A site whose sign-in page the Sign in page opens as it appears. */
  const [signInStart, setSignInStart] = useState<string | null>(null);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [toasts, setToasts] = useState<Toast[]>([]);
  const [theme, setTheme] = useState<Theme>(() =>
    loadPref<Theme>("theme", "dark", ["system", "light", "dark"]),
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

  const receive = useCallback((everything: Download[]) => {
    // A batch's own downloads are listed in its details, not as rows.
    const items = new Map<number, Download[]>();
    const list: Download[] = [];
    for (const d of everything) {
      if (d.parent == null) list.push(d);
      else items.set(d.parent, [...(items.get(d.parent) ?? []), d]);
    }
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
    // A system notification when a download finishes or fails, while the
    // app is not in front (in front, the list shows it already).
    const was = lastStatus.current;
    if (was && appPrefsRef.current?.notifications && !document.hasFocus()) {
      for (const d of list) {
        const before = was.get(d.id);
        if (before == null || before === d.status) continue;
        if (d.status === "completed") api.notify(t.app.notify.done, displayName(d)).catch(() => {});
        else if (d.status === "failed") api.notify(t.app.notify.failed, displayName(d)).catch(() => {});
      }
    }
    lastStatus.current = new Map(list.map((d) => [d.id, d.status]));
    // A file that does not match its checksum is always told, in front or not.
    const checked = lastCheck.current;
    if (checked) {
      for (const d of everything) {
        const state = d.checksum?.state;
        if (state !== "failed" || checked.get(d.id) === "failed") continue;
        toast(t.checksum.toastFailed(displayName(d)), "error");
        if (appPrefsRef.current?.notifications && !document.hasFocus()) {
          api.notify(t.checksum.notifyFailed, displayName(d)).catch(() => {});
        }
      }
    }
    lastCheck.current = new Map(everything.map((d) => [d.id, d.checksum?.state ?? ""]));
    setDownloads(list);
    setBatchItems(items);
  }, []);

  useEffect(() => {
    api.list().then(receive);
    api.getSettings().then(setSettings);
    api.getAppPrefs().then(setAppPrefs).catch(() => {});
    api.schedules().then(setSchedules).catch(() => {});
    const unlisten = api.onChange(receive);
    const unlistenSchedules = api.onSchedules(setSchedules);
    return () => {
      unlisten.then((f) => f());
      unlistenSchedules.then((f) => f());
    };
  }, [receive]);

  // The tray menu speaks the interface's language.
  useEffect(() => {
    if (isTauri) api.setTrayText(t.app.tray.show, t.app.tray.quit).catch(() => {});
  }, [locale]);

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

  const chooseTheme = useCallback((next: Theme) => {
    setTheme(next);
    savePref("theme", next);
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
      () => setToasts((list) => list.filter((item) => item.id !== id)),
      tone === "error" ? 8000 : 3000,
    );
  }, []);

  // Once every download has finished: a notification, or the countdown
  // (kept by the backend, after.rs) before the window, the app or the
  // computer goes.
  useEffect(() => {
    const a = t.views.settings.after;
    const unlisten = api.onAfterAll((n) => {
      if (n.left != null) return setAfterAll(n);
      setAfterAll(null);
      if (n.reason === "done" && n.action === "notify") api.notify(a.notifyTitle, a.notifyBody).catch(() => {});
      if (n.reason === "busy") toast(a.busy, "info");
    });
    const unlistenFailed = api.onAfterAllFailed((why) => toast(t.views.settings.after.failed(why), "error"));
    return () => {
      unlisten.then((f) => f());
      unlistenFailed.then((f) => f());
    };
  }, [toast]);

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
      const text = t.app.duplicate;
      const percent =
        twin.total && twin.total > 0 ? Math.floor((twin.downloaded / twin.total) * 100) : null;
      const status =
        twin.status === "completed"
          ? text.status.completed
          : twin.status === "failed"
            ? text.status.failed
            : twin.status === "paused"
              ? percent != null
                ? text.status.pausedAt(percent)
                : text.status.paused
              : twin.status === "queued"
                ? text.status.waiting
                : percent != null
                  ? text.status.downloadingAt(percent)
                  : text.status.downloading;
      const statusTone =
        twin.status === "completed" ? "done" : twin.status === "failed" ? "failed" : "active";
      const torrent = isTorrentSource(url);
      const title =
        twin.status === "completed"
          ? torrent
            ? text.title.doneTorrent
            : text.title.doneFile
          : twin.status === "failed"
            ? torrent
              ? text.title.failedTorrent
              : text.title.failedFile
            : torrent
              ? text.title.runningTorrent
              : text.title.runningFile;
      const reply = await ask({
        title,
        message: twin.filename,
        status,
        statusTone,
        detail: torrent ? text.detailTorrent : text.detailFile,
        okLabel: text.ok,
        altLabel: text.show,
        cancelLabel: text.cancel,
        tone: "primary",
      });
      if (reply === "alt") {
        inspect(twin.id);
        return "show";
      }
      return reply ? "go" : "cancel";
    };
    // The torrent window (folder and files) shows only when Settings asks
    // where to save; otherwise the whole torrent starts in the usual folder.
    const openTorrent = async (source: string) => {
      if ((await checkDuplicate(source)) !== "go") return;
      if (askFolderRef.current) {
        setTorrentSource(source.trim());
        return;
      }
      try {
        await api.add({ url: source.trim(), media: { max_height: null, audio_only: false } });
        toast(t.dialogs.torrent.added);
        setNavState("downloads");
        savePref("nav", "downloads");
      } catch (e) {
        fail(e);
      }
    };
    const all: Actions = {
      toast,
      checkDuplicate,
      pause: (ids) => ids.forEach((id) => api.pause(id).catch(fail)),
      resume: (ids) => ids.forEach((id) => api.resume(id).catch(fail)),
      remove: async (ids) => {
        const unfinished = downloadsRef.current.filter(
          (d) => ids.includes(d.id) && d.status !== "completed",
        );
        if (unfinished.length > 0) {
          const text = t.app.cancelDownload;
          const what =
            unfinished.length === 1 ? unfinished[0].filename : text.unfinished(unfinished.length);
          const batch = unfinished.length === 1 && unfinished[0].batch;
          const ok = await ask({
            title: batch ? t.batch.cancelConfirm.title : unfinished.length === 1 ? text.titleOne : text.titleMany,
            message: what,
            detail: batch
              ? t.batch.cancelConfirm.detail
              : unfinished.every((d) => d.profile) ? text.detailProfile : text.detailFile,
            okLabel: text.ok,
            cancelLabel: text.cancel,
          });
          if (!ok) return;
        }
        ids.forEach((id) => api.remove(id, false).catch(fail));
        setSelection((sel) => new Set([...sel].filter((id) => !ids.includes(id))));
        setInspected((id) => (id != null && ids.includes(id) ? null : id));
      },
      deleteFile: async (d) => {
        const ok = await ask({
          title: t.app.deleteFile.title,
          message: d.filename,
          detail: t.app.deleteFile.detail,
          okLabel: t.app.deleteFile.ok,
          cancelLabel: t.app.deleteFile.cancel,
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
          .then(() => toast(t.app.toast.copied(what)))
          .catch(() => toast(t.app.toast.copyFailed(what), "error"));
      },
      scan: (url) => setScanUrl(url),
      inspect,
      openAdd: (url, mode) => setAdd({ url: url ?? "", mode: mode ?? "download" }),
      openBatch: () => setBatchOpen(true),
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
        // A share link or project page (GitHub, Drive, MEGA...): its files
        // are listed to pick from. A store or paid course says why not.
        const site = await api.siteKind(link).catch(() => null);
        if (site?.support === "blocked") {
          toast(siteBlockedText(site), "error");
          return false;
        }
        const duplicate = await checkDuplicate(link);
        if (duplicate !== "go") return duplicate === "show";
        if (site?.support === "files") {
          setSiteSource(link);
          return true;
        }
        try {
          // With "Ask where to save" on in Settings, every download (a file,
          // a video, a playlist or a profile) is given its folder first.
          let dir: string | undefined;
          if (askFolderRef.current) {
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
      // A download whose file has been deleted is fetched afresh from its
      // link, as a new download; the old line stays in History as it was.
      again: (d) => {
        void all.submitLink(d.url);
      },
      schedule: (ids) => setScheduling({ picked: ids }),
      limit: (d) => setLimiting(d),
      refreshLink: (d) => setRelinking(d),
    };
    return all;
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
          toast(t.app.toast.pickOnPage);
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
          label: t.app.menu.pause(chosen.filter(canPause).length),
          icon: Pause,
          onSelect: () => actions.pause(chosen.filter(canPause).map((c) => c.id)),
        });
      }
      if (chosen.some(canResume)) {
        items.push({
          label: t.app.menu.resume(chosen.filter(canResume).length),
          icon: Play,
          onSelect: () => actions.resume(chosen.filter(canResume).map((c) => c.id)),
        });
      }
      if (items.length > 0) items.push("separator");
      items.push({
        label: t.app.menu.remove(chosen.length),
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
    siteSource != null ||
    batchOpen ||
    paletteOpen ||
    aboutOpen ||
    scheduling != null ||
    limiting != null ||
    relinking != null ||
    afterAll != null ||
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
      toast(t.app.toast.clipboardLink);
    });
    return () => {
      unlisten.then((f) => f());
    };
  }, [clipWatch, setNav, toast]);

  // The browser extension: a page it sends lands in the Home box like a
  // copied link (whether or not copied links are picked up), and files it
  // sends start downloading straight away.
  useEffect(() => {
    const link = api.onExtensionLink((sent) => {
      const text = sent.trim();
      if (!text) return;
      setCopied((last) => ({ text, n: (last?.n ?? 0) + 1 }));
      if (!overlayNow.current) setNav("home");
      toast(t.app.toast.extensionLink);
    });
    const added = api.onExtensionAdded((count) => {
      toast(t.app.toast.extensionAdded(count));
    });
    // A page on the Browser page started a file download: the app has it.
    const caught = api.onBrowserDownload(() => toast(t.app.toast.browserDownload));
    const refused = api.onBrowserDownloadFailed((why) => toast(why, "error"));
    return () => {
      link.then((f) => f());
      added.then((f) => f());
      caught.then((f) => f());
      refused.then((f) => f());
    };
  }, [setNav, toast]);

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
      // The Batch Download dialog takes any files dropped while it is open.
      if (dropTo.current) {
        dropTo.current(paths);
        return;
      }
      const torrent = paths.find((p) => /\.torrent$/i.test(p));
      if (torrent) actions.openTorrent(torrent);
      else if (paths.length > 0) toast(t.app.toast.onlyTorrents, "error");
    });
    return () => {
      unlisten.then((f) => f());
    };
  }, [toast, actions]);

  /* ---------- commands ---------- */

  const commands = useMemo<Command[]>(
    () => {
      const c = t.app.commands;
      return [
        { id: "add", group: c.groupActions, label: c.add, icon: Plus, shortcut: "Ctrl N", run: () => setAdd({ url: "", mode: "download" }) },
        { id: "scan", group: c.groupActions, label: c.scan, icon: ScanSearch, run: () => setAdd({ url: "", mode: "scan" }) },
        { id: "batch", group: c.groupActions, label: t.batch.command, icon: Layers, run: () => setBatchOpen(true) },
        { id: "pause-all", group: c.groupActions, label: c.pauseAll, icon: Pause, shortcut: "Ctrl Shift P", run: pauseAll },
        { id: "resume-all", group: c.groupActions, label: c.resumeAll, icon: Play, shortcut: "Ctrl Shift R", run: resumeAll },
        {
          id: "folder",
          group: c.groupActions,
          label: c.folder,
          icon: FolderOpen,
          run: () => api.openDownloadDir().catch((e) => toast(String(e), "error")),
        },
        {
          id: "theme",
          group: c.groupActions,
          label: dark ? c.lightTheme : c.darkTheme,
          icon: dark ? Sun : Moon,
          run: () => chooseTheme(dark ? "light" : "dark"),
        },
        { id: "nav-home", group: c.groupGoTo, label: c.home, icon: House, shortcut: "Ctrl 1", run: () => setNav("home") },
        { id: "nav-downloads", group: c.groupGoTo, label: c.downloads, icon: DownloadGlyph, shortcut: "Ctrl 2", run: () => setNav("downloads") },
        { id: "nav-scheduler", group: c.groupGoTo, label: t.app.nav.scheduler, icon: CalendarClock, run: () => setNav("scheduler") },
        { id: "nav-settings", group: c.groupGoTo, label: c.settings, icon: SettingsGlyph, shortcut: "Ctrl ,", run: () => setNav("settings") },
        { id: "about", group: c.groupGoTo, label: c.about, icon: Info, run: () => setAboutOpen(true) },
      ];
    },
    // `t` changes when the language does, so the labels are built again.
    [dark, pauseAll, resumeAll, chooseTheme, setNav, toast, t],
  );

  const context = { downloads, batchItems, settings, history: history.current, actions, progressStyle };
  const rowHandlers = { selection, onSelect, onMenu };

  return (
    <AppContext.Provider value={context}>
      <div className="app">
        <Ambient />
        <nav className="sidebar" aria-label={t.app.nav.label}>
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
                aria-label={t.app.nav[item.nav]}
                title={t.app.nav[item.nav]}
                onClick={() => setNav(item.nav)}
              >
                <item.icon size={18} strokeWidth={2} aria-hidden="true" />
                <span>{t.app.nav[item.nav]}</span>
              </button>
            ))}
          </div>

          <TransferSummary />

          <div className="nav-group nav-bottom">
            <button
              type="button"
              className="nav-item"
              role="switch"
              aria-checked={dark}
              aria-label={t.app.nav.darkMode}
              title={t.app.nav.darkMode}
              onClick={() => chooseTheme(dark ? "light" : "dark")}
            >
              <Moon size={18} strokeWidth={2} aria-hidden="true" />
              <span>{t.app.nav.darkMode}</span>
              <span className="switch" aria-hidden="true" />
            </button>
            <button
              type="button"
              className="nav-item"
              aria-label={t.app.nav.about}
              title={t.app.nav.about}
              onClick={() => setAboutOpen(true)}
            >
              <Info size={18} strokeWidth={2} aria-hidden="true" />
              <span>{t.app.nav.about}</span>
            </button>
          </div>
        </nav>

        <main className={`workspace${leaving ? " is-leaving" : ""}`}>
          <div className="titlebar" data-tauri-drag-region>
            {isTauri && (
              <div className="window-controls">
                <button type="button" aria-label={t.app.window.minimize} title={t.app.window.minimize} onClick={() => api.windowMinimize()}>
                  <Minus size={16} strokeWidth={2} aria-hidden="true" />
                </button>
                <button type="button" aria-label={t.app.window.maximize} title={t.app.window.maximize} onClick={() => api.windowToggleMaximize()}>
                  <Square size={13} strokeWidth={2} aria-hidden="true" />
                </button>
                <button type="button" className="is-close" aria-label={t.app.window.close} title={t.app.window.close} onClick={() => api.windowClose()}>
                  <X size={17} strokeWidth={2} aria-hidden="true" />
                </button>
              </div>
            )}
          </div>

          {shownNav === "home" && <HomeView
              quality={quality}
              go={setNav}
              signIn={(site) => {
                setSignInStart(site);
                setNav("signin");
              }}
              copied={copied}
              onSettings={(s) => {
                setSettings(s);
                api.setSettings(s).catch((e) => toast(String(e), "error"));
              }}
            />}
          {shownNav === "downloads" && (
            <DownloadsView {...rowHandlers} clearSelection={() => setSelection(new Set())} go={setNav} />
          )}
          {shownNav === "scheduler" && (
            <SchedulerView
              schedules={schedules}
              onNew={() => setScheduling({})}
              onEdit={(schedule) => setScheduling({ schedule })}
            />
          )}
          {shownNav === "history" && <HistoryView onMenu={onMenu} />}
          {shownNav === "browser" && (
            <BrowserView quality={quality} go={setNav} start={browseStart} onStarted={() => setBrowseStart(null)} />
          )}
          {shownNav === "signin" && <SignInView start={signInStart} onStarted={() => setSignInStart(null)} />}
          {shownNav === "settings" && (
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
              section={settingsSection}
              onSection={setSettingsSection}
              appPrefs={appPrefs}
              scheduleCount={schedules.length}
              onOpenScheduler={() => setNav("scheduler")}
              onAppPrefs={(next) => {
                const before = appPrefs;
                setAppPrefs(next);
                api.setAppPrefs(next).catch((e) => {
                  setAppPrefs(before);
                  toast(String(e), "error");
                });
              }}
              onSettings={(s) => {
                setSettings(s);
                api.setSettings(s).catch((e) => toast(String(e), "error"));
              }}
            />
          )}
        </main>

        {inspectedDownload?.batch && (
          <BatchDetail
            d={inspectedDownload}
            onClose={() => {
              setInspected(null);
              setSelection(new Set());
              anchor.current = null;
            }}
          />
        )}
        {inspectedDownload && !inspectedDownload.batch && (
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
        {batchOpen && (
          <BatchDialog
            quality={quality}
            onClose={() => setBatchOpen(false)}
            onStarted={() => setNav("downloads")}
            takeDrops={(take) => (dropTo.current = take)}
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
        {siteSource && (
          <TorrentDialog
            site
            source={siteSource}
            onClose={() => setSiteSource(null)}
            onAdded={() => setNav("downloads")}
          />
        )}
        {scanUrl && (
          <ScanDialog url={scanUrl} media={qualityOptions(quality)} onClose={() => setScanUrl(null)} />
        )}
        {aboutOpen && <AboutDialog onClose={() => setAboutOpen(false)} />}
        {limiting && <SpeedLimitDialog d={limiting} onClose={() => setLimiting(null)} />}
        {relinking && <RefreshLinkDialog d={relinking} onClose={() => setRelinking(null)} />}
        {afterAll && afterAll.left != null && afterAll.action !== "nothing" && afterAll.action !== "notify" && (
          <Modal
            title={t.views.settings.after.dialog.title[afterAll.action]}
            onClose={() => api.afterAllCancel().catch(() => {})}
            footer={
              <>
                <span className="spacer" />
                <button type="button" className="btn" data-autofocus onClick={() => api.afterAllCancel().catch(() => {})}>
                  {t.dialogs.cancel}
                </button>
                <button
                  type="button"
                  className={`btn ${["sleep", "hibernate", "shutdown", "exit"].includes(afterAll.action) ? "btn-danger-solid" : "btn-primary"}`}
                  onClick={() => api.afterAllNow().catch(() => {})}
                >
                  {t.views.settings.after.dialog.now[afterAll.action]}
                </button>
              </>
            }
          >
            <p>{t.views.settings.after.dialog.body}</p>
            <p className="after-count" aria-live="polite">
              {t.views.settings.after.dialog.in(durationText(afterAll.left))}
            </p>
          </Modal>
        )}
        {scheduling && (
          <ScheduleDialog
            schedule={scheduling.schedule}
            picked={scheduling.picked}
            onClose={() => setScheduling(null)}
          />
        )}
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
        <PairPrompt />
        <Toasts toasts={toasts} dismiss={(id) => setToasts((list) => list.filter((item) => item.id !== id))} />
      </div>
    </AppContext.Provider>
  );
}
