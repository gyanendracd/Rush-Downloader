// Development-only stand-in for the Tauri backend, so the interface can be
// opened in an ordinary browser (`npm run dev`) with sample data. It is loaded
// from main.tsx only when the app is not running inside Tauri in dev mode and
// is never part of a production build.

import type { Download, Found, NewDownload, ProfileFace, Settings, ToolStatus, TorrentPreview } from "./api";

const MB = 1024 * 1024;
const now = Math.floor(Date.now() / 1000);

/** A stand-in for the picture a site shows for a video. */
const SAMPLE_THUMB =
  "data:image/svg+xml," +
  encodeURIComponent(
    '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 320 180"><defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1">' +
      '<stop offset="0" stop-color="#0f766e"/><stop offset="1" stop-color="#1d4ed8"/></linearGradient></defs>' +
      '<rect width="320" height="180" fill="url(#g)"/><circle cx="160" cy="90" r="34" fill="#fff" opacity=".9"/>' +
      '<path d="M150 72l30 18-30 18z" fill="#1d4ed8"/></svg>',
  );

function make(partial: Partial<Download> & Pick<Download, "id" | "url" | "filename" | "status">): Download {
  const dir = "D:\\Downloads";
  return {
    dir,
    path: `${dir}\\${partial.filename}`,
    error: null,
    total: null,
    downloaded: 0,
    speed: 0,
    resumable: true,
    media: false,
    torrent: false,
    connections: 16,
    active_connections: 0,
    connect_failures: 0,
    profile: false,
    items: 0,
    current: null,
    thumbnail: null,
    deleted_at: null,
    created_at: now,
    segments: [],
    ...partial,
  };
}

function segments(total: number, fill: number[]): [number, number, number][] {
  const size = Math.floor(total / fill.length);
  return fill.map((f, i) => [i * size, i * size + Math.floor(size * f), (i + 1) * size]);
}

const isoSegments = segments(1800 * MB, [1, 0.82, 0.64, 0.71, 0.33, 0.48, 0.12, 0.05]);
const liveSegments = segments(640 * MB, [0.4, 0.3, 0.35, 0.2, 0.25, 0.1, 0.15, 0.05]);

/** The files of the sample torrent: path inside its folder, and size. */
const TORRENT_FILES: [string, number][] = [
  ["Big Buck Bunny (2008)/Big Buck Bunny.mkv", 820 * MB],
  ["Big Buck Bunny (2008)/Subtitles/English.srt", 0.1 * MB],
  ["Sintel (2010)/Sintel.mkv", 1240 * MB],
  ["Sintel (2010)/Poster.jpg", 2.4 * MB],
  ["Tears of Steel (2012)/Tears of Steel.mkv", 1630 * MB],
  ["README.txt", 0.01 * MB],
];

/** How far each file of the sample torrent is: up to 12 files at once, each
 *  on its own (as with "side by side" in Settings), sharing what came in. */
function torrentItems(downloaded: number, running: boolean) {
  const got = TORRENT_FILES.map(() => 0);
  let left = downloaded;
  while (left > 1) {
    const active = TORRENT_FILES.map((_, i) => i).filter((i) => got[i] < TORRENT_FILES[i][1]).slice(0, 12);
    if (active.length === 0) break;
    // each takes an equal share, no more than it still needs
    const share = Math.min(left / active.length, ...active.map((i) => TORRENT_FILES[i][1] - got[i]));
    for (const i of active) got[i] += share;
    left -= share * active.length;
  }
  return TORRENT_FILES.map(([path, size], i) => {
    const done = got[i] >= size - 1;
    const name = path.split("/").pop()!;
    return {
      title: path,
      state: done ? ("done" as const) : running && got[i] > 0 ? ("downloading" as const) : ("waiting" as const),
      size,
      file: done ? name : null,
      path: null,
      thumbnail: null,
      duration: null,
      channel: null,
      url: null,
      downloaded: done ? null : Math.round(got[i]),
      // each file's own engine: a big file has few peers, as on a real swarm
      net:
        running && !done
          ? { peers: 2 + i * 3, seeders: 1 + i, found: 12 + i * 9, down: (0.4 + i * 0.6) * MB, up: (0.05 + i * 0.1) * MB }
          : null,
    };
  });
}

let downloads: Download[] = [
  make({ id: 1, url: "https://example.com/files/project-backup-2026.zip", filename: "project-backup-2026.zip", status: "completed", total: 1240 * MB, downloaded: 1240 * MB, created_at: now - 86400 }),
  make({ id: 2, url: "https://example.com/docs/annual-report.pdf", filename: "annual-report.pdf", status: "completed", total: 12 * MB, downloaded: 12 * MB, created_at: now - 7200 }),
  make({ id: 3, url: "https://example.com/watch/conference-keynote", filename: "Conference keynote 1080p [a1b2c3].mp4", status: "completed", total: 452 * MB, downloaded: 452 * MB, media: true, thumbnail: SAMPLE_THUMB, created_at: now - 3600 }),
  make({ id: 4, url: "https://example.com/music/field-recording.flac", filename: "field-recording.flac", status: "completed", total: 86 * MB, downloaded: 86 * MB, created_at: now - 1800 }),
  make({
    id: 5,
    url: "https://example.com/iso/workstation-installer-with-a-rather-long-descriptive-file-name-x64.iso",
    filename: "workstation-installer-with-a-rather-long-descriptive-file-name-x64.iso",
    status: "paused",
    total: 1800 * MB,
    downloaded: isoSegments.reduce((sum, s) => sum + s[1] - s[0], 0),
    segments: isoSegments,
    created_at: now - 900,
  }),
  make({ id: 6, url: "https://example.com/setup/editor-setup.exe", filename: "editor-setup.exe", status: "failed", error: "HTTP 404 Not Found", created_at: now - 600 }),
  make({ id: 7, url: "https://example.com/blog/some-article", filename: "some-article", status: "failed", media: true, error: "This link is a web page with no video RushDownload can download", created_at: now - 300 }),
  make({
    id: 8,
    url: "https://example.com/video/nature-documentary-4k.mkv",
    filename: "nature-documentary-4k.mkv",
    status: "downloading",
    total: 640 * MB,
    downloaded: liveSegments.reduce((sum, s) => sum + s[1] - s[0], 0),
    segments: liveSegments,
    speed: 9.4 * MB,
    active_connections: 8,
    created_at: now - 60,
  }),
  make({ id: 9, url: "https://example.com/photos/trip-raw-photos.zip", filename: "trip-raw-photos.zip", status: "queued", total: 2300 * MB, created_at: now - 20 }),
  make({ id: 10, url: "https://example.com/img/poster.png", filename: "poster.png", status: "connecting", created_at: now - 10 }),
  // A torrent of several files, all filling at once (see TORRENT_FILES).
  make({
    id: 16,
    url: "magnet:?xt=urn:btih:0000000000000000000000000000000000000016&dn=Open+Movie+Collection",
    filename: "Open Movie Collection",
    status: "downloading",
    torrent: true,
    total: TORRENT_FILES.reduce((sum, [, size]) => sum + size, 0),
    downloaded: 1150 * MB,
    speed: 6.2 * MB,
    created_at: now - 40,
  }),
  // A resumed torrent while what it had is checked before it carries on.
  make({
    id: 15,
    url: "C:\\Users\\me\\Downloads\\planet-sample.osm.pbf.torrent",
    filename: "planet-sample.osm.pbf",
    status: "connecting",
    torrent: true,
    total: 89.5 * 1024 * MB,
    downloaded: 5.1 * 1024 * MB,
    checking: 38 * 1024 * MB,
    created_at: now - 5,
  }),
  make({
    id: 11,
    url: "https://example.com/playlist?list=sample",
    filename: "Sample lecture series",
    status: "downloading",
    profile: true,
    items: 3,
    downloaded: 410 * MB,
    speed: 6 * MB,
    current: { name: "Lecture 4: a rather long title that will not fit on one line of the dialog", downloaded: 20 * MB, total: 180 * MB, index: 4, count: 12 },
    created_at: now - 30,
  }),
  // Files deleted from disk since: one lately (still on the Downloads page,
  // struck through), one long ago (only in History).
  make({
    id: 13,
    url: "https://example.com/files/holiday-video.mp4",
    filename: "holiday-video.mp4",
    status: "completed",
    total: 310 * MB,
    downloaded: 310 * MB,
    deleted_at: now - 2 * 86400,
    created_at: now - 6 * 86400,
  }),
  make({
    id: 14,
    url: "https://example.com/files/old-installer.exe",
    filename: "old-installer.exe",
    status: "completed",
    total: 48 * MB,
    downloaded: 48 * MB,
    deleted_at: now - 15 * 86400,
    created_at: now - 40 * 86400,
  }),
  // A whole profile, saved: its row and its dialog show whose it is.
  make({
    id: 12,
    url: "https://example.com/@sample.studio",
    filename: "sample.studio (Example)",
    status: "completed",
    profile: true,
    items: 6,
    total: 76 * MB,
    downloaded: 76 * MB,
    created_at: now - 4000,
  }),
];

let settings: Settings = {
  download_dir: "D:\\Downloads",
  max_concurrent: 3,
  connections: 16,
  speed_limit: 0,
  cookies_browser: null,
  secure_dns: true,
  torrent_files_apart: true,
};

let nextId = 16;
const listeners = new Map<symbol, (list: Download[]) => void>();
const emit = () => listeners.forEach((cb) => cb(downloads.map((d) => ({ ...d }))));
const patch = (id: number, change: Partial<Download>) => {
  downloads = downloads.map((d) => (d.id === id ? { ...d, ...change } : d));
  emit();
};

// Advance whatever is "downloading" twice a second.
setInterval(() => {
  let changed = false;
  downloads = downloads.map((d) => {
    if (d.status === "downloading" && d.current?.total) {
      // A playlist: the current file fills, then the next one starts.
      changed = true;
      const c = d.current;
      const got = Math.min(c.total!, c.downloaded + d.speed / 2);
      const next = got >= c.total! && (c.index ?? 0) < (c.count ?? 0);
      return {
        ...d,
        downloaded: d.downloaded + d.speed / 2,
        items: next ? d.items + 1 : d.items,
        current: next ? { ...c, name: `Lecture ${c.index! + 1}`, downloaded: 0, index: c.index! + 1 } : { ...c, downloaded: got },
      };
    }
    if (d.status !== "downloading" || !d.total) return d;
    changed = true;
    const speed = Math.max(2 * MB, d.speed + (Math.random() - 0.5) * 2 * MB);
    const step = speed / 2;
    const open = d.segments.filter((s) => s[1] < s[2]);
    const segs = d.segments.map((s): [number, number, number] =>
      s[1] < s[2] ? [s[0], Math.min(s[2], s[1] + Math.floor(step / Math.max(1, open.length))), s[2]] : s,
    );
    const done = segs.length ? segs.reduce((sum, s) => sum + s[1] - s[0], 0) : Math.min(d.total, d.downloaded + step);
    if (done >= d.total) {
      return { ...d, status: "completed" as const, downloaded: d.total, speed: 0, segments: [], active_connections: 0 };
    }
    return { ...d, downloaded: done, speed, segments: segs };
  });
  if (changed) emit();
}, 500);

let signedIn: string[] = [];
let openSite: string | null = null;
const mediaListeners = new Set<(found: { url: string; kind: "video" | "audio" | "stream"; referer: string; label: string; heights: number[]; master: boolean }) => void>();
const navListeners = new Set<(url: string) => void>();
const titleListeners = new Set<(title: string) => void>();

/** A profile about which nothing could be found. */
const NOBODY = {
  name: null,
  picture: null,
  about: null,
  banner: null,
  posts: null,
  media: null,
  followers: null,
  following: null,
  joined: null,
} satisfies ProfileFace as ProfileFace;

let appPrefs = { start_with_system: false, close_to_tray: false, notifications: true, hardware_acceleration: true };

export const mockApi = {
  list: async () => downloads,
  getAppPrefs: async () => ({ ...appPrefs }),
  setAppPrefs: async (prefs: typeof appPrefs) => {
    appPrefs = { ...prefs };
  },
  setTrayText: async () => {},
  // A plain browser has no system notifications to send here.
  notify: async (title: string, body: string) => console.info("[notification]", title, body),
  items: async (id: number) => {
    await new Promise((r) => setTimeout(r, 600));
    const d = downloads.find((x) => x.id === id);
    if (d?.torrent && d.id === 16) return torrentItems(d.downloaded, d.status === "downloading");
    const at = d?.current?.index;
    // A saved profile: what is in its folder. A plain browser cannot show a
    // file from this computer, so these stand in with a sign of their kind.
    if (d?.profile && !at && d.status === "completed") {
      return Array.from({ length: d.items }, (_, i) => {
        const name = `17220073210254257${10 + i}_1.${i % 3 === 2 ? "mp4" : "jpg"}`;
        return {
          title: name,
          // one of them has been deleted from the folder since
          state: i === 3 ? ("deleted" as const) : ("done" as const),
          size: (150 + i * 73) * 1024,
          file: name,
          path: null,
          thumbnail: null,
          duration: null,
          channel: null,
          url: null,
        };
      });
    }
    if (!d?.profile || !at) return [];
    return Array.from({ length: d.current?.count ?? 0 }, (_, i) => ({
      title: i + 1 === at ? d.current!.name : `Lecture ${i + 1}: ${["an introduction", "the long middle part with a title that runs on and on", "wrapping up"][i % 3]}`,
      state: i + 1 < at ? (i === 1 ? ("missed" as const) : ("done" as const)) : i + 1 === at ? ("downloading" as const) : ("waiting" as const),
      size: i + 1 < at && i !== 1 ? (120 + i * 17) * MB : i + 1 === at ? d.current!.downloaded : null,
      file: i + 1 < at && i !== 1 ? `00${i + 1} - Lecture ${i + 1} [abc${i}].mp4` : null,
      // a plain browser cannot show a file from this computer
      path: null,
      // every third one has no picture, to show the stand-in
      thumbnail:
        i % 3 === 2
          ? null
          : "data:image/svg+xml," +
            encodeURIComponent(
              `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 320 180"><rect width="320" height="180" fill="hsl(${(i * 47) % 360} 60% 45%)"/><path d="M140 62l52 28-52 28z" fill="#fff"/></svg>`,
            ),
      duration: 600 + i * 173,
      channel: "Sample channel",
      url: `https://example.com/watch?v=abc${i}`,
    }));
  },
  add: async (request: NewDownload) => {
    const isTorrent = /^magnet:\?/i.test(request.url) || /\.torrent"?$/i.test(request.url);
    if (!isTorrent && !/^https?:\/\//i.test(request.url)) throw "That is not a valid link";
    const id = nextId++;
    const name = isTorrent
      ? "Sample Collection"
      : request.filename || decodeURIComponent(request.url.split("/").pop() || "download");
    downloads = [...downloads, make({ id, url: request.url, filename: name, status: "queued", torrent: isTorrent })];
    emit();
    return id;
  },
  scan: async (url: string): Promise<Found[]> => {
    await new Promise((r) => setTimeout(r, 900));
    return [
      { url: `${url}/clip.mp4`, kind: "video", name: "clip.mp4", size: 48 * MB, referer: url },
      { url: `${url}/second part.mkv`, kind: "video", name: "second part.mkv", size: 310 * MB, referer: url },
      { url: `${url}/master.m3u8`, kind: "stream", name: "master.m3u8", size: null, referer: url },
      { url: `${url}/theme.mp3`, kind: "audio", name: "theme.mp3", size: 5.5 * MB, referer: url },
      { url: "https://player.example.com/embed/42", kind: "embed", name: "Embedded player (player.example.com)", size: null, referer: url },
    ];
  },
  signinOpen: async (url: string, _area: unknown, browse = false) => {
    openSite = new URL(url).hostname.replace(/^www\./, "");
    if (!browse) return;
    // The sample preview pretends the page played a video and loaded a stream.
    window.setTimeout(() => {
      navListeners.forEach((cb) => cb(url));
      titleListeners.forEach((cb) => cb("Sample video page"));
    }, 300);
    window.setTimeout(() => {
      mediaListeners.forEach((cb) => cb({ url: `https://cdn.${openSite}/media/holiday-clip-1080p.mp4`, kind: "video", referer: url, label: "1080p", heights: [], master: false }));
    }, 1200);
    window.setTimeout(() => {
      mediaListeners.forEach((cb) => cb({ url: `https://cdn.${openSite}/hls/master.m3u8`, kind: "stream", referer: url, label: "", heights: [1080, 720, 480], master: true }));
    }, 2000);
  },
  browserGo: async (_action: string) => {},
  browserAutoplay: async () => {},
  browserKeepSignIn: async () => {},
  // The sample preview can fake a copy with: window.dispatchEvent(new CustomEvent("mock-copy", { detail: "https://…" }))
  onClipboardLink: (cb: (link: string) => void) => {
    const hear = (e: Event) => cb(String((e as CustomEvent).detail));
    window.addEventListener("mock-copy", hear);
    return Promise.resolve(() => window.removeEventListener("mock-copy", hear));
  },
  setClipboardWatch: async (_on: boolean) => {},
  // Likewise for the browser extension: new CustomEvent("mock-extension-link", { detail: "https://…" })
  onExtensionLink: (cb: (link: string) => void) => {
    const hear = (e: Event) => cb(String((e as CustomEvent).detail));
    window.addEventListener("mock-extension-link", hear);
    return Promise.resolve(() => window.removeEventListener("mock-extension-link", hear));
  },
  onExtensionAdded: (_cb: (count: number) => void) => Promise.resolve(() => {}),
  linkSize: async (url: string, _referer: string) => {
    await new Promise((r) => setTimeout(r, 500));
    return /1080/.test(url) ? 412 * MB : 96 * MB;
  },
  onBrowserMedia: (cb: (found: { url: string; kind: "video" | "audio" | "stream"; referer: string; label: string; heights: number[]; master: boolean }) => void) => {
    mediaListeners.add(cb);
    return Promise.resolve(() => void mediaListeners.delete(cb));
  },
  onBrowserNav: (cb: (url: string) => void) => {
    navListeners.add(cb);
    return Promise.resolve(() => void navListeners.delete(cb));
  },
  onBrowserTitle: (cb: (title: string) => void) => {
    titleListeners.add(cb);
    return Promise.resolve(() => void titleListeners.delete(cb));
  },
  onBrowserPopup: (_cb: (url: string) => void) => Promise.resolve(() => {}),
  onBrowserPage: (_cb: (picture: string) => void) => Promise.resolve(() => {}),
  signinBounds: async (_area: unknown) => {},
  onSignInPopup: (cb: (host: string) => void) => {
    // In the sample preview, pretend the site tried to open Google.
    const timer = window.setTimeout(() => openSite && cb("accounts.google.com"), 2500);
    return Promise.resolve(() => window.clearTimeout(timer));
  },
  signinClose: async () => {
    if (openSite) signedIn = [...new Set([...signedIn, openSite])].sort();
    openSite = null;
    return signedIn;
  },
  signInStatus: async () => signedIn,
  pickCookiesFile: async () => "cookies.txt" as string | null,
  importSignIn: async (_path: string) => {
    signedIn = [...new Set([...signedIn, "google.com", "youtube.com"])].sort();
    return signedIn;
  },
  importBrowserSignIn: async (browser: string) => {
    await new Promise((r) => setTimeout(r, 1200));
    // The browsers built on Chrome keep their sign-ins locked on Windows.
    if (browser !== "firefox") {
      throw `${browser[0].toUpperCase()}${browser.slice(1)} locks its sign-ins so that other apps cannot read them. Sign in on this page instead, or import a cookies file.`;
    }
    signedIn = [...new Set([...signedIn, "instagram.com", "x.com", "youtube.com"])].sort();
    return signedIn;
  },
  forgetSignIns: async () => {
    signedIn = [];
  },
  playlistInfo: async (url: string) => {
    // "slow" in the link stands for a site that takes long to list a profile's posts.
    await new Promise((r) => setTimeout(r, /slow/.test(url) ? 12000 : 900));
    if (!/playlist\?list=/.test(url)) {
      // "locked" in the link stands for a site that lists a profile's posts
      // only for signed-in visitors, and tells the downloader it is not there.
      if (/locked/.test(url)) throw "This profile was not found";
      // A profile's posts, counted. Whose it is comes from `profileFace`.
      return {
        title: /([\w.]+)\/?$/.exec(url)?.[1] ?? "sample",
        count: 38,
        photos: 214,
        more: false,
        duration: null,
        thumbnail: null,
        channel: null,
        samples: [],
      };
    }
    return {
      title: "Weekend projects, start to finish",
      count: 27,
      photos: 0,
      more: false,
      duration: 27 * 754,
      samples: ["https://www.youtube.com/watch?v=sample1"],
      thumbnail: null,
      channel: "Sample channel",
    };
  },
  profileFace: async (url: string) => {
    await new Promise((r) => setTimeout(r, /slow/.test(url) ? 1500 : 300));
    // "plain" in the link stands for a site that says nothing about whose it is.
    if (/plain/.test(url) || /playlist\?list=/.test(url)) return NOBODY;
    return {
      ...NOBODY,
      name: "Sample Studio",
      picture:
        "data:image/svg+xml," +
        encodeURIComponent(
          '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64"><rect width="64" height="64" fill="#3b6fd4"/>' +
            '<circle cx="32" cy="25" r="11" fill="#fff"/><path d="M10 64c2-16 12-22 22-22s20 6 22 22z" fill="#fff"/></svg>',
        ),
      about: "Short films and behind-the-scenes photos from a two-person studio. New work every Friday.",
    };
  },
  profileStats: async (url: string) => {
    await new Promise((r) => setTimeout(r, /slow/.test(url) ? 3500 : 700));
    // Only some sites tell a visitor a profile's numbers: "plain" and "bare" stand for the others.
    if (/plain|bare/.test(url) || /playlist\?list=/.test(url)) return NOBODY;
    return {
      ...NOBODY,
      name: "Sample Studio",
      about: "Short films and behind-the-scenes photos from a two-person studio. New work every Friday.",
      banner:
        "data:image/svg+xml," +
        encodeURIComponent(
          '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 600 200" preserveAspectRatio="none"><defs>' +
            '<linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#274b9f"/>' +
            '<stop offset="1" stop-color="#7a4fd0"/></linearGradient></defs><rect width="600" height="200" fill="url(#g)"/>' +
            '<circle cx="470" cy="60" r="90" fill="#fff" opacity=".12"/><circle cx="120" cy="190" r="120" fill="#fff" opacity=".08"/></svg>',
        ),
      posts: 4678,
      media: 988,
      followers: 94442,
      following: 166,
      joined: "2017-12-14",
    };
  },
  // "gallery" in a page's link stands for a site the app has no rules for,
  // whose page the downloading programs find to be a whole collection.
  collectionOf: async (url: string) => {
    await new Promise((r) => setTimeout(r, 1200));
    const match = /^https?:\/\/(?:www\.)?([^./]+)[^/]*\/(?:gallery\/)?([\w.-]+)/.exec(url.trim());
    if (!/gallery/.test(url) || !match) return null;
    return { site: match[1][0].toUpperCase() + match[1].slice(1), name: match[2], by: "gallerydl" as const };
  },
  profileOf: async (url: string) => {
    if (/playlist\?list=/.test(url)) return { site: "YouTube playlist", name: "PLsample" };
    const match = /^https?:\/\/[^/]+\/@?([\w.]+)\/?$/.exec(url.trim());
    return match && /profile|@/.test(url) ? { site: "Example", name: match[1] } : null;
  },
  mediaInfo: async (url: string) => {
    await new Promise((r) => setTimeout(r, 900));
    if (/nothing/i.test(url)) throw "no video was found on this page";
    // A private post: it shows only once a sign-in has been made.
    if (/private|members/i.test(url) && signedIn.length === 0) {
      throw "ERROR: [Instagram] sample: You need to log in to access this content. Use --cookies-from-browser or --cookies for the authentication";
    }
    const picture =
      "data:image/svg+xml," +
      encodeURIComponent(
        '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 320 180"><defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#2F7DFC"/><stop offset="1" stop-color="#0a36a8"/></linearGradient></defs><rect width="320" height="180" fill="url(#g)"/><circle cx="250" cy="40" r="70" fill="#fff" opacity=".12"/><path d="M140 62l52 28-52 28z" fill="#fff"/></svg>',
      );
    return {
      title: "Conference keynote: building fast, calm software for everyone",
      thumbnail: picture,
      duration: 3765,
      heights: [2160, 1440, 1080, 720, 480, 360, 240, 144],
      site: "Example",
    };
  },
  torrentPreview: async (source: string): Promise<TorrentPreview> => {
    await new Promise((r) => setTimeout(r, 900));
    if (/broken/i.test(source)) throw "Could not read this torrent";
    // A map data set: tens of thousands of small tiles, most in one folder.
    if (/lod2|huge/i.test(source)) {
      const tiles = Array.from({ length: 30000 }, (_, i) => ({
        path: ["bavaria-lod2-citygml", i < 29000 ? "tiles" : `extra-${Math.floor(i / 200)}`, `${600 + Math.floor(i / 180)}_${5300 + (i % 180)}.gml`],
        size: (2 + (i % 7)) * MB,
      }));
      const files = tiles.map((f, index) => ({ index, ...f }));
      return { name: "bavaria-lod2-citygml", total: files.reduce((sum, f) => sum + f.size, 0), files };
    }
    const files = [
      { path: ["Sample Collection", "Readme.txt"], size: 4 * 1024 },
      { path: ["Sample Collection", "Videos", "Part 1.mkv"], size: 1400 * MB },
      { path: ["Sample Collection", "Videos", "Part 2.mkv"], size: 1350 * MB },
      { path: ["Sample Collection", "Videos", "Extras", "Trailer.mp4"], size: 48 * MB },
      { path: ["Sample Collection", "Music", "Theme.flac"], size: 36 * MB },
      { path: ["Sample Collection", "Music", "Credits.flac"], size: 41 * MB },
      { path: ["Sample Collection", "Cover.jpg"], size: 2 * MB },
    ].map((f, index) => ({ index, ...f }));
    return { name: "Sample Collection", total: files.reduce((sum, f) => sum + f.size, 0), files };
  },
  onFileDrop: (_cb: (paths: string[]) => void) => Promise.resolve(() => {}),
  pause: async (id: number) => patch(id, { status: "paused", speed: 0, active_connections: 0 }),
  resume: async (id: number) => {
    const d = downloads.find((x) => x.id === id);
    if (d?.total) patch(id, { status: "downloading", speed: 6 * MB, active_connections: 8, error: null });
    else patch(id, { status: "failed", error: d?.error ?? "HTTP 404 Not Found" });
  },
  remove: async (id: number, _deleteFile: boolean) => {
    downloads = downloads.filter((d) => d.id !== id);
    emit();
  },
  open: async (_id: number) => {},
  reveal: async (_id: number) => {},
  openDownloadDir: async () => {},
  tools: async (): Promise<ToolStatus> => ({ yt_dlp: true, ffmpeg: true, gallery_dl: true }),
  getSettings: async () => settings,
  setSettings: async (next: Settings) => {
    settings = next;
  },
  onChange: (cb: (list: Download[]) => void) => {
    const key = Symbol();
    listeners.set(key, cb);
    return Promise.resolve(() => {
      listeners.delete(key);
    });
  },
  pickFolder: async (defaultPath?: string) => (defaultPath ?? "C:/Users/you/Downloads") as string | null,
  pickLinkFile: async () => "links.txt" as string | null,
  importLinks: async (_path: string) => [
    "https://example.com/batch/first-file.zip",
    "https://example.com/batch/second-file.mp4",
  ],
  windowMinimize: async () => {},
  windowToggleMaximize: async () => {},
  windowClose: async () => {},
};
