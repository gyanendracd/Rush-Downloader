import { useEffect, useId, useMemo, useRef, useState, type FormEvent, type MouseEvent as ReactMouseEvent, type ReactNode } from "react";
import { flushSync } from "react-dom";
import {
  Archive,
  ArrowLeft,
  ArrowRight,
  Brush,
  CalendarClock,
  Download as DownloadGlyph,
  File as FileGlyph,
  FileText,
  Film,
  Folder,
  FolderOpen,
  Gauge,
  Globe,
  Image as ImageGlyph,
  Info,
  KeyRound,
  Languages,
  LayoutGrid,
  ListFilter,
  Music,
  Package,
  Pause,
  Play,
  Puzzle,
  RotateCw,
  Search,
  Settings as SettingsGlyph,
  ShieldCheck,
  Users,
  Wifi,
  Wrench,
  X,
  Zap,
  type LucideIcon,
} from "lucide-react";
import {
  type AppPrefs,
  api,
  canPause,
  canResume,
  categoryOf,
  COOKIE_BROWSERS,
  displayName,
  formatBytes,
  formatDuration,
  formatSpeed,
  isDeleted,
  isRunning,
  loadPref,
  needsSignIn,
  onDownloadsPage,
  percentOf,
  checkPercent,
  looksLikePage,
  looksLikeUrl,
  QUALITIES,
  qualityOptions,
  savePref,
  SIGN_IN_SITES,
  signInSiteOf,
  type Category,
  type Download,
  type CaughtMedia,
  type MediaInfo,
  type MediaOptions,
  type PlaylistInfo,
  type ProfileFace,
  type ProfileRef,
  type Settings,
  type ToolStatus,
} from "./api";
import { SITE_LOGOS } from "./siteLogos";
import { DownloadRow, Dropdown, FileIcon, IconBtn, Modal, useApp, type ProgressStyle, Select } from "./ui";
import { changeLanguageLabel, dateText, ENGLISH_CHANGE_LANGUAGE, formatLocale, languageOptionLabel, listText, type Locale, locale, LOCALES, num, pct, setLocale, t } from "./i18n";

export type Nav = "home" | "downloads" | "history" | "browser" | "signin" | "settings";
export const NAVS: readonly Nav[] = ["home", "downloads", "history", "browser", "signin", "settings"];

export interface RowHandlers {
  selection: Set<number>;
  onSelect: (id: number, e: ReactMouseEvent, visible: number[]) => void;
  onMenu: (d: Download, x: number, y: number) => void;
}

/* ---------- browser ---------- */

/** Videos smaller than this are taken for preview clips, not the video itself. */
const PREVIEW_BYTES = 4 * 1024 * 1024;

/** Order of the caught list: streams, then videos by picture size, then audio. */
function rank(item: CaughtMedia): number {
  if (item.kind === "stream") return 100000;
  if (item.kind === "audio") return -1;
  return Number.parseInt(item.label, 10) || 0;
}

/** One line of a caught video's quality list. */
interface CaughtChoice {
  key: string;
  item: CaughtMedia;
  /** The quality picked from a stream's list; none for a plain file. */
  height: number | null;
  label: string;
  /** A very small video, most likely a preview clip. */
  preview?: boolean;
}

/**
 * What the files caught from a page offer, best first. With a stream caught,
 * only the stream counts (its small pieces show up as files of their own);
 * very small videos are preview clips and go last.
 */
function caughtChoices(found: CaughtMedia[], sizes: Map<string, number | null>): CaughtChoice[] {
  const preview = (item: CaughtMedia) => {
    const size = sizes.get(item.url);
    return item.kind === "video" && size != null && size < PREVIEW_BYTES;
  };
  const streams = found.filter((item) => item.kind === "stream");
  const masters = streams.filter((item) => item.master);
  const pool = masters.length > 0 ? masters : streams.length > 0 ? streams : found;
  const listed = [...pool].sort((a, b) => Number(preview(a)) - Number(preview(b)) || rank(b) - rank(a));
  return listed.flatMap((item) => {
    if (item.kind === "stream" && item.heights.length > 0) {
      return item.heights.map((h) => ({
        key: `${item.url}|${h}`,
        item,
        height: h as number | null,
        label: t.common.quality.height(String(h)),
      }));
    }
    const size = sizes.get(item.url);
    const what =
      item.kind === "stream"
        ? t.views.browser.stream
        : item.label || (item.kind === "audio" ? t.views.browser.audio : t.views.browser.video);
    const detail = item.kind === "stream" ? "" : size != null ? formatBytes(size) : "";
    const label = [what, detail, preview(item) ? t.views.browser.previewClip : ""].filter(Boolean).join(" · ");
    return [{ key: `${item.url}|`, item, height: null as number | null, label, preview: preview(item) }];
  });
}

/** The last part of an address, readable: "holiday-clip.mp4". */
function mediaName(address: string): string {
  try {
    const url = new URL(address);
    const last = decodeURIComponent(url.pathname.split("/").filter(Boolean).pop() ?? "");
    return last || url.hostname;
  } catch {
    return address;
  }
}

/**
 * A small browser inside the app. Open a page and play its video: every video
 * or audio file the page loads is listed above it, ready to download, the way
 * a download manager's browser add-on catches them.
 */
export function BrowserView(props: {
  quality: string;
  go: (nav: Nav) => void;
  /** A page to open straight away, e.g. a link that could not be downloaded.
   *  With `fetch`, the player is also started, so the video's qualities show
   *  up without the user pressing play. Downloading stays the user's choice. */
  start?: { url: string; fetch: boolean } | null;
  onStarted?: () => void;
}) {
  const { actions } = useApp();
  const [address, setAddress] = useState("");
  /** The page being shown, and the video the site downloader finds on it. */
  const [pageUrl, setPageUrl] = useState("");
  const [pageVideo, setPageVideo] = useState<MediaInfo | null>(null);
  const [pageChoice, setPageChoice] = useState("best");
  const [open, setOpen] = useState(false);
  const [title, setTitle] = useState("");
  const [found, setFound] = useState<CaughtMedia[]>([]);
  const [blocked, setBlocked] = useState<string | null>(null);
  const [started, setStarted] = useState<Set<string>>(new Set());
  /** While the app is finding this page's video by itself: when it started (0: it is not). */
  const [fetching, setFetching] = useState(0);
  /** The caught file picked in the quality list; empty for "the best one". */
  const [pick, setPick] = useState("");
  /** Size of each caught file: a number once known, null when the server will not say. */
  const [sizes, setSizes] = useState<Map<string, number | null>>(new Map());
  const asked = useRef<Set<string>>(new Set());
  const stage = useRef<HTMLDivElement>(null);
  const page = useRef("");

  const area = () => {
    const box = stage.current?.getBoundingClientRect();
    return box
      ? { x: box.left, y: box.top, width: box.width, height: box.height }
      : { x: 0, y: 0, width: 0, height: 0 };
  };

  useEffect(() => {
    const listeners = [
      api.onBrowserMedia((item) =>
        setFound((list) => {
          const known = list.find((f) => f.url === item.url);
          // A stream is told again once its list of qualities has been read.
          if (known && !(item.master && !known.master) && item.heights.length <= known.heights.length) return list;
          // Largest picture first; streams, which hold every quality, on top.
          return [...list.filter((f) => f.url !== item.url), item].slice(-40).sort((a, b) => rank(b) - rank(a));
        }),
      ),
      api.onBrowserNav((url) => {
        setAddress(url);
        setBlocked(null);
        // A new page starts a new list; moving within a page keeps it.
        const where = url.split("#")[0];
        if (where !== page.current) {
          page.current = where;
          setFound([]);
          setStarted(new Set());
          setPick("");
          setPageVideo(null);
          setPageUrl(where);
        }
      }),
      api.onBrowserTitle(setTitle),
      api.onBrowserPopup(setBlocked),
    ];
    // Leaving the page keeps any sign-in made while browsing and closes the site.
    return () => {
      listeners.forEach((l) => l.then((f) => f()));
      api.signinClose().catch(() => {});
    };
  }, []);

  // The site's page follows this page's layout.
  useEffect(() => {
    if (!open || !stage.current) return;
    const follow = () => api.signinBounds(area()).catch(() => {});
    const watcher = new ResizeObserver(follow);
    watcher.observe(stage.current);
    window.addEventListener("resize", follow);
    follow();
    return () => {
      watcher.disconnect();
      window.removeEventListener("resize", follow);
    };
  }, [open]);

  // Sites the site downloader knows give the whole video straight from the
  // page's address, in every resolution. That beats anything caught from the
  // player, which is often only a short preview clip.
  useEffect(() => {
    if (!pageUrl) return;
    let stale = false;
    const timer = window.setTimeout(() => {
      api
        .mediaInfo(pageUrl)
        .then((info) => {
          if (stale) return;
          setPageVideo(info);
          const usual = Number(props.quality);
          const fits = Number.isFinite(usual) ? info.heights.find((h) => h <= usual) : undefined;
          setPageChoice(props.quality === "audio" ? "audio" : fits ? String(fits) : "best");
        })
        .catch(() => {
          // Not a page the site downloader knows; catching still works.
        });
    }, 900);
    return () => {
      stale = true;
      window.clearTimeout(timer);
    };
  }, [pageUrl, props.quality]);

  async function downloadPage() {
    try {
      await api.browserKeepSignIn();
      await api.add({
        url: pageUrl,
        via_site: true,
        media:
          pageChoice === "audio"
            ? { max_height: null, audio_only: true }
            : { max_height: pageChoice === "best" ? null : Number(pageChoice), audio_only: false },
      });
      setStarted((set) => new Set(set).add(pageUrl));
      actions.toast(t.views.downloadStarted);
    } catch (e) {
      actions.toast(String(e), "error");
    }
  }

  // A few megabytes at most is a preview clip, not the video itself.
  const isPreview = (item: CaughtMedia) => {
    const size = sizes.get(item.url);
    return item.kind === "video" && size != null && size < PREVIEW_BYTES;
  };
  // A video that comes as a stream is fetched in many small pieces, and those
  // pieces show up as tiny files of their own. With a stream caught, only the
  // stream counts: the list of qualities if there is one, else what was found.
  const streams = found.filter((item) => item.kind === "stream");
  const masters = streams.filter((item) => item.master);
  const pool = masters.length > 0 ? masters : streams.length > 0 ? streams : found;
  const listed = [...pool].sort((a, b) => Number(isPreview(a)) - Number(isPreview(b)) || rank(b) - rank(a));
  /** "720p · 45.0 MB", "Stream · every quality", "Preview clip · 198 KB". */
  const optionLabel = (item: CaughtMedia) => {
    const size = sizes.get(item.url);
    const what =
      item.kind === "stream"
        ? t.views.browser.stream
        : item.label || (item.kind === "audio" ? t.views.browser.audio : t.views.browser.video);
    const detail =
      item.kind === "stream"
        ? t.views.browser.everyQuality
        : size != null
          ? formatBytes(size)
          : sizes.has(item.url)
            ? ""
            : "…";
    // A very small video is most likely a preview clip; say so, last.
    return [what, detail, isPreview(item) ? t.views.browser.previewClip : ""].filter(Boolean).join(" · ");
  };
  // What the quality list offers: every quality a stream names, one line
  // each, or the file itself. Best first.
  const choices = listed.flatMap((item) =>
    item.kind === "stream" && item.heights.length > 0
      ? item.heights.map((h) => ({
          key: `${item.url}|${h}`,
          item,
          height: h as number | null,
          label: t.common.quality.height(String(h)),
        }))
      : [{ key: `${item.url}|`, item, height: null as number | null, label: optionLabel(item) }],
  );
  // The one on show: the user's pick, else the best there is.
  const chosen = choices.find((c) => c.key === pick) ?? choices[0];

  // Each caught file is asked for its size once. A stream is many small
  // pieces, so it has no single size to show.
  useEffect(() => {
    for (const item of found) {
      if (item.kind === "stream" || asked.current.has(item.url)) continue;
      asked.current.add(item.url);
      api
        .linkSize(item.url, item.referer)
        .catch(() => null)
        .then((size) => setSizes((known) => new Map(known).set(item.url, size)));
    }
  }, [found]);

  async function visit(target: string) {
    const typed = target.trim();
    if (!typed) return;
    const url = /^https?:\/\//i.test(typed) ? typed : `https://${typed}`;
    try {
      await api.signinOpen(url, area(), true);
      setOpen(true);
      setAddress(url);
    } catch (e) {
      actions.toast(String(e), "error");
    }
  }

  // A page handed over from a failed download opens by itself.
  useEffect(() => {
    if (!props.start) return;
    const { url, fetch } = props.start;
    // Wait for the layout, so the page gets its real area. The hand-over is
    // marked done only once the page is asked for, or this would cancel itself.
    const timer = window.setTimeout(() => {
      visit(url);
      setFetching(fetch ? Date.now() : 0);
      props.onStarted?.();
    }, 50);
    return () => window.clearTimeout(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [props.start]);

  // Finding a page's video for the user: start the player a few times (pages
  // take a while to be ready) so its qualities show up. Nothing is downloaded
  // here; the user picks a quality and presses Download.
  useEffect(() => {
    if (!fetching || !open) return;
    const presses = [2500, 6000, 11000, 18000].map((after) =>
      window.setTimeout(() => api.browserAutoplay().catch(() => {}), after),
    );
    const giveUp = window.setTimeout(() => {
      setFetching(0);
      actions.toast(t.views.browser.autoplayFailed, "error");
    }, 40000);
    return () => {
      presses.forEach(window.clearTimeout);
      window.clearTimeout(giveUp);
    };
  }, [fetching, open, actions]);

  useEffect(() => {
    if (!fetching) return;
    // A page the site downloader does read after all: its row is on show.
    if (pageVideo) {
      setFetching(0);
      actions.toast(t.views.browser.videoFound);
      return;
    }
    const best = choices[0];
    // Wait for a real video: a stream, or a file known not to be a preview clip.
    const ready =
      best &&
      !isPreview(best.item) &&
      (best.item.kind === "stream" || sizes.get(best.item.url) != null || Date.now() - fetching > 15000);
    if (!ready) return;
    // A moment more, so a stream's list of qualities, read a little later, is in.
    // Then the choice is the user's: nothing downloads by itself.
    const timer = window.setTimeout(() => {
      setFetching(0);
      actions.toast(t.views.browser.videoFound);
    }, 3500);
    return () => window.clearTimeout(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [fetching, found, sizes, pageVideo]);

  /** `height`: the quality picked from a stream's list; none for a plain file. */
  async function download(item: CaughtMedia, height: number | null, key: string) {
    try {
      await api.browserKeepSignIn();
      await api.add({
        url: item.url,
        media: height ? { max_height: height, audio_only: false } : qualityOptions(props.quality),
        via_site: true,
        headers: [["Referer", item.referer]],
        filename: title.trim() || undefined,
      });
      setStarted((set) => new Set(set).add(key));
      actions.toast(t.views.downloadStarted);
    } catch (e) {
      actions.toast(String(e), "error");
    }
  }

  return (
    <div className="page page-browser">
      <form
        className="browser-bar"
        onSubmit={(e) => {
          e.preventDefault();
          visit(address);
        }}
      >
        <button type="button" className="icon-btn" aria-label={t.views.browser.back} title={t.views.browser.back} disabled={!open} onClick={() => api.browserGo("back")}>
          <ArrowLeft size={17} strokeWidth={2} aria-hidden="true" />
        </button>
        <button type="button" className="icon-btn" aria-label={t.views.browser.forward} title={t.views.browser.forward} disabled={!open} onClick={() => api.browserGo("forward")}>
          <ArrowRight size={17} strokeWidth={2} aria-hidden="true" />
        </button>
        <button type="button" className="icon-btn" aria-label={t.views.browser.reload} title={t.views.browser.reload} disabled={!open} onClick={() => api.browserGo("reload")}>
          <RotateCw size={16} strokeWidth={2} aria-hidden="true" />
        </button>
        <input
          type="text"
          value={address}
          onChange={(e) => setAddress(e.target.value)}
          placeholder={t.views.browser.addressPlaceholder}
          aria-label={t.views.browser.address}
          spellCheck={false}
          autoFocus
        />
        <button type="submit" className="btn btn-primary" disabled={!address.trim()}>
          {t.views.browser.go}
        </button>
      </form>

      {blocked && (
        <p className="browser-note" role="status">
          <span>{t.views.browser.popup(mediaName(blocked))}</span>
          <button type="button" className="btn btn-small" onClick={() => visit(blocked)}>
            {t.views.browser.openHere}
          </button>
          <button type="button" className="btn btn-small" onClick={() => setBlocked(null)}>
            {t.views.browser.ignore}
          </button>
        </p>
      )}

      {fetching > 0 && (
        <p className="browser-note" role="status">
          <span>{t.views.browser.finding}</span>
          <button type="button" className="btn btn-small" onClick={() => setFetching(0)}>
            {t.views.browser.stop}
          </button>
        </p>
      )}
      {open && (
        <section className="browser-found" aria-label={t.views.browser.foundRegion}>
          {pageVideo && (
            <div className="browser-page-video">
              <span className="browser-kind is-full">{t.views.browser.fullVideo}</span>
              <span className="browser-name" title={pageVideo.title}>
                {pageVideo.title}
              </span>
              <Select
                aria-label={t.views.browser.resolution}
                value={pageChoice}
                onChange={setPageChoice}
                options={[
                  { value: "best", label: t.views.quality.best },
                  ...pageVideo.heights.map((h) => ({ value: String(h), label: t.common.quality.height(String(h)) })),
                  { value: "audio", label: t.views.quality.audioOnly },
                ]}
              />
              {started.has(pageUrl) ? (
                <button type="button" className="btn btn-small" onClick={() => props.go("downloads")}>
                  {t.views.browser.inDownloads}
                </button>
              ) : (
                <button type="button" className="btn btn-small btn-primary" onClick={downloadPage}>
                  {t.views.browser.download}
                </button>
              )}
            </div>
          )}
          {/* With the full video on offer, the files caught from the player
              (clips, pieces, previews) would only be clutter beside it. */}
          {pageVideo ? null : found.length === 0 ? (
            <p className="browser-empty">{t.views.browser.nothingCaught}</p>
          ) : (
            // Everything caught from the player is one row: the best file is
            // chosen, the other qualities are in the list beside it.
            <div className="browser-page-video is-caught">
              <span className="browser-kind">
                {chosen.item.kind === "audio" ? t.views.browser.audio : t.views.browser.video}
              </span>
              <span className="browser-name" title={chosen.item.url}>
                {title.trim() || mediaName(chosen.item.url)}
              </span>
              <Select
                aria-label={t.views.browser.qualityLabel}
                value={chosen.key}
                onChange={setPick}
                options={choices.map((choice) => ({ value: choice.key, label: choice.label }))}
              />
              {started.has(chosen.key) ? (
                <button type="button" className="btn btn-small" onClick={() => props.go("downloads")}>
                  {t.views.browser.inDownloads}
                </button>
              ) : (
                <button
                  type="button"
                  className="btn btn-small btn-primary"
                  onClick={() => download(chosen.item, chosen.height, chosen.key)}
                >
                  {t.views.browser.download}
                </button>
              )}
            </div>
          )}
        </section>
      )}

      <div ref={stage} className={`signin-stage${open ? " is-open" : ""}`}>
        {open ? (
          <p>{t.views.browser.shownHere}</p>
        ) : (
          <p>{t.views.browser.intro}</p>
        )}
      </div>
    </div>
  );
}

/* ---------- sign in ---------- */

/** The mark of a site that can be signed in to, drawn in its own colours. */
function SiteIcon({ site }: { site: string }) {
  const gradient = useId().replace(/[^a-zA-Z0-9]/g, "");
  const common = { className: "site-icon", width: 22, height: 22, viewBox: "0 0 24 24", "aria-hidden": true } as const;
  const logo = SITE_LOGOS[site];
  if (!logo) {
    // Streamable has no mark in the set the others come from: its colour, with its initial.
    if (site === "Streamable") {
      return (
        <svg {...common}>
          <rect x="1" y="1" width="22" height="22" rx="6" fill="#0F90FA" />
          <text x="12" y="12.6" fill="#fff" fontSize="14" fontWeight="800" textAnchor="middle" dominantBaseline="central">
            S
          </text>
        </svg>
      );
    }
    return <Globe className="site-icon" size={22} strokeWidth={1.75} aria-hidden="true" />;
  }
  // A mark too pale to stand on glass is drawn dark, on a tile of its colour.
  if (logo.color.startsWith("badge:")) {
    return (
      <svg {...common}>
        <rect width="24" height="24" rx="6" fill={logo.color.slice(6)} />
        <path d={logo.path} fill="#111" transform="translate(4.2 4.2) scale(0.65)" />
      </svg>
    );
  }
  if (logo.color === "gradient") {
    return (
      <svg {...common}>
        <defs>
          <linearGradient id={gradient} x1="2" y1="22" x2="22" y2="2" gradientUnits="userSpaceOnUse">
            <stop offset="0" stopColor="#FEC43B" />
            <stop offset="0.5" stopColor="#EE2A7B" />
            <stop offset="1" stopColor="#6228D7" />
          </linearGradient>
        </defs>
        <path d={logo.path} fill={`url(#${gradient})`} />
      </svg>
    );
  }
  return (
    <svg {...common}>
      <path d={logo.path} fill={logo.color === "text" ? "currentColor" : logo.color} />
    </svg>
  );
}

/**
 * Sign in to a site without leaving the app. The site's own page is shown
 * inside this page; the sign-in made there is kept for downloads.
 */
export function SignInView(props: {
  /** A site (its label on this page) to open straight away, then forgotten. */
  start?: string | null;
  onStarted?: () => void;
}) {
  const { actions } = useApp();
  const [signedIn, setSignedIn] = useState<string[]>([]);
  /** The site whose page is open, by its button label. */
  const [open, setOpen] = useState<string | null>(null);
  const stage = useRef<HTMLDivElement>(null);
  /** A note shown when the site's page opens another site's sign-in window. */
  const [blocked, setBlocked] = useState<string | null>(null);

  useEffect(() => {
    setBlocked(null);
    if (!open) return;
    const unlisten = api.onSignInPopup((host) =>
      setBlocked(
        /(^|\.)google\.com$|(^|\.)youtube\.com$/i.test(host)
          ? t.views.signin.googlePopup
          : host
            ? t.views.signin.popup(host)
            : t.views.signin.popupUnknown,
      ),
    );
    return () => {
      unlisten.then((f) => f());
    };
  }, [open]);

  const area = () => {
    const box = stage.current?.getBoundingClientRect();
    return box
      ? { x: box.left, y: box.top, width: box.width, height: box.height }
      : { x: 0, y: 0, width: 0, height: 0 };
  };

  useEffect(() => {
    api.signInStatus().then(setSignedIn).catch(() => setSignedIn([]));
    // Leaving this page saves the sign-in and closes the site.
    return () => {
      api.signinClose().catch(() => {});
    };
  }, []);

  // The site's page follows this page's layout.
  useEffect(() => {
    if (!open || !stage.current) return;
    const follow = () => api.signinBounds(area()).catch(() => {});
    const watcher = new ResizeObserver(follow);
    watcher.observe(stage.current);
    window.addEventListener("resize", follow);
    follow();
    return () => {
      watcher.disconnect();
      window.removeEventListener("resize", follow);
    };
  }, [open]);

  // Sent here for one site (from the note on Home): its own sign-in page
  // opens at once, once this page is laid out and its area is known.
  useEffect(() => {
    if (!props.start) return;
    const site = SIGN_IN_SITES.find((s) => s.label === props.start);
    const frame = requestAnimationFrame(() => {
      // Asked once: the start is forgotten as soon as it is acted on.
      props.onStarted?.();
      if (site) void openSite(site);
    });
    return () => cancelAnimationFrame(frame);
    // Only a new start matters, not the callbacks.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [props.start]);

  async function openSite(site: { label: string; url: string }) {
    try {
      await api.signinOpen(site.url, area());
      setOpen(site.label);
    } catch (e) {
      actions.toast(String(e), "error");
    }
  }

  async function done() {
    try {
      const sites = await api.signinClose();
      setSignedIn(sites);
      setOpen(null);
      actions.toast(sites.length > 0 ? t.views.signin.saved : t.views.signin.noneFound);
    } catch (e) {
      actions.toast(String(e), "error");
    }
  }

  async function importFile() {
    try {
      const path = await api.pickCookiesFile();
      if (!path) return;
      setSignedIn(await api.importSignIn(path));
      actions.toast(t.views.signin.imported);
    } catch (e) {
      actions.toast(String(e), "error");
    }
  }

  /** The browser whose sign-ins are being read right now, if any. */
  const [reading, setReading] = useState<string | null>(null);

  async function importBrowser(browser: { value: string; label: string }) {
    if (reading) return;
    setReading(browser.value);
    try {
      const before = new Set(signedIn);
      const sites = await api.importBrowserSignIn(browser.value);
      setSignedIn(sites);
      const fresh = sites.filter((site) => !before.has(site)).length;
      actions.toast(
        fresh > 0
          ? t.views.signin.broughtOver(fresh, browser.label)
          : t.views.signin.upToDate(browser.label),
      );
    } catch (e) {
      actions.toast(String(e), "error");
    } finally {
      setReading(null);
    }
  }

  async function forget() {
    try {
      await api.forgetSignIns();
      setSignedIn([]);
      setOpen(null);
      actions.toast(t.views.signin.forgotten);
    } catch (e) {
      actions.toast(String(e), "error");
    }
  }

  return (
    <div className={`page page-signin${open ? " is-open" : ""}`}>
      <div className="page-head">
        <h1>{t.views.signin.title}</h1>
        {open && (
          <>
            <span className="signin-where">
              {t.views.signin.whereBefore}
              <strong>{open}</strong>
              {t.views.signin.whereAfter}
            </span>
            <span className="spacer" />
            <button type="button" className="btn btn-primary" onClick={done}>
              {t.views.signin.saveAndClose}
            </button>
          </>
        )}
      </div>

      {open ? (
        blocked && (
          <p className="signin-blocked" role="status">
            {blocked}
          </p>
        )
      ) : (
        <div className="signin-intro">
          <p>{t.views.signin.intro}</p>
          <div className="signin-sites site-grid">
            {SIGN_IN_SITES.map((site) => (
              <button key={site.label} type="button" className="btn site-btn" onClick={() => openSite(site)}>
                <SiteIcon site={site.label} />
                {site.label}
              </button>
            ))}
          </div>
          <section className="signin-browsers" aria-label={t.views.signin.browsersRegion}>
            <h2>{t.views.signin.elsewhereTitle}</h2>
            <p>{t.views.signin.elsewhereText}</p>
            <div className="signin-sites">
              {COOKIE_BROWSERS.map((browser) => (
                <button
                  key={browser.value}
                  type="button"
                  className={`btn${reading === browser.value ? " is-busy" : ""}`}
                  disabled={reading != null}
                  aria-busy={reading === browser.value}
                  onClick={() => importBrowser(browser)}
                >
                  <Globe size={16} strokeWidth={2} aria-hidden="true" />
                  {reading === browser.value ? t.views.signin.reading(browser.label) : browser.label}
                </button>
              ))}
              <button type="button" className="btn" disabled={reading != null} onClick={importFile}>
                <FolderOpen size={16} strokeWidth={2} aria-hidden="true" /> {t.views.signin.importFile}
              </button>
            </div>
            <small className="form-note">{t.views.signin.browserNote}</small>
          </section>
          <p className="signin-status">
            {signedIn.length > 0 ? (
              <>
                {t.views.signin.signedIn} <strong>{listText(signedIn)}</strong>
                <button type="button" className="btn btn-small" onClick={forget}>
                  {t.views.signin.forget}
                </button>
              </>
            ) : (
              t.views.signin.notSignedIn
            )}
          </p>
        </div>
      )}

      <div ref={stage} className={`signin-stage${open ? " is-open" : ""}`}>
        {open ? (
          <p>{t.views.signin.shownHere}</p>
        ) : (
          <p>{t.views.signin.googleNote}</p>
        )}
      </div>
    </div>
  );
}

/* ---------- home ---------- */

/** What the app is good at, in four marks along the foot of the Home page. */
/** A profile's numbers: every digit for posts, "94.4K" for a following. */
const short = (n: number) =>
  new Intl.NumberFormat(formatLocale(), { notation: "compact", maximumFractionDigits: 1 }).format(n);

/** "Dec 2017", from the day a profile joined ("2017-12-14"). */
function joinedIn(day: string): string {
  const at = new Date(`${day}T00:00:00`);
  return Number.isNaN(at.getTime()) ? day : dateText(at, { month: "short", year: "numeric" });
}

type Feature = "fast" | "safe" | "builtIn" | "smart";
const FEATURES: { id: Feature; icon: LucideIcon }[] = [
  { id: "fast", icon: Zap },
  { id: "safe", icon: ShieldCheck },
  { id: "builtIn", icon: Globe },
  { id: "smart", icon: LayoutGrid },
];

/** The link in the Home box, kept while the app is open: a visit to another
 *  page (to sign in, say) finds it still there on the way back. */
let keptLink = "";

export function HomeView(props: {
  quality: string;
  go: (nav: Nav) => void;
  /** Opens a site's own sign-in page on the Sign in page (by its label). */
  signIn: (site: string) => void;
  /** A link the user just copied somewhere; it goes straight into the box. */
  copied?: { text: string; n: number } | null;
}) {
  const { downloads, actions } = useApp();
  const [url, setUrl] = useState(props.copied?.text ?? keptLink);
  keptLink = url;
  /** The link's site will not show this to a visitor who is not signed in. */
  const [locked, setLocked] = useState(false);
  useEffect(() => {
    if (props.copied) setUrl(props.copied.text);
  }, [props.copied]);
  const [busy, setBusy] = useState(false);
  const valid = looksLikeUrl(url);
  const running = downloads.filter(isRunning);
  const speed = running.reduce((sum, d) => sum + d.speed, 0);

  // A video page shows its picture, title and resolutions under the box.
  const [video, setVideo] = useState<MediaInfo | null>(null);
  // A profile link downloads everything the profile has posted.
  const [profile, setProfile] = useState<ProfileRef | null>(null);
  /** What the playlist or profile holds (name, picture, videos, photos, length), once looked up. */
  const [playlist, setPlaylist] = useState<PlaylistInfo | null>(null);
  const [counting, setCounting] = useState(false);
  /** Whose profile it is: the name it shows, its picture and its line about itself. */
  const [face, setFace] = useState<ProfileFace | null>(null);
  /** The same, with its numbers, as the site itself answers a visitor who is not signed in. */
  const [stats, setStats] = useState<ProfileFace | null>(null);
  /** Each part of the card is looked up on its own and shows the moment its
   *  answer comes; until then it is a placeholder, and one that comes back
   *  empty simply goes. */
  const [faceLoading, setFaceLoading] = useState(false);
  const [statsLoading, setStatsLoading] = useState(false);
  /** How long the count has been going, to say so when it is slow. */
  const [countSeconds, setCountSeconds] = useState(0);
  const [bannerFailed, setBannerFailed] = useState<string | null>(null);
  /** A picture the site would not hand over; the card falls back to its plain badge. */
  const [pictureFailed, setPictureFailed] = useState<string | null>(null);
  /** The resolutions the playlist's videos really have; null while unknown. */
  const [listHeights, setListHeights] = useState<number[] | null>(null);
  const [sizing, setSizing] = useState(false);
  const [looking, setLooking] = useState(false);
  /** "best", "audio" or a height such as "720". */
  const [choice, setChoice] = useState("best");
  /** A page the site downloader cannot read, whose video is being caught out of sight. */
  const [catchLink, setCatchLink] = useState<string | null>(null);
  const [catching, setCatching] = useState(false);
  const [caught, setCaught] = useState<CaughtMedia[]>([]);
  const [caughtSizes, setCaughtSizes] = useState<Map<string, number | null>>(new Map());
  const sizeAsked = useRef<Set<string>>(new Set());
  const [caughtPick, setCaughtPick] = useState("");
  const [pageTitle, setPageTitle] = useState("");
  const [picture, setPicture] = useState<string | null>(null);
  const link = url.trim();
  useEffect(() => {
    setVideo(null);
    setProfile(null);
    setPlaylist(null);
    setFace(null);
    setStats(null);
    setFaceLoading(false);
    setStatsLoading(false);
    setListHeights(null);
    setSizing(false);
    setCounting(false);
    setLooking(false);
    setCatchLink(null);
    setLocked(false);
    // A profile's name may end like a file's ("john.doe"), so any web link is asked about.
    if (!looksLikeUrl(link) || !link.toLowerCase().startsWith("http")) return;
    let stale = false;
    // Wait for typing to stop before asking the site.
    const timer = window.setTimeout(async () => {
      // Everything a profile card shows: whose it is, its numbers, what it holds.
      const showProfile = (who: ProfileRef) => {
        setProfile(who);
        // Start from the usual quality.
        setChoice(QUALITIES.some((q) => q.value === props.quality) ? props.quality : "best");
        setCounting(true);
        // Whose profile it is comes first and on its own: it takes a second,
        // and the site gives it even where it will not list the posts.
        const onProfile = !who.site.endsWith("playlist");
        setFaceLoading(onProfile);
        setStatsLoading(onProfile);
        const seen = onProfile ? api.profileFace(link).catch(() => null) : Promise.resolve(null);
        seen.then((found) => {
          if (stale) return;
          setFace(found);
          setFaceLoading(false);
        });
        // Its numbers (posts, followers, when it joined) take a moment longer,
        // and are asked for as a visitor who is not signed in.
        if (onProfile) {
          api
            .profileStats(link)
            .then((found) => !stale && setStats(found))
            .catch(() => {})
            .finally(() => !stale && setStatsLoading(false));
        }
        api
          .playlistInfo(link)
          .then((found) => !stale && setPlaylist(found))
          .catch(async (e) => {
            if (stale) return;
            // The card still says what Download will do. A profile the site
            // shows to everyone but "cannot find" for the downloader is one
            // it only lists for signed-in visitors.
            const there = await seen;
            if (stale) return;
            const hidden = !!(there?.name || there?.picture) && /not found|could not be found/i.test(String(e));
            if (needsSignIn(String(e)) || hidden) setLocked(true);
          })
          .finally(() => !stale && setCounting(false));
      };
      const who = await api.profileOf(link).catch(() => null);
      if (stale) return;
      if (who) {
        showProfile(who);
        return;
      }
      if (!looksLikePage(link)) return;
      setLooking(true);
      // A page on any other site may still be someone's whole collection:
      // the downloading programs are asked, side by side with the usual
      // look for a single video. Their answer decides which card shows.
      const collection = api.collectionOf(link).catch(() => null);
      const media = api.mediaInfo(link);
      media.catch(() => {});
      const found = await collection;
      if (stale) return;
      if (found) {
        setLooking(false);
        showProfile(found);
        return;
      }
      media
        .then((found) => {
          if (stale) return;
          setVideo(found);
          // Start from the usual quality, or the nearest one this video has.
          const usual = Number(props.quality);
          if (props.quality === "audio" || props.quality === "best" || !Number.isFinite(usual)) {
            setChoice(props.quality === "audio" ? "audio" : "best");
          } else {
            const fits = found.heights.find((h) => h <= usual);
            setChoice(fits ? String(fits) : "best");
          }
        })
        .catch((e) => {
          if (stale) return;
          // The site wants a signed-in visitor: looking again would find the
          // same closed door, so the user is told where to sign in.
          if (needsSignIn(String(e))) {
            setLocked(true);
            return;
          }
          // The site downloader cannot read this page. The page itself is
          // opened out of sight and its video is caught as it plays, so the
          // same card can be shown.
          setCatchLink(link);
        })
        .finally(() => {
          if (!stale) setLooking(false);
        });
    }, 600);
    return () => {
      stale = true;
      window.clearTimeout(timer);
    };
  }, [link, props.quality]);

  // Finding a video by opening its page out of sight: the page is loaded where
  // it cannot be seen, its player is started, and what it plays is caught.
  // Nothing is downloaded; the card below offers what was found.
  useEffect(() => {
    if (!catchLink) return;
    setCaught([]);
    setCaughtSizes(new Map());
    sizeAsked.current = new Set();
    setPageTitle("");
    setPicture(null);
    setCaughtPick("");
    setCatching(true);
    let live = true;
    const listeners = [
      api.onBrowserMedia((item) => {
        if (!live) return;
        setCaught((list) => {
          const known = list.find((f) => f.url === item.url);
          if (known && !(item.master && !known.master) && item.heights.length <= known.heights.length) return list;
          return [...list.filter((f) => f.url !== item.url), item].slice(-40);
        });
      }),
      api.onBrowserTitle((title) => live && setPageTitle(title)),
      api.onBrowserPage((address) => live && setPicture(address)),
    ];
    // Far off to the side of the window, at the size of a normal browser.
    api.signinOpen(catchLink, { x: -6000, y: 0, width: 1280, height: 720 }, true).catch(() => live && setCatching(false));
    const presses = [2500, 6000, 11000, 18000].map((after) =>
      window.setTimeout(() => api.browserAutoplay().catch(() => {}), after),
    );
    const enough = window.setTimeout(() => live && setCatching(false), 30000);
    return () => {
      live = false;
      listeners.forEach((l) => l.then((f) => f()));
      presses.forEach(window.clearTimeout);
      window.clearTimeout(enough);
      api.signinClose().catch(() => {});
    };
  }, [catchLink]);

  // Each caught file is asked for its size once, to tell preview clips apart.
  useEffect(() => {
    for (const item of caught) {
      if (item.kind === "stream" || sizeAsked.current.has(item.url)) continue;
      sizeAsked.current.add(item.url);
      api
        .linkSize(item.url, item.referer)
        .catch(() => null)
        .then((size) => setCaughtSizes((known) => new Map(known).set(item.url, size)));
    }
  }, [caught]);

  const caughtOffers = catchLink ? caughtChoices(caught, caughtSizes) : [];
  const caughtChosen = caughtOffers.find((c) => c.key === caughtPick) ?? caughtOffers[0];
  // Once something real is caught, the looking is over.
  useEffect(() => {
    if (catching && caughtOffers.some((c) => c.item.kind === "stream" || !c.preview)) {
      const settle = window.setTimeout(() => setCatching(false), 3500);
      return () => window.clearTimeout(settle);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [catching, caught, caughtSizes]);

  const lockedSite = locked ? signInSiteOf(link) : null;
  const isPlaylist = !!profile && profile.site.endsWith("playlist");
  // Sites whose profiles are videos, where a resolution can be picked.
  const picksQuality =
    !!profile && (isPlaylist || profile.by === "ytdlp" || ["YouTube", "TikTok", "Pornhub"].includes(profile.site));
  /** The name the profile shows, once looked up; until then the one in its link. */
  const listedName = playlist?.title.trim() || "";
  /** Whose profile it is, from both places that say: the site's own answer first, its page otherwise. */
  const who: ProfileFace | null =
    face || stats
      ? {
          name: stats?.name ?? face?.name ?? null,
          // The page's picture is the small one, which is all a card needs.
          picture: face?.picture ?? stats?.picture ?? null,
          about: stats?.about ?? face?.about ?? null,
          banner: stats?.banner ?? null,
          posts: stats?.posts ?? null,
          media: stats?.media ?? null,
          followers: stats?.followers ?? null,
          following: stats?.following ?? null,
          joined: stats?.joined ?? null,
        }
      : null;
  const shownName =
    // The listing's own name, unless it only repeats the one in the link.
    (profile && listedName.toLowerCase() !== profile.name.toLowerCase() ? listedName : "") ||
    who?.name ||
    listedName ||
    profile?.name ||
    "";
  /** The profile's picture: its own from the page, else what the listing has (a channel's own on YouTube). */
  const cardPicture = isPlaylist
    ? (playlist?.thumbnail ?? null)
    : profile?.site === "YouTube"
      ? (playlist?.thumbnail ?? who?.picture ?? null)
      : (who?.picture ?? playlist?.thumbnail ?? null);
  const cardBanner = !isPlaylist && who?.banner && bannerFailed !== who.banner ? who.banner : null;
  /** "4,678 posts", "94.4K followers", "Joined Dec 2017": each only where the site says. */
  const numbers: [string, string][] = who
    ? ([
        who.posts != null ? [num(who.posts), t.views.home.numbers.posts(who.posts)] : null,
        who.media != null ? [num(who.media), t.views.home.numbers.media(who.media)] : null,
        who.followers != null ? [short(who.followers), t.views.home.numbers.followers(who.followers)] : null,
        who.following != null ? [short(who.following), t.views.home.numbers.following(who.following)] : null,
        who.joined ? ["", t.views.home.numbers.joined(joinedIn(who.joined))] : null,
      ].filter(Boolean) as [string, string][])
    : [];
  /** The site shows this profile to anyone; only the list of its posts is kept for signed-in visitors. */
  const profileShown = !!profile && !isPlaylist && !!(who?.name || who?.picture);
  // A profile the site lists only to signed-in visitors: downloading now
  // would fetch nothing, so Download waits for a sign-in.
  const gated = locked && !!profile && !isPlaylist;
  const downloadButton = useRef<HTMLButtonElement>(null);
  /** The note about signing in has been pointed at: its border stays lit,
   *  steady, for as long as the link is the same. */
  const [pointedAt, setPointedAt] = useState(false);
  useEffect(() => setPointedAt(false), [url]);
  /** Shakes the Download button and lights up the note about signing in. */
  function nudge() {
    setPointedAt(true);
    const el = downloadButton.current;
    if (!el) return;
    el.classList.remove("is-shaking");
    // Read the layout once, so the shake starts again on every click.
    void el.offsetWidth;
    el.classList.add("is-shaking");
  }
  /** The name in the link, when the card is titled with the shown name instead. */
  const handle =
    profile && shownName.toLowerCase() !== profile.name.toLowerCase() ? `@${profile.name}` : null;
  const plus = playlist?.more ? "+" : "";
  /** "27 videos", "214 photos", "5:39:18 total": what was counted. */
  const holds = playlist
    ? [
        playlist.count > 0 || playlist.photos === 0
          ? t.views.home.holds.videos(playlist.count, plus)
          : null,
        playlist.photos > 0 ? t.views.home.holds.photos(playlist.photos, plus) : null,
        playlist.duration ? t.views.home.holds.total(formatDuration(playlist.duration)) : null,
      ].filter(Boolean)
    : [];

  // The seconds the count has taken, ticking while it goes.
  useEffect(() => {
    setCountSeconds(0);
    if (!counting) return;
    const started = Date.now();
    const timer = window.setInterval(() => setCountSeconds(Math.floor((Date.now() - started) / 1000)), 1000);
    return () => window.clearInterval(timer);
  }, [counting]);
  /** Still waiting to hear whose profile it is. */
  const faceWaiting = !isPlaylist && (faceLoading || statsLoading) && !who?.picture;

  // Which resolutions the playlist really has: a few of its videos are looked
  // up, so the card does not offer 4K for a playlist that has none.
  useEffect(() => {
    if (!playlist || !picksQuality || playlist.samples.length === 0) return;
    let stale = false;
    setSizing(true);
    Promise.all(playlist.samples.map((sample) => api.mediaInfo(sample).catch(() => null))).then((found) => {
      if (stale) return;
      const heights = [...new Set(found.flatMap((f) => f?.heights ?? []))].sort((a, b) => b - a);
      setSizing(false);
      if (heights.length === 0) return;
      setListHeights(heights);
      // The usual quality, or the nearest one below it that the playlist has.
      setChoice((now) => {
        const wanted = Number(now);
        if (!Number.isFinite(wanted) || heights.includes(wanted)) return now;
        const fits = heights.find((h) => h <= wanted);
        return fits ? String(fits) : "best";
      });
    });
    return () => {
      stale = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [playlist]);
  const listChoices = listHeights
    ? [
        { value: "best", label: t.views.quality.best },
        ...listHeights.map((h) => ({
          value: String(h),
          label: h === 2160 ? t.views.quality.fourK(h) : t.common.quality.height(String(h)),
        })),
        { value: "audio", label: t.views.quality.audioOnly },
      ]
    : QUALITIES.map((q) => ({
        value: q.value,
        label:
          q.value === "best"
            ? t.views.quality.best
            : q.value === "audio"
              ? t.views.quality.audioOnly
              : q.value === "2160"
                ? t.views.quality.fourK(2160)
                : q.label,
      }));
  /** Still finding out what the playlist has: the choice is shown once that is known. */
  const checkingList = picksQuality && !listHeights && (counting || sizing);

  async function submit(e: FormEvent) {
    e.preventDefault();
    if (!valid) return;
    if (gated) {
      nudge();
      return;
    }
    // A video found on its page: the picked quality downloads.
    if (caughtChosen) {
      try {
        await api.browserKeepSignIn();
        await api.add({
          url: caughtChosen.item.url,
          media: caughtChosen.height
            ? { max_height: caughtChosen.height, audio_only: false }
            : qualityOptions(props.quality),
          via_site: true,
          headers: [["Referer", caughtChosen.item.referer]],
          filename: pageTitle.trim() || undefined,
        });
        actions.toast(t.views.downloadStarted);
        setUrl("");
        props.go("downloads");
      } catch (err) {
        actions.toast(String(err), "error");
      }
      return;
    }
    const media: MediaOptions | undefined = video || picksQuality
      ? choice === "audio"
        ? { max_height: null, audio_only: true }
        : { max_height: choice === "best" ? null : Number(choice), audio_only: false }
      : undefined;
    // One box for everything: the app works out what kind of link it is.
    if (await actions.submitLink(url, media)) setUrl("");
  }

  async function importFile() {
    if (busy) return;
    setBusy(true);
    try {
      const path = await api.pickLinkFile();
      if (!path) return;
      if (/\.torrent$/i.test(path)) {
        actions.openTorrent(path);
        return;
      }
      const links = await api.importLinks(path);
      if (links.length === 0) {
        actions.toast(t.views.home.noLinks, "error");
        return;
      }
      const media = qualityOptions(props.quality);
      await Promise.all(links.map((link) => api.add({ url: link, media })));
      actions.toast(t.views.home.added(links.length));
      props.go("downloads");
    } catch (err) {
      actions.toast(String(err), "error");
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="page page-home">
      <div className="welcome">
        <h1>{t.views.home.welcome}</h1>
        <p>{t.views.home.subtitle}</p>

        <label className="lang-switch">
          <Languages size={14} strokeWidth={2} aria-hidden="true" />
          <ChangeLanguageText />
          <Select<Locale>
            aria-label={changeLanguageLabel()}
            value={locale}
            onChange={(code) => void setLocale(code, true)}
            options={LOCALES.map((l) => ({ value: l.code, label: languageOptionLabel(l), lang: l.code }))}
          />
        </label>

        <form className="paste" onSubmit={submit}>
          <input
            type="text"
            value={url}
            onChange={(e) => setUrl(e.target.value)}
            placeholder={t.views.home.placeholder}
            aria-label={t.views.home.linkLabel}
            spellCheck={false}
            autoFocus
          />
          <button
            ref={downloadButton}
            type="submit"
            className="btn btn-primary"
            disabled={!valid}
            // Still takes the click, to point at what is missing; see `gated`.
            aria-disabled={gated || undefined}
            onAnimationEnd={(e) => e.currentTarget.classList.remove("is-shaking")}
          >
            {t.views.home.download}
          </button>
        </form>

        {locked && (
          <div className={`signin-needed${pointedAt ? " is-glowing" : ""}`} role="status">
            <KeyRound size={18} strokeWidth={2} aria-hidden="true" />
            <p>
              <strong>
                {profileShown
                  ? lockedSite
                    ? t.views.home.locked.profileShown(lockedSite)
                    : t.views.home.locked.profileShownThisSite
                  : lockedSite
                    ? t.views.home.locked.site(lockedSite)
                    : t.views.home.locked.thisSite}
              </strong>{" "}
              {lockedSite === "YouTube"
                ? t.views.home.locked.goYouTube
                : lockedSite
                  ? t.views.home.locked.goSite(lockedSite)
                  : t.views.home.locked.go}{" "}
              {t.views.home.locked.comeBack}
            </p>
            <button
              type="button"
              className="btn btn-small btn-primary"
              // Straight to the site's own sign-in where the app knows it;
              // otherwise the Sign in page, to pick the site there.
              onClick={() => (lockedSite ? props.signIn(lockedSite) : props.go("signin"))}
            >
              {t.views.home.locked.button}
            </button>
          </div>
        )}
        {((looking && !video) || (catchLink && catching && caughtOffers.length === 0)) && (
          <p className="video-looking" role="status">
            <span className="live-dot" aria-hidden="true" />{" "}
            {catchLink ? t.views.home.findingVideo : t.views.home.lookingPage}
          </p>
        )}
        {catchLink && !catching && caughtOffers.length === 0 && (
          <p className="video-looking" role="status">
            {t.views.home.noVideo}
            <button type="button" className="btn btn-small" onClick={() => actions.browse(catchLink)}>
              {t.views.home.openInBrowser}
            </button>
          </p>
        )}
        {catchLink && caughtChosen && (
          <section className="video-card is-caught" aria-label={t.views.home.caughtRegion}>
            {picture ? (
              <img className="video-thumb" src={picture} alt="" referrerPolicy="no-referrer" />
            ) : (
              <span className="video-thumb video-thumb-empty" aria-hidden="true" />
            )}
            <div className="video-body">
              <h2 className="video-title" title={pageTitle}>
                {pageTitle.trim() || mediaName(catchLink)}
              </h2>
              <p className="video-meta">
                {catching ? t.views.home.foundStillLooking : t.views.home.foundOnPage}
              </p>
              <div className="video-choices" role="radiogroup" aria-label={t.views.home.qualityLabel}>
                {caughtOffers.map((offer) => (
                  <button
                    key={offer.key}
                    type="button"
                    role="radio"
                    aria-checked={caughtChosen.key === offer.key}
                    className={`video-choice${caughtChosen.key === offer.key ? " is-on" : ""}`}
                    onClick={() => setCaughtPick(offer.key)}
                  >
                    {offer.label}
                  </button>
                ))}
              </div>
            </div>
          </section>
        )}
        {profile && (
          <section
            className={`video-card profile-card${cardBanner ? " has-banner" : ""}`}
            aria-label={t.views.home.profileRegion}
          >
            {cardBanner && (
              <img
                className="profile-banner"
                src={cardBanner}
                alt=""
                referrerPolicy="no-referrer"
                onError={() => setBannerFailed(cardBanner)}
              />
            )}
            {cardPicture && pictureFailed !== cardPicture ? (
              <img
                // A playlist shows a video's picture; a profile shows whose it is.
                className={isPlaylist ? "video-thumb" : "profile-face"}
                src={cardPicture}
                alt=""
                referrerPolicy="no-referrer"
                onError={() => setPictureFailed(cardPicture)}
              />
            ) : faceWaiting || (!isPlaylist && profile.site === "YouTube" && !playlist && counting) ? (
              <span className="profile-face shimmer" aria-hidden="true" />
            ) : (
              <span className="profile-badge" aria-hidden="true">
                <Users size={22} strokeWidth={2} />
              </span>
            )}
            <div className="video-body">
              {isPlaylist ? (
                <>
                  {!playlist && counting ? (
                    <div className="shimmer-lines" aria-hidden="true">
                      <span className="shimmer shimmer-title" />
                      <span className="shimmer shimmer-meta" />
                    </div>
                  ) : (
                    <>
                      <h2 className="video-title" title={playlist?.title}>
                        {playlist ? playlist.title : profile.site}
                      </h2>
                      <p className="video-meta">
                        {playlist
                          ? [...holds, playlist.channel, t.views.home.playlist].filter(Boolean).join(" · ")
                          : t.views.home.playlist}
                      </p>
                    </>
                  )}
                </>
              ) : (
                <>
                  {/* Each part shows the moment its answer comes, in place of
                      its own placeholder; a part that comes back empty goes. */}
                  <h2 className="video-title is-arriving" key={shownName} title={shownName}>{shownName}</h2>
                  <p className="video-meta">
                    {[handle, ...holds, t.views.home.profileOf(profile.site)].filter(Boolean).join(" · ")}
                    {/* what it holds joins this line once counted */}
                    {counting && !playlist && <span className="shimmer shimmer-inline" aria-hidden="true" />}
                  </p>
                  {who?.about ? (
                    <p className="profile-about is-arriving" title={who.about}>{who.about}</p>
                  ) : (
                    faceWaiting && (
                      <div className="shimmer-lines" aria-hidden="true">
                        <span className="shimmer shimmer-about" />
                      </div>
                    )
                  )}
                  {/* What the profile shows any visitor: its numbers, and when it joined. */}
                  {numbers.length > 0 ? (
                    <p className="profile-numbers is-arriving">
                      {numbers.map(([figure, what]) => (
                        <span key={what}>
                          {figure && <strong>{figure}</strong>} {what}
                        </span>
                      ))}
                    </p>
                  ) : (
                    statsLoading && (
                      <div className="shimmer-numbers" aria-hidden="true">
                        {[58, 74, 66].map((width, i) => (
                          <span key={i} className="shimmer shimmer-number" style={{ width }} />
                        ))}
                      </div>
                    )
                  )}
                </>
              )}
              {checkingList && (
                <div className="video-choices" aria-hidden="true">
                  {[64, 52, 52, 46, 46].map((width, i) => (
                    <span key={i} className="shimmer shimmer-chip" style={{ width }} />
                  ))}
                </div>
              )}
              {(counting || checkingList) && (
                <div className="card-loading" role="status">
                  <span>
                    {!counting
                      ? t.views.home.loading.resolutions
                      : isPlaylist
                        ? t.views.home.loading.playlist
                        : countSeconds < 6
                          ? t.views.home.loading.counting
                          : // A slow site is said to be slow; nothing waits on the count.
                            t.views.home.loading.slow(countSeconds, profile.site)}
                  </span>
                  <div className="bar bar-downloading bar-busy" />
                </div>
              )}
              {picksQuality && !checkingList && (
                <div className="video-choices" role="radiogroup" aria-label={t.views.home.resolution}>
                  {listChoices.map((q) => (
                    <button
                      key={q.value}
                      type="button"
                      role="radio"
                      aria-checked={choice === q.value}
                      className={`video-choice${choice === q.value ? " is-on" : ""}`}
                      onClick={() => setChoice(q.value)}
                    >
                      {q.label}
                    </button>
                  ))}
                </div>
              )}
            </div>
          </section>
        )}
        {profile && (
          <p className="profile-note">
            <Info size={14} strokeWidth={2} aria-hidden="true" />
            {isPlaylist ? (
              <span>{t.views.home.playlistNote}</span>
            ) : (
              <span>
                {t.views.home.profileNoteBefore}
                <strong>{t.views.home.profileFolder(profile.name, profile.site)}</strong>
                {picksQuality ? t.views.home.profileNoteAfterQuality : t.views.home.profileNoteAfter}
              </span>
            )}
          </p>
        )}
        {video && (
          <section className="video-card" aria-label={t.views.home.videoRegion}>
            {video.thumbnail ? (
              <img className="video-thumb" src={video.thumbnail} alt="" referrerPolicy="no-referrer" />
            ) : (
              <span className="video-thumb video-thumb-empty" aria-hidden="true" />
            )}
            <div className="video-body">
              <h2 className="video-title" title={video.title}>{video.title}</h2>
              <p className="video-meta">
                {[video.site, video.duration ? formatDuration(video.duration) : null]
                  .filter(Boolean)
                  .join(" · ")}
              </p>
              <div className="video-choices" role="radiogroup" aria-label={t.views.home.resolution}>
                {[
                  { value: "best", label: t.views.quality.best },
                  ...video.heights.map((h) => ({
                    value: String(h),
                    label: h === 2160 ? t.views.quality.fourK(h) : t.common.quality.height(String(h)),
                  })),
                  { value: "audio", label: t.views.quality.audioOnly },
                ].map((option) => (
                  <button
                    key={option.value}
                    type="button"
                    role="radio"
                    aria-checked={choice === option.value}
                    className={`video-choice${choice === option.value ? " is-on" : ""}`}
                    onClick={() => setChoice(option.value)}
                  >
                    {option.label}
                  </button>
                ))}
              </div>
            </div>
          </section>
        )}

        <span className="or">{t.views.home.or}</span>

        <div className="welcome-actions">
          <button
            type="button"
            className="btn btn-pill"
            title={t.views.home.addFromBrowserTitle}
            onClick={() => actions.openAdd("", "scan")}
          >
            <Puzzle size={17} strokeWidth={0} fill="currentColor" aria-hidden="true" /> {t.views.home.addFromBrowser}
          </button>
          <button
            type="button"
            className="btn btn-pill"
            title={t.views.home.browseFilesTitle}
            disabled={busy}
            onClick={importFile}
          >
            <Folder size={17} strokeWidth={0} fill="currentColor" aria-hidden="true" /> {t.views.home.browseFiles}
          </button>
        </div>

        {running.length > 0 && (
          <section className="glance" aria-label={t.views.home.downloadingRegion}>
            <button type="button" className="live" onClick={() => props.go("downloads")}>
              <span className="live-dot" aria-hidden="true" />
              {t.views.home.downloading(running.length, formatSpeed(speed))}
              <ArrowRight size={15} strokeWidth={2} aria-hidden="true" />
            </button>
            {/* The first few, each with how far along it is; the rest are on the Downloads page. */}
            <ul className="glance-list">
              {running.slice(0, 3).map((d) => {
                const percent = percentOf(d);
                const checked = checkPercent(d);
                return (
                  <li key={d.id}>
                    <FileIcon d={d} />
                    <span className="glance-name" title={displayName(d)}>{displayName(d)}</span>
                    <span className="glance-figure">
                      {checked != null
                        ? t.views.home.checking(checked)
                        : percent != null
                        ? pct(percent)
                        : d.status === "connecting"
                          ? t.common.status.connecting
                          : formatBytes(d.downloaded)}
                    </span>
                    <span className={`glance-bar${percent == null ? " is-open" : ""}`} aria-hidden="true">
                      <span style={{ width: `${percent ?? 100}%` }} />
                    </span>
                  </li>
                );
              })}
            </ul>
          </section>
        )}
      </div>

      <ul className="features" aria-label={t.views.home.featuresRegion}>
        {FEATURES.map((feature) => (
          <li key={feature.id}>
            <feature.icon size={24} strokeWidth={2} aria-hidden="true" />
            <strong>{t.views.home.features[feature.id]}</strong>
            <span>{t.views.home.features[`${feature.id}Detail`]}</span>
          </li>
        ))}
      </ul>
    </div>
  );
}

/* ---------- downloads ---------- */

type Chip = "all" | "running" | "completed" | "paused" | "failed" | "queued" | "deleted";
type Sort = "newest" | "oldest" | "name" | "largest" | "smallest" | "fastest";
type TypeFilter = "all" | Category;
type DateFilter = "any" | "today" | "week" | "month";
type SizeFilter = "any" | "small" | "medium" | "large";

// Labels are getters, read at render time, so they follow the language.
const CHIPS: { id: Chip; readonly label: string; match: (d: Download) => boolean }[] = (
  [
    ["all", () => true],
    ["running", isRunning],
    // Completed means the file is there; one deleted since has a chip of its own.
    ["completed", (d) => d.status === "completed" && !isDeleted(d)],
    ["paused", (d) => d.status === "paused"],
    ["failed", (d) => d.status === "failed"],
    ["queued", (d) => d.status === "queued"],
    ["deleted", isDeleted],
  ] as [Chip, (d: Download) => boolean][]
).map(([id, match]) => ({
  id,
  match,
  get label() {
    return t.views.downloads.chips[id];
  },
}));

/** Options of a menu whose labels are read from `labels` at render time. */
function labelled<T extends string>(values: T[], labels: () => Record<T, string>) {
  return values.map((value) => ({
    value,
    get label() {
      return labels()[value];
    },
  }));
}

const SORTS = labelled<Sort>(["newest", "oldest", "name", "largest", "smallest", "fastest"], () => t.views.downloads.sorts);

const TYPE_ICONS: Record<TypeFilter, LucideIcon> = {
  all: ListFilter,
  video: Film,
  audio: Music,
  image: ImageGlyph,
  document: FileText,
  archive: Archive,
  software: Package,
  other: FileGlyph,
};
const TYPES = (Object.keys(TYPE_ICONS) as TypeFilter[]).map((value) => ({
  value,
  icon: TYPE_ICONS[value],
  get label() {
    return t.views.downloads.types[value];
  },
}));

const DATES = labelled<DateFilter>(["any", "today", "week", "month"], () => t.views.downloads.dates);

const SIZES = labelled<SizeFilter>(["any", "small", "medium", "large"], () => t.views.downloads.sizes);

const MB = 1024 * 1024;

function matchesDate(d: Download, f: DateFilter): boolean {
  if (f === "any") return true;
  const age = Date.now() / 1000 - d.created_at;
  if (f === "today") {
    return new Date(d.created_at * 1000).toDateString() === new Date().toDateString();
  }
  return age <= (f === "week" ? 7 : 30) * 86400;
}

function matchesSize(d: Download, f: SizeFilter): boolean {
  if (f === "any") return true;
  if (d.total == null) return false;
  if (f === "small") return d.total < 100 * MB;
  if (f === "medium") return d.total >= 100 * MB && d.total <= 1024 * MB;
  return d.total > 1024 * MB;
}

function sortDownloads(list: Download[], sort: Sort): Download[] {
  const out = [...list];
  switch (sort) {
    case "newest":
      return out.sort((a, b) => b.id - a.id);
    case "oldest":
      return out.sort((a, b) => a.id - b.id);
    case "name":
      return out.sort((a, b) => a.filename.localeCompare(b.filename));
    case "largest":
      return out.sort((a, b) => (b.total ?? 0) - (a.total ?? 0));
    case "smallest":
      return out.sort((a, b) => (a.total ?? Infinity) - (b.total ?? Infinity));
    case "fastest":
      return out.sort((a, b) => b.speed - a.speed);
  }
}

/* ---------- transfer summary ---------- */

/** Total download speed, read once a second. Kept for as long as the app is
 *  open, so the graph is still there after a visit to another page. */
const speedLog: { at: number; speed: number }[] = [];
const SPEED_LOG_LENGTH = 60;
const SPEED_LOG_MAX_AGE = 90_000;

/** A small filled line of recent readings; flat when there is nothing to show. */
function MiniGraph({ values }: { values: number[] }) {
  const fade = useId().replace(/[^a-zA-Z0-9]/g, "");
  const w = 240;
  const h = 40;
  const top = 4;
  const bottom = h - 2;
  const shown = values.length > 1 ? values : [0, 0];
  // Headroom above the fastest reading, so a steady speed is a line, not a full block.
  const high = Math.max(...shown, 1) * 1.4;
  const pts = shown.map(
    (v, i) => [(i / (shown.length - 1)) * w, bottom - (v / high) * (bottom - top)] as const,
  );
  // A soft curve: each reading is a control point, the line passes midway between them.
  let line = `M${pts[0][0].toFixed(1)},${pts[0][1].toFixed(1)}`;
  for (let i = 1; i < pts.length - 1; i++) {
    const midX = (pts[i][0] + pts[i + 1][0]) / 2;
    const midY = (pts[i][1] + pts[i + 1][1]) / 2;
    line += ` Q${pts[i][0].toFixed(1)},${pts[i][1].toFixed(1)} ${midX.toFixed(1)},${midY.toFixed(1)}`;
  }
  const last = pts[pts.length - 1];
  line += ` L${last[0].toFixed(1)},${last[1].toFixed(1)}`;
  return (
    <svg className="stat-graph" viewBox={`0 0 ${w} ${h}`} preserveAspectRatio="none" aria-hidden="true">
      <defs>
        <linearGradient id={fade} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" stopColor="currentColor" stopOpacity="0.22" />
          <stop offset="1" stopColor="currentColor" stopOpacity="0" />
        </linearGradient>
      </defs>
      <path d={`${line} L${w},${h} L0,${h} Z`} fill={`url(#${fade})`} />
      <path d={line} className="stat-line" />
    </svg>
  );
}

/** Speed now, as a graph of the last minute, and how much has been downloaded.
 *  Shown at the foot of the sidebar, so it is in view on every page. */
export function TransferSummary() {
  const { downloads } = useApp();
  const latest = useRef(downloads);
  latest.current = downloads;
  const [, redraw] = useState(0);

  useEffect(() => {
    const read = () => {
      const now = Date.now();
      while (speedLog.length > 0 && now - speedLog[0].at > SPEED_LOG_MAX_AGE) speedLog.shift();
      speedLog.push({ at: now, speed: latest.current.filter(isRunning).reduce((sum, d) => sum + d.speed, 0) });
      if (speedLog.length > SPEED_LOG_LENGTH) speedLog.shift();
      redraw((n) => n + 1);
    };
    read();
    const timer = window.setInterval(read, 1000);
    return () => window.clearInterval(timer);
  }, []);

  // What is in hand: downloading, connecting, waiting for a slot, or paused.
  // With nothing in hand there is nothing to read here, and the sidebar is
  // left clear. (The speed log above keeps running, so the graph has its
  // last minute ready the moment something starts.)
  const inHand = downloads.filter((d) => isRunning(d) || d.status === "queued" || d.status === "paused");
  if (inHand.length === 0) return null;

  const speed = inHand.filter(isRunning).reduce((sum, d) => sum + d.speed, 0);
  // Nothing is coming down because what was downloaded is being checked.
  const checkingOnly = speed === 0 && inHand.some((d) => checkPercent(d) != null);
  const done = inHand.reduce((sum, d) => sum + d.downloaded, 0);
  // Everything in hand will have downloaded once it is finished. A download
  // whose size is not known yet counts for what it has so far. What is
  // already finished is History's business, not this meter's.
  const total = inHand.reduce((sum, d) => sum + Math.max(d.total ?? 0, d.downloaded), 0);
  const share = total > 0 ? Math.min(100, (done / total) * 100) : 0;

  return (
    <section className="stats" aria-label={t.views.summary.region}>
      <div className="stat" title={t.views.summary.speed}>
        {/* The icon stands in for the word, and moves while anything is coming down. */}
        <div className="stat-trace">
          <MiniGraph values={speedLog.map((s) => s.speed)} />
          <DownloadGlyph
            className={`stat-icon${speed > 0 ? " is-active" : ""}`}
            size={16}
            strokeWidth={2.25}
            aria-hidden="true"
          />
        </div>
        {checkingOnly ? (
          <strong title={t.views.summary.checkingTitle}>{t.views.summary.checking}</strong>
        ) : (
          <strong aria-label={t.views.summary.speedValue(formatSpeed(speed))}>{formatSpeed(speed)}</strong>
        )}
      </div>
      <div className="stat" title={t.views.summary.sizeTitle}>
        <span className="stat-graph stat-meter" aria-hidden="true">
          <span className="bar bar-tall">
            <span className="fill" style={{ width: `${share}%` }} />
          </span>
          <small>{t.views.summary.percent(Math.floor(share))}</small>
        </span>
        <strong>
          {formatBytes(done)} <span className="stat-of">/ {formatBytes(total)}</span>
        </strong>
        <span>{t.views.summary.total}</span>
      </div>
    </section>
  );
}

export function DownloadsView(props: RowHandlers & { clearSelection: () => void; go: (nav: Nav) => void }) {
  const { downloads: everything, actions } = useApp();
  // This page is for what can be used or is on its way: everything running,
  // waiting, failed or still on disk, and what was deleted in the last few
  // days. Older deleted downloads are in History.
  const downloads = useMemo(() => everything.filter((d) => onDownloadsPage(d)), [everything]);
  const [chip, setChip] = useState<Chip>(() => loadPref<Chip>("chip", "all", CHIPS.map((c) => c.id)));
  const [sort, setSort] = useState<Sort>(() => loadPref<Sort>("sort", "newest", SORTS.map((s) => s.value)));
  const [type, setType] = useState<TypeFilter>("all");
  const [date, setDate] = useState<DateFilter>("any");
  const [size, setSize] = useState<SizeFilter>("any");
  const [query, setQuery] = useState("");
  const [filtersOpen, setFiltersOpen] = useState(false);
  const searchRef = useRef<HTMLInputElement>(null);

  const narrowed = type !== "all" || date !== "any" || size !== "any" || query.trim() !== "";

  const visible = useMemo(() => {
    const q = query.trim().toLowerCase();
    const match = CHIPS.find((c) => c.id === chip)!.match;
    return sortDownloads(
      downloads.filter(
        (d) =>
          match(d) &&
          (type === "all" || categoryOf(d) === type) &&
          matchesDate(d, date) &&
          matchesSize(d, size) &&
          (!q || d.filename.toLowerCase().includes(q) || d.url.toLowerCase().includes(q)),
      ),
      sort,
    );
  }, [downloads, chip, sort, type, date, size, query]);
  const visibleIds = useMemo(() => visible.map((d) => d.id), [visible]);
  const selected = downloads.filter((d) => props.selection.has(d.id));

  function clearFilters() {
    setType("all");
    setDate("any");
    setSize("any");
    setQuery("");
    setChip("all");
    savePref("chip", "all");
  }

  return (
    <div className="page page-downloads">
      <header className="page-head">
        <h1>{t.views.downloads.title}</h1>
        <span className="spacer" />
        <IconBtn
          outlined
          icon={FolderOpen}
          label={t.views.downloads.openFolder}
          onClick={() => api.openDownloadDir().catch((e) => actions.toast(String(e), "error"))}
        />
        <IconBtn
          outlined
          icon={Search}
          label={t.views.downloads.search}
          active={filtersOpen && query !== ""}
          onClick={() => {
            setFiltersOpen(true);
            window.setTimeout(() => searchRef.current?.focus(), 0);
          }}
        />
        <IconBtn
          outlined
          icon={ListFilter}
          label={filtersOpen ? t.views.downloads.hideFilters : t.views.downloads.showFilters}
          active={filtersOpen || narrowed}
          onClick={() => setFiltersOpen((v) => !v)}
        />
        <Dropdown
          label={t.views.downloads.sortBy}
          value={sort}
          options={SORTS}
          onChange={(v) => {
            setSort(v);
            savePref("sort", v);
          }}
        />
      </header>

      <div className="chips" role="tablist" aria-label={t.views.downloads.filterByStatus}>
        {CHIPS.map((c) => {
          const count = downloads.filter(c.match).length;
          // Queued only earns a chip while something is actually waiting, and
          // Deleted only while a deleted download is still shown here.
          if ((c.id === "queued" || c.id === "deleted") && count === 0 && chip !== c.id) return null;
          return (
            <button
              key={c.id}
              type="button"
              role="tab"
              aria-selected={chip === c.id}
              className={`chip${chip === c.id ? " is-active" : ""}`}
              onClick={() => {
                setChip(c.id);
                savePref("chip", c.id);
              }}
            >
              {c.label} <span className="chip-count">{count}</span>
            </button>
          );
        })}
      </div>

      {filtersOpen && (
        <div className="filters">
          <Dropdown label={t.views.downloads.type} value={type} options={TYPES} onChange={setType} showValue />
          <Dropdown label={t.views.downloads.date} value={date} options={DATES} onChange={setDate} showValue />
          <Dropdown label={t.views.downloads.size} value={size} options={SIZES} onChange={setSize} showValue />
          <label className="search">
            <Search size={16} strokeWidth={2} aria-hidden="true" />
            <input
              ref={searchRef}
              type="text"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder={t.views.downloads.searchPlaceholder}
              aria-label={t.views.downloads.search}
              spellCheck={false}
            />
            {query && (
              <button type="button" className="search-clear" aria-label={t.views.downloads.clearSearch} onClick={() => setQuery("")}>
                <X size={14} strokeWidth={2.25} aria-hidden="true" />
              </button>
            )}
          </label>
        </div>
      )}

      {selected.length > 1 && (
        <div className="bulk" role="toolbar" aria-label={t.views.downloads.bulkRegion}>
          <strong>{t.views.downloads.selected(selected.length)}</strong>
          <button
            type="button"
            className="btn btn-small"
            disabled={!selected.some(canPause)}
            onClick={() => actions.pause(selected.filter(canPause).map((d) => d.id))}
          >
            <Pause size={14} strokeWidth={0} fill="currentColor" aria-hidden="true" /> {t.views.downloads.pause}
          </button>
          <button
            type="button"
            className="btn btn-small"
            disabled={!selected.some(canResume)}
            onClick={() => actions.resume(selected.filter(canResume).map((d) => d.id))}
          >
            <Play size={14} strokeWidth={0} fill="currentColor" aria-hidden="true" /> {t.views.downloads.resume}
          </button>
          <button type="button" className="btn btn-small" onClick={() => actions.remove(selected.map((d) => d.id))}>
            <X size={15} strokeWidth={2} aria-hidden="true" /> {t.views.downloads.remove}
          </button>
          <span className="spacer" />
          <button type="button" className="link" onClick={props.clearSelection}>{t.views.downloads.clearSelection}</button>
        </div>
      )}

      {visible.length === 0 ? (
        <div className="empty">
          {downloads.length === 0 ? (
            <>
              <h2>{t.views.downloads.emptyTitle}</h2>
              <p>{t.views.downloads.emptyText}</p>
              <button type="button" className="btn btn-primary" onClick={() => props.go("home")}>
                {t.views.downloads.goHome}
              </button>
            </>
          ) : (
            <>
              <h2>{t.views.downloads.noMatchTitle}</h2>
              <p>{t.views.downloads.noMatchText}</p>
              <button type="button" className="btn" onClick={clearFilters}>{t.views.downloads.clearFilters}</button>
            </>
          )}
        </div>
      ) : (
        <ul className="list" role="listbox" aria-multiselectable="true" aria-label={t.views.downloads.title}>
          {visible.map((d) => (
            <DownloadRow
              key={d.id}
              d={d}
              selected={props.selection.has(d.id)}
              onSelect={(e) => props.onSelect(d.id, e, visibleIds)}
              onMenu={(x, y) => props.onMenu(d, x, y)}
            />
          ))}
        </ul>
      )}
    </div>
  );
}

/* ---------- settings ---------- */

export type Theme = "system" | "light" | "dark";
export type Density = "comfortable" | "compact";

type Section = "general" | "downloads" | "network" | "performance" | "scheduler" | "appearance" | "advanced";
export type SettingsSection = Section;
/** The sections of Settings, in order: one tab each. */
export const SETTINGS_SECTIONS: { id: Section; readonly label: string; icon: LucideIcon }[] = (
  [
    ["general", SettingsGlyph],
    ["downloads", DownloadGlyph],
    ["network", Wifi],
    ["performance", Gauge],
    ["scheduler", CalendarClock],
    ["appearance", Brush],
    ["advanced", Wrench],
  ] as [Section, LucideIcon][]
).map(([id, icon]) => ({
  id,
  icon,
  // Read at render time, so it follows the language.
  get label() {
    return t.views.settings.sections[id];
  },
}));

/** The stops of the speed slider, 100 KB/s to 100 MB/s: fine at the slow end, coarse at the fast end. */
const SPEED_STOPS = [0.1, 0.25, 0.5, 0.75, 1, 1.5, 2, 3, 4, 5, 7.5, 10, 15, 20, 30, 40, 50, 75, 100].map((mb) =>
  Math.round(mb * MB),
);
const DEFAULT_SPEED_LIMIT = 5 * MB;

/** The slider stop nearest to a limit, so a limit set elsewhere still has a place on it. */
function speedStopOf(limit: number): number {
  let best = 0;
  for (let i = 1; i < SPEED_STOPS.length; i++) {
    if (Math.abs(SPEED_STOPS[i] - limit) < Math.abs(SPEED_STOPS[best] - limit)) best = i;
  }
  return best;
}

type Shortcut = keyof typeof t.views.settings.shortcuts;
/** Keys, and what they do (looked up in `t.views.settings.shortcuts` at render time). */
export const SHORTCUTS: [string, Shortcut][] = [
  ["Ctrl N", "add"],
  ["Ctrl V", "paste"],
  ["Ctrl K", "search"],
  ["Ctrl 1 / 2", "pages"],
  ["Ctrl ,", "settings"],
  ["Ctrl A", "selectAll"],
  ["Space", "toggle"],
  ["Delete", "remove"],
  ["Ctrl Shift P", "pauseAll"],
  ["Ctrl Shift R", "resumeAll"],
  ["Esc", "close"],
];

/** A setting shown for a feature the app cannot do yet: visible, switched off, labelled. */
/** One setting on a line: what it is, and a word about it, on the left; its control on the right. */
/** "Change language" in the language switched on; in any language but
 *  English the English words follow on a pill, so someone who switched to a
 *  language they cannot read can still find the way back. */
function ChangeLanguageText() {
  if (locale === "en") return <span>{ENGLISH_CHANGE_LANGUAGE}</span>;
  return (
    <span className="change-language">
      {t.common.changeLanguage}
      <span className="lang-pill" lang="en" dir="ltr">
        {ENGLISH_CHANGE_LANGUAGE}
      </span>
    </span>
  );
}

function SettingRow(props: { label: ReactNode; id?: string; hint?: ReactNode; wide?: boolean; children: ReactNode }) {
  return (
    <div className={`setting-row${props.wide ? " is-wide" : ""}`}>
      <div className="setting-text">
        <span id={props.id}>{props.label}</span>
        {props.hint && <small>{props.hint}</small>}
      </div>
      <div className="setting-control">{props.children}</div>
    </div>
  );
}

/** A setting that is on or off: the whole line is the switch. Without `onChange` it is not built yet. */
function SettingToggle(props: { label: string; hint?: string; checked?: boolean; onChange?: (on: boolean) => void }) {
  const soon = !props.onChange;
  return (
    <label className={`setting-row toggle${soon ? " is-disabled" : ""}`}>
      <span className="setting-text">
        <span>
          {props.label}
          {soon && <span className="soon">{t.views.settings.soon}</span>}
        </span>
        {props.hint && <small>{props.hint}</small>}
      </span>
      <input
        type="checkbox"
        disabled={soon}
        checked={!soon && !!props.checked}
        onChange={(e) => props.onChange?.(e.target.checked)}
      />
      <span className="toggle-dot" aria-hidden="true" />
    </label>
  );
}

function Segmented<T extends string>(props: {
  label: string;
  value: T;
  options: { value: T; label: string }[];
  onChange: (v: T) => void;
}) {
  return (
    <div className="segmented" role="radiogroup" aria-label={props.label}>
      {props.options.map((o) => (
        <button
          key={o.value}
          type="button"
          role="radio"
          aria-checked={props.value === o.value}
          className={props.value === o.value ? "is-active" : undefined}
          onClick={() => props.onChange(o.value)}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

export function SettingsView(props: {
  theme: Theme;
  onTheme: (t: Theme) => void;
  density: Density;
  onDensity: (d: Density) => void;
  clipWatch: boolean;
  onClipWatch: (on: boolean) => void;
  askFolder: boolean;
  onAskFolder: (on: boolean) => void;
  progressStyle: ProgressStyle;
  onProgressStyle: (p: ProgressStyle) => void;
  quality: string;
  onQuality: (q: string) => void;
  onSettings: (s: Settings) => void;
  /** How the app sits on the desktop; null until known. */
  appPrefs: AppPrefs | null;
  onAppPrefs: (prefs: AppPrefs) => void;
  /** Which tab is in front, and what picks another. */
  section: Section;
  onSection: (section: Section) => void;
}) {
  const { settings, actions } = useApp();
  const section = props.section;
  /** The turn from one tab to another that is on its way, to know which one ends last. */
  const turning = useRef<object | null>(null);

  /** Brings another tab to the front. Where the webview can, the old panel
   *  and the new one are shown as one move: what is leaving slides off the
   *  way the tabs run, what is arriving follows it in, and the panel's
   *  height glides to fit. Anywhere else the tab simply changes. */
  function turnTo(next: Section) {
    if (next === section) return;
    const order = SETTINGS_SECTIONS.map((s) => s.id);
    const root = document.documentElement;
    const start = (
      document as Document & { startViewTransition?: (change: () => void) => { finished: Promise<unknown> } }
    ).startViewTransition;
    if (!start || window.matchMedia("(prefers-reduced-motion: reduce)").matches) {
      props.onSection(next);
      return;
    }
    // Which way the tabs run from here: the content travels the same way.
    root.dataset.settingsTurn = order.indexOf(next) > order.indexOf(section) ? "forward" : "back";
    const turn = start.call(document, () => flushSync(() => props.onSection(next)));
    turning.current = turn;
    const settle = () => {
      // A newer turn has taken over: it clears up after itself.
      if (turning.current === turn) delete root.dataset.settingsTurn;
    };
    turn.finished.then(settle, settle);
  }
  const [tools, setTools] = useState<ToolStatus | null>(null);
  /** The limit last set by hand, to come back to after a spell of Unlimited. */
  const lastLimit = useRef(DEFAULT_SPEED_LIMIT);
  useEffect(() => {
    api.tools().then(setTools).catch(() => setTools(null));
  }, []);

  if (!settings) return <div className="page" aria-busy="true" />;

  const update = (patch: Partial<Settings>) => props.onSettings({ ...settings, ...patch });
  const clamp = (value: string, min: number, max: number) => {
    const n = Math.round(Number(value));
    return Number.isFinite(n) ? Math.min(max, Math.max(min, n)) : min;
  };
  const limited = settings.speed_limit > 0;
  if (limited) lastLimit.current = settings.speed_limit;
  const speedStop = speedStopOf(settings.speed_limit);

  async function browse() {
    const dir = await api.pickFolder(settings!.download_dir);
    if (dir) update({ download_dir: dir });
  }

  const folderRow = (
    <SettingRow label={t.views.settings.folder} id="set-dir" wide>
      <div className="input-with-button">
        <input type="text" readOnly aria-labelledby="set-dir" value={settings.download_dir} />
        <button type="button" title={t.views.settings.chooseFolder} aria-label={t.views.settings.chooseFolder} onClick={browse}>
          <Folder size={17} strokeWidth={2} aria-hidden="true" />
        </button>
      </div>
    </SettingRow>
  );

  return (
    <div className="page page-settings">
      <div className="settings">
        <div className="settings-side">
          <h1>{t.views.settings.title}</h1>
        </div>

        {/* Tabs as a browser has them: the one in front is part of the panel under it. */}
        <div className="settings-main">
          <div className="settings-nav settings-tabs" role="tablist" aria-label={t.views.settings.sectionsLabel}>
            {SETTINGS_SECTIONS.map((s) => (
              <button
                key={s.id}
                type="button"
                role="tab"
                id={`settings-tab-${s.id}`}
                aria-selected={section === s.id}
                aria-controls="settings-panel"
                className={section === s.id ? "is-active" : undefined}
                // In a narrow window only the mark shows; its name is still here.
                title={s.label}
                onClick={() => turnTo(s.id)}
              >
                <s.icon size={15} strokeWidth={2} aria-hidden="true" />
                <span>{s.label}</span>
              </button>
            ))}
          </div>
          <div
            className="settings-panel"
            id="settings-panel"
            role="tabpanel"
            aria-labelledby={`settings-tab-${section}`}
          >
        <div className="settings-body" key={section}>
          {section === "general" && (
            <>
              <h2>{t.views.settings.sections.general}</h2>
              <div className="setting-card">
                <SettingToggle
                  label={t.views.settings.general.clipWatch}
                  checked={props.clipWatch}
                  onChange={props.onClipWatch}
                />
                <SettingToggle
                  label={t.views.settings.general.askFolder}
                  checked={props.askFolder}
                  onChange={props.onAskFolder}
                />
              </div>
              <div className="setting-card">
                <SettingRow label={<ChangeLanguageText />} id="set-language" hint={t.common.languageHint}>
                  <Select<Locale>
                    aria-labelledby="set-language"
                    value={locale}
                    onChange={(code) => void setLocale(code, true)}
                    options={LOCALES.map((l) => ({ value: l.code, label: languageOptionLabel(l), lang: l.code }))}
                  />
                </SettingRow>
                {folderRow}
              </div>
              {props.appPrefs && (
                <div className="setting-card">
                  <SettingToggle
                    label={t.views.settings.general.startup}
                    hint={t.views.settings.general.startupHint}
                    checked={props.appPrefs.start_with_system}
                    onChange={(on) => props.onAppPrefs({ ...props.appPrefs!, start_with_system: on })}
                  />
                  <SettingToggle
                    label={t.views.settings.general.tray}
                    hint={t.views.settings.general.trayHint}
                    checked={props.appPrefs.close_to_tray}
                    onChange={(on) => props.onAppPrefs({ ...props.appPrefs!, close_to_tray: on })}
                  />
                  <SettingToggle
                    label={t.views.settings.general.notifications}
                    hint={t.views.settings.general.notificationsHint}
                    checked={props.appPrefs.notifications}
                    onChange={(on) => props.onAppPrefs({ ...props.appPrefs!, notifications: on })}
                  />
                  <SettingToggle
                    label={t.views.settings.general.hardware}
                    hint={t.views.settings.general.hardwareHint}
                    checked={props.appPrefs.hardware_acceleration}
                    onChange={(on) => props.onAppPrefs({ ...props.appPrefs!, hardware_acceleration: on })}
                  />
                </div>
              )}
            </>
          )}

          {section === "downloads" && (
            <>
              <h2>{t.views.settings.sections.downloads}</h2>
              <div className="setting-card">
                {folderRow}
                <SettingRow label={t.views.settings.downloads.folder} hint={t.views.settings.downloads.folderHint}>
                  <button
                    type="button"
                    className="btn"
                    onClick={() => api.openDownloadDir().catch((e) => actions.toast(String(e), "error"))}
                  >
                    <FolderOpen size={16} strokeWidth={2} aria-hidden="true" /> {t.views.settings.downloads.openFolder}
                  </button>
                </SettingRow>
              </div>
              <div className="setting-card">
                <SettingRow
                  label={t.views.settings.downloads.quality}
                  id="set-quality"
                  hint={t.views.settings.downloads.qualityHint}
                >
                  <Select
                    aria-labelledby="set-quality"
                    value={props.quality}
                    onChange={props.onQuality}
                    options={QUALITIES.map((q) => ({ value: q.value, label: q.label }))}
                  />
                </SettingRow>
                <SettingRow
                  label={t.views.settings.downloads.cookies}
                  id="set-cookies"
                  hint={t.views.settings.downloads.cookiesHint}
                >
                  <Select
                    aria-labelledby="set-cookies"
                    value={settings.cookies_browser ?? ""}
                    onChange={(v) => update({ cookies_browser: v || null })}
                    options={[
                      { value: "", label: t.views.settings.downloads.off },
                      ...COOKIE_BROWSERS.map((b) => ({ value: b.value, label: t.views.settings.downloads.useBrowser(b.label) })),
                    ]}
                  />
                </SettingRow>
              </div>
              <div className="setting-card">
                <SettingToggle
                  label={t.views.settings.downloads.torrentApart}
                  hint={t.views.settings.downloads.torrentApartHint}
                  checked={settings.torrent_files_apart}
                  onChange={(on) => update({ torrent_files_apart: on })}
                />
              </div>
            </>
          )}

          {section === "network" && (
            <>
              <h2>{t.views.settings.sections.network}</h2>
              <div className="setting-card">
                <SettingRow
                  label={t.views.settings.network.dns}
                  id="set-dns"
                  hint={t.views.settings.network.dnsHint}
                >
                  <Select
                    aria-labelledby="set-dns"
                    value={settings.secure_dns ? "cloudflare" : "off"}
                    onChange={(v) => update({ secure_dns: v === "cloudflare" })}
                    options={[
                      { value: "cloudflare", label: "Cloudflare (1.1.1.1)" },
                      { value: "off", label: t.views.settings.network.dnsOff },
                    ]}
                  />
                </SettingRow>
              </div>
              <div className="setting-card">
                <SettingRow
                  label={t.views.settings.network.connections}
                  id="set-connections"
                  hint={t.views.settings.network.connectionsHint}
                >
                  <input
                    type="number"
                    aria-labelledby="set-connections"
                    min={1}
                    max={32}
                    value={settings.connections}
                    onChange={(e) => update({ connections: clamp(e.target.value, 1, 32) })}
                  />
                </SettingRow>
                <SettingRow
                  label={t.views.settings.network.concurrent}
                  id="set-concurrent"
                  hint={t.views.settings.network.concurrentHint}
                >
                  <input
                    type="number"
                    aria-labelledby="set-concurrent"
                    min={1}
                    max={10}
                    value={settings.max_concurrent}
                    onChange={(e) => update({ max_concurrent: clamp(e.target.value, 1, 10) })}
                  />
                </SettingRow>
              </div>
            </>
          )}

          {section === "performance" && (
            <>
              <h2>{t.views.settings.sections.performance}</h2>
              <div className="setting-card">
                <SettingRow
                  label={t.views.settings.performance.maxSpeed}
                  id="set-speed"
                  hint={limited ? t.views.settings.performance.limitedHint : t.views.settings.performance.unlimitedHint}
                >
                  <Select
                    aria-labelledby="set-speed"
                    value={limited ? "manual" : "unlimited"}
                    onChange={(v) => update({ speed_limit: v === "manual" ? lastLimit.current : 0 })}
                    options={[
                      { value: "unlimited", label: t.views.settings.performance.unlimited },
                      { value: "manual", label: t.views.settings.performance.manual },
                    ]}
                  />
                </SettingRow>
                {limited && (
                  <div className="setting-row setting-slider">
                    <input
                      type="range"
                      min={0}
                      max={SPEED_STOPS.length - 1}
                      step={1}
                      value={speedStop}
                      aria-label={t.views.settings.performance.maxSpeed}
                      aria-valuetext={formatSpeed(settings.speed_limit)}
                      style={{ "--fill": `${(speedStop / (SPEED_STOPS.length - 1)) * 100}%` } as React.CSSProperties}
                      onChange={(e) => update({ speed_limit: SPEED_STOPS[Number(e.target.value)] })}
                    />
                    <output>{formatSpeed(settings.speed_limit)}</output>
                  </div>
                )}
              </div>
            </>
          )}

          {section === "scheduler" && (
            <>
              <h2>{t.views.settings.sections.scheduler}</h2>
              <div className="setting-card setting-note">
                <p className="note">
                  {t.views.settings.scheduler.before}
                  <kbd>Ctrl</kbd> <kbd>Shift</kbd> <kbd>P</kbd>
                  {t.views.settings.scheduler.and}
                  <kbd>Ctrl</kbd> <kbd>Shift</kbd> <kbd>R</kbd>
                  {t.views.settings.scheduler.after}
                </p>
              </div>
            </>
          )}

          {section === "appearance" && (
            <>
              <h2>{t.views.settings.sections.appearance}</h2>
              <div className="setting-card">
                <SettingRow label={t.views.settings.appearance.theme}>
                  <Segmented
                    label={t.views.settings.appearance.theme}
                    value={props.theme}
                    onChange={props.onTheme}
                    options={[
                      { value: "system", label: t.views.settings.appearance.system },
                      { value: "light", label: t.views.settings.appearance.light },
                      { value: "dark", label: t.views.settings.appearance.dark },
                    ]}
                  />
                </SettingRow>
                <SettingRow label={t.views.settings.appearance.density}>
                  <Segmented
                    label={t.views.settings.appearance.density}
                    value={props.density}
                    onChange={props.onDensity}
                    options={[
                      { value: "comfortable", label: t.views.settings.appearance.comfortable },
                      { value: "compact", label: t.views.settings.appearance.compact },
                    ]}
                  />
                </SettingRow>
              </div>
              <div className="setting-card">
                <SettingRow
                  label={t.views.settings.appearance.progress}
                  hint={t.views.settings.appearance.progressHint}
                >
                  <Segmented
                    label={t.views.settings.appearance.progress}
                    value={props.progressStyle}
                    onChange={props.onProgressStyle}
                    options={[
                      { value: "single", label: t.views.settings.appearance.single },
                      { value: "threads", label: t.views.settings.appearance.threads },
                    ]}
                  />
                </SettingRow>
              </div>
            </>
          )}

          {section === "advanced" && (
            <>
              <h2>{t.views.settings.sections.advanced}</h2>
              <dl className="facts setting-card">
                <div className="fact">
                  <dt>{t.views.settings.advanced.siteDownloads}</dt>
                  <dd>
                    {tools == null
                      ? t.views.settings.advanced.checking
                      : tools.yt_dlp && tools.ffmpeg
                        ? t.views.settings.advanced.sitesReady
                        : t.views.settings.advanced.missing(
                            listText([!tools.yt_dlp && "yt-dlp", !tools.ffmpeg && "ffmpeg"].filter(Boolean) as string[]),
                          )}
                  </dd>
                </div>
                <div className="fact">
                  <dt>{t.views.settings.advanced.profileDownloads}</dt>
                  <dd>
                    {tools == null
                      ? t.views.settings.advanced.checking
                      : tools.gallery_dl
                        ? t.views.settings.advanced.galleryReady
                        : t.views.settings.advanced.galleryMissing}
                  </dd>
                </div>
              </dl>
              <h3>{t.views.settings.advanced.shortcuts}</h3>
              <dl className="shortcuts setting-card">
                {SHORTCUTS.map(([keys, what]) => (
                  <div key={keys}>
                    <dt>{t.views.settings.shortcuts[what]}</dt>
                    <dd>
                      {keys.split(" ").map((k, i) =>
                        k === "/" ? <span key={i}>/</span> : <kbd key={i}>{k}</kbd>,
                      )}
                    </dd>
                  </div>
                ))}
              </dl>
            </>
          )}
        </div>
          </div>
        </div>
      </div>
    </div>
  );
}

/* ---------- about ---------- */

export function AboutDialog({ onClose }: { onClose: () => void }) {
  return (
    <Modal
      title={t.views.about.title}
      onClose={onClose}
      footer={
        <>
          <span className="spacer" />
          <button type="button" className="btn btn-primary" onClick={onClose}>{t.views.about.close}</button>
        </>
      }
    >
      <dl className="facts">
        <div className="fact">
          <dt>{t.views.about.version}</dt>
          <dd>{__APP_VERSION__}</dd>
        </div>
        <div className="fact">
          <dt>{t.views.about.files}</dt>
          <dd>{t.views.about.filesText}</dd>
        </div>
        <div className="fact">
          <dt>{t.views.about.sites}</dt>
          <dd>{t.views.about.sitesText}</dd>
        </div>
        <div className="fact">
          <dt>{t.views.about.torrents}</dt>
          <dd>{t.views.about.torrentsText}</dd>
        </div>
        <div className="fact">
          <dt>{t.views.about.notBuilt}</dt>
          <dd>{t.views.about.notBuiltText}</dd>
        </div>
      </dl>
    </Modal>
  );
}
