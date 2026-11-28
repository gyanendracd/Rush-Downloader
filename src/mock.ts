// Development-only stand-in for the Tauri backend, so the interface can be
// opened in an ordinary browser (`npm run dev`) with sample data. It is loaded
// from main.tsx only when the app is not running inside Tauri in dev mode and
// is never part of a production build.

import type { AfterAllNotice, BatchInfo, Download, MediaOptions, LinkCheck, Found, NewDownload, ProfileFace, Schedule, ScheduleSpec, Settings, SiteFile, SiteFiles, SiteKind, ToolStatus, TorrentPreview } from "./api";

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
    speed_limit: 0,
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
  start_scheduled_on_launch: true,
  after_all: "nothing",
  after_countdown: 60,
  after_failures: false,
  organize: {
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
  },
};

/* ---------- after all downloads ----------
 * The real countdown runs in the backend (after.rs). Here it can be tried
 * from the browser's console: __rushAfterAll("shutdown", 10). */
const afterListeners = new Map<symbol, (n: AfterAllNotice) => void>();
let afterTimer: number | null = null;
const afterEmit = (n: AfterAllNotice) => afterListeners.forEach((cb) => cb(n));
(window as unknown as Record<string, unknown>).__rushAfterAll = (action: AfterAllNotice["action"], seconds = 10) => {
  if (afterTimer) window.clearInterval(afterTimer);
  if (action === "notify") return afterEmit({ action, left: null, reason: "done" });
  let left = seconds;
  afterEmit({ action, left, reason: null });
  afterTimer = window.setInterval(() => {
    left -= 1;
    if (left > 0) return afterEmit({ action, left, reason: null });
    window.clearInterval(afterTimer!);
    afterTimer = null;
    afterEmit({ action, left: null, reason: "done" });
  }, 1000);
};

/* ---------- schedules ----------
 * The real ones are kept and run by the engine (schedule.rs). Here they
 * only say when they next start, worked out the same way, so the Scheduler
 * page has something to show. */

/** Minutes after midnight, `offset` minutes from now on the clock. */
function clockIn(offset: number): number {
  const d = new Date();
  return (((d.getHours() * 60 + d.getMinutes() + offset) % 1440) + 1440) % 1440;
}

function dayText(d: Date): string {
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

/** The local start on day `d` (a date at midnight) at `minutes`. */
function at(d: Date, minutes: number): Date {
  return new Date(d.getFullYear(), d.getMonth(), d.getDate(), Math.floor(minutes / 60), minutes % 60);
}

function nextStart(spec: ScheduleSpec, from: Date): Date | null {
  if (spec.repeat.kind === "once") {
    const [y, m, day] = spec.repeat.date.split("-").map(Number);
    const t = at(new Date(y, m - 1, day), spec.start);
    return t > from ? t : null;
  }
  for (let ahead = 0; ahead < 8; ahead++) {
    const d = new Date(from.getFullYear(), from.getMonth(), from.getDate() + ahead);
    const t = at(d, spec.start);
    // bit 0 is Monday; getDay() counts from Sunday
    if (t > from && spec.repeat.days & (1 << ((d.getDay() + 6) % 7))) return t;
  }
  return null;
}

function stopAfter(spec: ScheduleSpec, start: Date): Date | null {
  if (spec.stop == null) return null;
  const same = at(start, spec.stop);
  return spec.stop > spec.start ? same : new Date(same.getTime() + 86400_000);
}

const secs = (d: Date | null) => (d ? Math.floor(d.getTime() / 1000) : null);

function describe(id: number, spec: ScheduleSpec, extra: Partial<Schedule> = {}): Schedule {
  const next = nextStart(spec, new Date());
  return {
    ...spec,
    id,
    enabled: true,
    created_at: now - 86400,
    status: "waiting",
    next_start: secs(next),
    next_stop: next ? secs(stopAfter(spec, next)) : null,
    last: null,
    ...extra,
  };
}

let schedules: Schedule[] = [
  describe(1, {
    name: "Night downloads",
    repeat: { kind: "weekly", days: 0b0011111 },
    start: 23 * 60,
    stop: 7 * 60,
    downloads: [9, 5],
  }),
  describe(
    2,
    { name: "Big files while I'm at lunch", repeat: { kind: "weekly", days: 0b1111111 }, start: clockIn(-20), stop: clockIn(40), downloads: [8] },
    { status: "active", next_stop: now + 40 * 60, last: { kind: "started", at: now - 1200 } },
  ),
  describe(3, {
    name: "Movie collection",
    repeat: { kind: "once", date: dayText(new Date(Date.now() + 86400_000)) },
    start: 6 * 60 + 30,
    stop: null,
    downloads: [16],
  }),
  describe(
    4,
    { name: "Weekend backup", repeat: { kind: "weekly", days: 0b1100000 }, start: 10 * 60, stop: 18 * 60, downloads: [12] },
    { enabled: false, status: "paused" },
  ),
  describe(
    5,
    { name: "Report download", repeat: { kind: "once", date: dayText(new Date(Date.now() - 86400_000)) }, start: 8 * 60, stop: null, downloads: [2] },
    { status: "done", next_start: null, next_stop: null, last: { kind: "started", at: now - 80000 } },
  ),
];
let nextScheduleId = 6;
const scheduleListeners = new Map<symbol, (list: Schedule[]) => void>();
const emitSchedules = () => scheduleListeners.forEach((cb) => cb(schedules.map((x) => ({ ...x }))));

/* A sample batch: one row, its downloads listed in its details. The
 * engine works its status out from theirs (batch.rs); `rollUp` does the
 * same here. */
const BATCH_DIR = "D:\\Downloads\\YouTube Collection November";
const sampleBatchItems: [string, Download["status"], number, string | null][] = [
  ["Opening keynote [wFNzXwueV1w].mp4", "completed", 412, null],
  ["Design systems at scale [g9K-sdtqeR4].mp4", "completed", 268, null],
  ["Building for the desktop [tLFUd_aQrGM].mp4", "downloading", 520, null],
  ["Panel discussion [abc123].mp4", "queued", 0, null],
  ["Closing remarks [def456].mp4", "queued", 0, null],
  ["Private rehearsal [ghi789]", "failed", 0, "Private video. Sign in if you've been granted access to this video"],
];
downloads.push(
  make({ id: 30, url: "", filename: "YouTube Collection November", status: "downloading", dir: "D:\\Downloads", path: BATCH_DIR, created_at: now - 240 }),
  ...sampleBatchItems.map(([name, status, size, error], i) =>
    make({
      id: 31 + i,
      url: `https://youtu.be/sample${i}?si=batch`,
      filename: name,
      status,
      error,
      media: true,
      parent: 30,
      dir: BATCH_DIR,
      path: `${BATCH_DIR}\\${name}`,
      total: size ? size * MB : null,
      downloaded: status === "completed" ? size * MB : status === "downloading" ? size * MB * 0.35 : 0,
      speed: status === "downloading" ? 7 * MB : 0,
      finished_at: status === "completed" || status === "failed" ? now - 60 : null,
    }),
  ),
);

/** Works each batch's status, sizes and speed out from its downloads. */
function rollUp(list: Download[]): Download[] {
  return list.map((d) => {
    if (!d.batch && !list.some((c) => c.parent === d.id)) return d;
    const items = list.filter((c) => c.parent === d.id);
    const b: BatchInfo = { count: items.length, completed: 0, downloading: 0, queued: 0, paused: 0, failed: 0, cancelled: 0, progress: 0, current: null };
    let downloaded = 0;
    let total: number | null = 0;
    let parts = 0;
    let speed = 0;
    for (const c of items) {
      if (c.cancelled) b.cancelled++;
      else if (c.status === "completed") b.completed++;
      else if (c.status === "downloading" || c.status === "connecting") {
        b.downloading++;
        b.current ??= c.filename;
      } else if (c.status === "queued") b.queued++;
      else if (c.status === "paused") b.paused++;
      else b.failed++;
      if (c.cancelled || c.status === "failed") continue;
      downloaded += c.status === "completed" ? (c.total ?? c.downloaded) : c.downloaded;
      total = c.total != null && total != null ? total + c.total : null;
      parts += c.status === "completed" ? 1 : c.total ? Math.min(1, c.downloaded / c.total) : 0;
      speed += c.speed;
    }
    const counted = b.count - b.failed - b.cancelled;
    b.progress = counted ? parts / counted : 0;
    const bad = b.failed + b.cancelled;
    const status: Download["status"] = b.downloading
      ? "downloading"
      : b.queued
        ? "queued"
        : b.paused
          ? "paused"
          : bad === 0 || b.completed > 0
            ? "completed"
            : "failed";
    const error = status === "completed" && bad ? `${bad} of ${b.count} links did not download` : status === "failed" ? "None of the links could be downloaded" : null;
    return { ...d, batch: b, status, error, downloaded, total, speed, items: b.completed };
  });
}

/** Starts waiting downloads of each batch, two at a time. */
function startQueued() {
  for (const parent of downloads.filter((d) => d.batch)) {
    const items = downloads.filter((c) => c.parent === parent.id);
    let running = items.filter((c) => c.status === "downloading").length;
    for (const c of items) {
      if (running >= 2) break;
      if (c.status !== "queued") continue;
      running++;
      const total = c.total ?? (120 + ((c.id * 37) % 300)) * MB;
      downloads = downloads.map((x) => (x.id === c.id ? { ...x, status: "downloading", total, speed: 6 * MB } : x));
    }
  }
}

downloads = rollUp(downloads);

let nextId = 40;
const listeners = new Map<symbol, (list: Download[]) => void>();
const emit = () => {
  startQueued();
  downloads = rollUp(downloads);
  listeners.forEach((cb) => cb(downloads.map((d) => ({ ...d }))));
};
const patch = (id: number, change: Partial<Download>) => {
  downloads = downloads.map((d) => (d.id === id ? { ...d, ...change } : d));
  emit();
};

// Advance whatever is "downloading" twice a second.
setInterval(() => {
  let changed = false;
  downloads = downloads.map((d) => {
    // a batch moves with its downloads (rollUp)
    if (d.batch) return d;
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
  list: async () => rollUp(downloads),
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
  onBrowserDownload: (_cb: (url: string) => void) => Promise.resolve(() => {}),
  onBrowserDownloadFailed: (_cb: (why: string) => void) => Promise.resolve(() => {}),
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
  // A GitHub release stands in for every site whose files are listed;
  // a store link for the sites the app cannot download from.
  siteKind: async (url: string): Promise<SiteKind | null> => {
    if (/store\.steampowered\.com/i.test(url)) return { site: "Steam", support: "blocked", why: "store" };
    if (/github\.com\/[^/]+\/[^/]+/i.test(url)) return { site: "GitHub", support: "files", why: null };
    return null;
  },
  siteFiles: async (_url: string): Promise<SiteFiles> => {
    await new Promise((r) => setTimeout(r, 700));
    const file = (name: string, size: number | null): SiteFile => ({
      path: [name],
      url: `https://github.com/example/tool/releases/download/v2.1.0/${name}`,
      size,
      headers: [],
      late: null,
      meta: null,
    });
    return {
      site: "GitHub",
      title: "tool v2.1.0",
      partial: false,
      files: [
        file("tool-2.1.0-x86_64-pc-windows-msvc.zip", 6 * MB),
        file("tool-2.1.0-aarch64-pc-windows-msvc.zip", 5.6 * MB),
        file("tool-2.1.0-x86_64-unknown-linux-musl.tar.gz", 7.1 * MB),
        file("tool-2.1.0-aarch64-apple-darwin.tar.gz", 6.4 * MB),
        file("tool_2.1.0-1_amd64.deb", 7.4 * MB),
        file("checksums.sha256", 1200),
        file("tool-2.1.0.zip", null),
        file("tool-2.1.0.tar.gz", null),
        file("tool-2.1.0-x86_64-unknown-linux-gnu.tar.gz", 7 * MB),
        file("tool-2.1.0-i686-pc-windows-msvc.zip", 5.2 * MB),
        file("tool-2.1.0-x86_64-apple-darwin.tar.gz", 6.6 * MB),
      ],
    };
  },
  setSpeedLimit: async (id: number, rate: number) => patch(id, { speed_limit: rate }),
  // The real check fetches pieces of the file (refresh.rs). Here a link to a
  // file of the same name counts as the same file.
  checkNewLink: async (id: number, url: string): Promise<LinkCheck> => {
    await new Promise((r) => setTimeout(r, 700));
    if (!/^https?:\/\/\S+$/i.test(url.trim())) throw "Enter a full link, starting with http:// or https://";
    const d = downloads.find((x) => x.id === id)!;
    const name = (u: string) => u.split(/[?#]/)[0].split("/").pop();
    const same = name(url) === name(d.url);
    return {
      resume: same || d.downloaded === 0,
      problem: same || d.downloaded === 0 ? null : "content_differs",
      downloaded: d.downloaded,
      old_total: d.total,
      new_total: d.total,
    };
  },
  useNewLink: async (id: number, url: string, _h: unknown, restart: boolean): Promise<LinkCheck> => {
    const check = await mockApi.checkNewLink(id, url);
    if (!check.resume && !restart) throw "The new link does not give the same file";
    patch(id, { url, error: null, status: "queued", ...(check.resume && !restart ? {} : { downloaded: 0, segments: [] }) });
    return check;
  },
  addSiteFiles: async (url: string, title: string, files: SiteFile[], _dir?: string, _speedLimit?: number) => {
    const id = nextId++;
    downloads = [
      ...downloads,
      make({
        id,
        url,
        filename: files.length === 1 ? files[0].path[files[0].path.length - 1] : title,
        status: "queued",
        bundle: files.length > 1,
        total: files.reduce((sum, f) => sum + (f.size ?? 0), 0) || null,
      }),
    ];
    emit();
    return id;
  },
  onFileDrop: (_cb: (paths: string[]) => void) => Promise.resolve(() => {}),
  onFileHover: (_cb: (over: boolean) => void) => Promise.resolve(() => {}),
  moveDownload: async (id: number, dir: string) => {
    const d = downloads.find((x) => x.id === id)!;
    patch(id, { dir, path: `${dir}\\${d.filename}` });
    return `${dir}\\${d.filename}`;
  },
  addBatch: async (name: string, urls: string[], media: MediaOptions, dir?: string) => {
    if (!name.trim()) throw "Give the batch a name";
    const id = nextId++;
    const folder = `${dir ?? settings.download_dir}\\${name.trim()}`;
    downloads = [
      ...downloads,
      make({ id, url: "", filename: name.trim(), status: "queued", dir: dir ?? settings.download_dir, path: folder }),
      ...urls.map((url) =>
        make({
          id: nextId++,
          url,
          filename: decodeURIComponent(url.split(/[?#]/)[0].split("/").filter(Boolean).pop() || url),
          status: "queued",
          media: !/\.[a-z0-9]{2,4}$/i.test(url.split(/[?#]/)[0]),
          parent: id,
          dir: folder,
        }),
      ),
    ];
    void media;
    emit();
    return id;
  },
  retryFailed: async (id: number) => {
    downloads = downloads.map((d) => (d.parent === id && d.status === "failed" ? { ...d, status: "queued", error: null, cancelled: false } : d));
    emit();
  },
  cancelItem: async (id: number) =>
    patch(id, { status: "failed", cancelled: true, error: "Cancelled", downloaded: 0, speed: 0, active_connections: 0 }),
  pickTextFiles: async () => [] as string[],
  readTextFile: async (path: string) => ({ name: path, text: "", bytes: 0 }),
  saveTextFile: async (defaultName: string) => defaultName as string | null,
  pause: async (id: number) => {
    const ids = downloads.some((d) => d.parent === id) ? downloads.filter((d) => d.parent === id).map((d) => d.id) : [id];
    downloads = downloads.map((d) =>
      ids.includes(d.id) && ["queued", "connecting", "downloading"].includes(d.status) ? { ...d, status: "paused", speed: 0, active_connections: 0 } : d,
    );
    emit();
  },
  resume: async (id: number) => {
    const items = downloads.filter((d) => d.parent === id);
    if (items.length) {
      const paused = items.some((d) => d.status === "paused");
      downloads = downloads.map((d) =>
        d.parent === id && (d.status === "paused" || (!paused && d.status === "failed")) ? { ...d, status: "queued", error: null, cancelled: false } : d,
      );
      emit();
      return;
    }
    const d = downloads.find((x) => x.id === id);
    if (d?.total) patch(id, { status: "downloading", speed: 6 * MB, active_connections: 8, error: null });
    else patch(id, { status: "failed", error: d?.error ?? "HTTP 404 Not Found" });
  },
  remove: async (id: number, _deleteFile: boolean) => {
    downloads = downloads.filter((d) => d.id !== id && d.parent !== id);
    emit();
  },
  open: async (_id: number) => {},
  reveal: async (_id: number) => {},
  showFolder: async (_id: number) => {},
  openDownloadDir: async () => {},
  tools: async (): Promise<ToolStatus> => ({ yt_dlp: true, ffmpeg: true, gallery_dl: true }),
  getSettings: async () => settings,
  schedules: async () => schedules.map((x) => ({ ...x })),
  saveSchedule: async (id: number | null, spec: ScheduleSpec) => {
    if (spec.downloads.length === 0) throw "no downloads picked";
    const sid = id ?? nextScheduleId++;
    const old = schedules.find((x) => x.id === sid);
    const made = describe(sid, spec, { enabled: old?.enabled ?? true, status: old?.enabled === false ? "paused" : "waiting" });
    schedules = old ? schedules.map((x) => (x.id === sid ? made : x)) : [...schedules, made];
    // its downloads wait for it
    for (const d of spec.downloads) {
      const x = downloads.find((y) => y.id === d);
      if (x && ["queued", "connecting", "downloading"].includes(x.status)) patch(d, { status: "paused", speed: 0, active_connections: 0 });
    }
    emitSchedules();
    return sid;
  },
  setScheduleEnabled: async (id: number, enabled: boolean) => {
    schedules = schedules.map((x) => (x.id === id ? { ...describe(id, x, { last: x.last }), enabled, status: enabled ? "waiting" : "paused" } : x));
    emitSchedules();
  },
  afterAllCancel: async () => {
    if (afterTimer) window.clearInterval(afterTimer);
    afterTimer = null;
    afterEmit({ action: settings.after_all, left: null, reason: "cancelled" });
  },
  afterAllNow: async () => {
    if (afterTimer) window.clearInterval(afterTimer);
    afterTimer = null;
    afterEmit({ action: settings.after_all, left: null, reason: "done" });
  },
  onAfterAll: (cb: (n: AfterAllNotice) => void) => {
    const key = Symbol();
    afterListeners.set(key, cb);
    return Promise.resolve(() => {
      afterListeners.delete(key);
    });
  },
  onAfterAllFailed: (_cb: (why: string) => void) => Promise.resolve(() => {}),
  deleteSchedule: async (id: number) => {
    schedules = schedules.filter((x) => x.id !== id);
    emitSchedules();
  },
  onSchedules: (cb: (list: Schedule[]) => void) => {
    // one key per subscription: the same callback may be subscribed twice
    const key = Symbol();
    scheduleListeners.set(key, cb);
    return Promise.resolve(() => {
      scheduleListeners.delete(key);
    });
  },
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
