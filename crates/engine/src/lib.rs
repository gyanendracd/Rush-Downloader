//! RushDownload download engine.
//!
//! UI-agnostic: the desktop and mobile apps, tests and any future CLI all
//! drive downloads through [`Engine`]. It owns the queue, persists progress
//! so downloads survive restarts, and reports changes through a watch channel.

mod dns;
mod limiter;
mod media;
mod probe;
mod profile;
mod scan;
mod torrent;
mod task;

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering::Relaxed;
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::runtime::Handle;
use tokio::sync::watch;

pub use media::MediaInfo;
pub use profile::{Fetcher, PlaylistInfo, ProfileFace, ProfileRef};
pub use scan::{Found, FoundKind};
pub use torrent::{TorrentFile, TorrentPreview};

use limiter::Limiter;
use task::{Fail, Live, MAX_CONNECTIONS};

// Identify honestly: hosts with bot protection drop clients that claim to be
// a browser but do not behave like one.
const USER_AGENT: &str = concat!("RushDownload/", env!("CARGO_PKG_VERSION"));

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Queued,
    Connecting,
    Downloading,
    Paused,
    Completed,
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Settings {
    pub download_dir: PathBuf,
    /// Downloads running at the same time; the rest wait in the queue.
    pub max_concurrent: usize,
    /// Connections per download.
    pub connections: usize,
    /// Global limit in bytes per second; 0 = unlimited.
    pub speed_limit: u64,
    /// Browser whose signed-in session site downloads may use (for sites that
    /// ask for a sign-in). `None` = never read any browser's cookies.
    #[serde(default)]
    pub cookies_browser: Option<String>,
    /// Look site names up at Cloudflare (1.1.1.1) over HTTPS instead of at
    /// the network's own resolver. On unless the user turns it off.
    #[serde(default = "on")]
    pub secure_dns: bool,
}

fn on() -> bool {
    true
}

/// Browsers the site backend can read a sign-in from.
pub const COOKIE_BROWSERS: [&str; 6] = ["firefox", "chrome", "edge", "brave", "opera", "vivaldi"];

impl Settings {
    pub fn new(download_dir: PathBuf) -> Self {
        Self {
            download_dir,
            max_concurrent: 3,
            connections: 16,
            speed_limit: 0,
            cookies_browser: None,
            secure_dns: true,
        }
    }

    fn normalized(mut self) -> Self {
        self.max_concurrent = self.max_concurrent.clamp(1, 10);
        self.connections = self.connections.clamp(1, MAX_CONNECTIONS);
        self.cookies_browser = self
            .cookies_browser
            .map(|b| b.to_ascii_lowercase())
            .filter(|b| COOKIE_BROWSERS.contains(&b.as_str()));
        self
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct ToolStatus {
    pub yt_dlp: bool,
    pub ffmpeg: bool,
    pub gallery_dl: bool,
}

/// What to take from a video page. Ignored for direct file links.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaOptions {
    /// Highest video resolution wanted (e.g. 720); `None` = best available.
    #[serde(default)]
    pub max_height: Option<u32>,
    /// Save only the sound, as an MP3.
    #[serde(default)]
    pub audio_only: bool,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct NewDownload {
    pub url: String,
    #[serde(default)]
    pub dir: Option<PathBuf>,
    #[serde(default)]
    pub filename: Option<String>,
    #[serde(default)]
    pub connections: Option<usize>,
    /// Extra request headers (cookies, referer...), e.g. from a browser.
    #[serde(default)]
    pub headers: Vec<(String, String)>,
    #[serde(default)]
    pub media: MediaOptions,
    /// Skip file detection and hand the link straight to the site backend
    /// (streaming playlists, embedded players).
    #[serde(default)]
    pub via_site: bool,
    /// For a torrent: which of its files to download, by index. `None` = all.
    #[serde(default)]
    pub torrent_files: Option<Vec<usize>>,
}

/// Snapshot of one download for display.
#[derive(Clone, Debug, Serialize)]
pub struct TaskInfo {
    pub id: u64,
    pub url: String,
    pub filename: String,
    pub dir: PathBuf,
    /// Final file path (exists once the download has completed).
    pub path: PathBuf,
    pub status: Status,
    pub error: Option<String>,
    pub total: Option<u64>,
    pub downloaded: u64,
    /// Bytes per second.
    pub speed: u64,
    pub resumable: bool,
    /// Downloaded from a web page through the site backend.
    pub media: bool,
    /// A BitTorrent download; `path` is the torrent's own folder.
    pub torrent: bool,
    pub connections: usize,
    pub active_connections: usize,
    pub created_at: u64,
    /// `[start, position, end]` per segment.
    pub segments: Vec<[u64; 3]>,
    /// Connection attempts that have failed in a row while connecting.
    pub connect_failures: u64,
    /// A whole social media profile; `path` is the profile's own folder.
    pub profile: bool,
    /// Files fetched so far, for profile downloads.
    pub items: u64,
    /// The file a running profile or playlist download is fetching right now.
    pub current: Option<CurrentFile>,
    /// The picture the site shows for a video or playlist, once it is known.
    pub thumbnail: Option<String>,
    /// When a finished download's file was found to be gone, in seconds
    /// since 1970; `None` while it is where it was saved.
    pub deleted_at: Option<u64>,
}

/// One video or file of a profile or playlist download, for display.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemInfo {
    pub title: String,
    pub state: ItemState,
    /// Its size on disk, once it is there.
    pub size: Option<u64>,
    /// The name of its file, once it is there.
    pub file: Option<String>,
    /// Where that file is on this computer, so that it can be shown.
    pub path: Option<PathBuf>,
    /// Its picture on the site.
    pub thumbnail: Option<String>,
    /// How long a video runs, in seconds.
    pub duration: Option<u64>,
    /// Who posted it.
    pub channel: Option<String>,
    /// Its own page on the site.
    pub url: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ItemState {
    Done,
    Downloading,
    Waiting,
    /// The download finished without it: the site would not give it.
    Missed,
    /// It was downloaded, and its file has since been deleted or moved away.
    Deleted,
}

/// One file of a profile or playlist, while it is being fetched.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CurrentFile {
    /// The site's own id for it, where there is one.
    #[serde(skip)]
    pub id: String,
    pub name: String,
    pub downloaded: u64,
    /// Its full size, where the site says.
    pub total: Option<u64>,
    /// Its place in a playlist, counted from 1.
    pub index: Option<u64>,
    /// How many files the playlist lists.
    pub count: Option<u64>,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Record {
    pub id: u64,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub dir: PathBuf,
    pub filename: String,
    pub user_named: bool,
    /// The file name is final and a `.part` file was created for it.
    pub part_started: bool,
    pub status: Status,
    pub error: Option<String>,
    pub total: Option<u64>,
    pub resumable: bool,
    pub etag: Option<String>,
    pub connections: usize,
    pub created_at: u64,
    pub segments: Vec<[u64; 3]>,
    pub downloaded: u64,
    #[serde(default)]
    pub media: bool,
    #[serde(default)]
    pub media_options: MediaOptions,
    #[serde(default)]
    pub torrent: bool,
    /// The app's own copy of a `.torrent` file opened from disk, so the
    /// download can resume even if the original file is moved or deleted.
    #[serde(default)]
    pub torrent_file: Option<PathBuf>,
    #[serde(default)]
    pub only_files: Option<Vec<usize>>,
    /// Which copy of the same video this site download is: 0 for the first,
    /// 1 for "name (2)" and so on. Decided when it first runs.
    #[serde(default)]
    pub media_copy: Option<u32>,
    /// For a site download given a name: the file name (without its ending)
    /// it settled on, quality and copy number included. Decided when it
    /// first runs and kept, so a pause and resume lands on the same file.
    #[serde(default)]
    pub named_stem: Option<String>,
    /// Downloads everything a social media profile has posted.
    #[serde(default)]
    pub profile: bool,
    /// Files fetched so far, for profile downloads.
    #[serde(default)]
    pub items: u64,
    /// The picture the site shows for it: a video's own, or a playlist's
    /// first video's. Learned when the site is first asked about the link.
    #[serde(default)]
    pub thumbnail: Option<String>,
    /// When a finished download's file was first found to be gone (deleted
    /// or moved away outside the app), in seconds since 1970. Cleared if the
    /// file comes back.
    #[serde(default)]
    pub deleted_at: Option<u64>,
    /// A collection the downloading programs found at a link the app does
    /// not read itself (someone's page on any site they know), and which
    /// program fetches it. Kept, so a resumed download fetches it the same way.
    #[serde(default)]
    pub collection: Option<profile::ProfileRef>,
}

struct Entry {
    rec: Record,
    live: Option<Arc<Live>>,
    speed: f64,
    last_bytes: u64,
}

impl Entry {
    /// Copies progress from the running task into the record.
    fn sync(&mut self) {
        if let Some(live) = &self.live {
            self.rec.downloaded = live.downloaded.load(Relaxed);
            if self.rec.profile {
                self.rec.items = live.items.load(Relaxed);
            }
            let hint = live.total_hint.load(Relaxed);
            if hint > 0 {
                self.rec.total = Some(hint);
            }
            let segs = live.segs.lock().unwrap();
            if !segs.is_empty() {
                self.rec.segments = segs.iter().map(|s| [s.start, s.pos, s.end]).collect();
            }
        }
    }

    fn info(&self) -> TaskInfo {
        let r = &self.rec;
        TaskInfo {
            id: r.id,
            url: r.url.clone(),
            filename: r.filename.clone(),
            dir: r.dir.clone(),
            path: r.dir.join(&r.filename),
            status: r.status,
            error: r.error.clone(),
            total: r.total,
            downloaded: r.downloaded,
            speed: self.speed as u64,
            resumable: r.resumable,
            media: r.media,
            torrent: r.torrent,
            connections: r.connections,
            active_connections: self.live.as_ref().map_or(0, |l| l.conns.load(Relaxed)),
            created_at: r.created_at,
            segments: r.segments.clone(),
            connect_failures: self.live.as_ref().map_or(0, |l| l.connect_failures.load(Relaxed)),
            profile: r.profile,
            items: r.items,
            current: self.live.as_ref().and_then(|l| l.current.lock().unwrap().clone()),
            thumbnail: r.thumbnail.clone(),
            deleted_at: r.deleted_at,
        }
    }
}

struct State {
    entries: Vec<Entry>,
    next_id: u64,
    settings: Settings,
}

#[derive(Serialize, Deserialize)]
struct Saved {
    next_id: u64,
    settings: Settings,
    tasks: Vec<Record>,
}

pub(crate) struct Inner {
    pub(crate) client: reqwest::Client,
    pub(crate) dns: Arc<dns::SecureDns>,
    pub(crate) limiter: Arc<Limiter>,
    /// Started the first time a torrent is downloaded.
    pub(crate) torrents: tokio::sync::OnceCell<Arc<librqbit::Session>>,
    /// What each playlist or channel download lists, by download id. Kept
    /// in memory only; asked from the site again when a download resumes.
    pub(crate) lists: Mutex<std::collections::HashMap<u64, profile::Listed>>,
    /// What the downloading programs said each link holds, asked once per
    /// link: a collection, or `None` for a single item (or nothing they know).
    collections: Mutex<std::collections::HashMap<String, Option<ProfileRef>>>,
    state: Mutex<State>,
    state_file: PathBuf,
    save_lock: Mutex<()>,
    version: watch::Sender<u64>,
    rt: Handle,
}

/// Handle to the download engine; cheap to clone.
#[derive(Clone)]
pub struct Engine {
    inner: Arc<Inner>,
}

impl Engine {
    /// Loads saved state from `state_dir` and resumes interrupted downloads.
    /// Must be called from within a Tokio runtime.
    pub fn new(state_dir: impl AsRef<Path>, defaults: Settings) -> Result<Engine, String> {
        let state_dir = state_dir.as_ref();
        std::fs::create_dir_all(state_dir).map_err(|e| format!("state folder: {e}"))?;
        let state_file = state_dir.join("downloads.json");

        let saved: Option<Saved> = std::fs::read(&state_file)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok());
        let (entries, next_id, settings) = match saved {
            Some(s) => {
                let entries = s
                    .tasks
                    .into_iter()
                    .map(|mut rec| {
                        // Whatever was running when the app closed continues.
                        if matches!(rec.status, Status::Connecting | Status::Downloading) {
                            rec.status = Status::Queued;
                        }
                        Entry {
                            rec,
                            live: None,
                            speed: 0.0,
                            last_bytes: 0,
                        }
                    })
                    .collect();
                (entries, s.next_id, s.settings.normalized())
            }
            None => (Vec::new(), 1, defaults.normalized()),
        };

        let dns = dns::SecureDns::new(settings.secure_dns);
        let client = reqwest::Client::builder()
            .dns_resolver(Arc::new(dns::Resolver(dns.clone())))
            .user_agent(USER_AGENT)
            .default_headers(
                [(reqwest::header::ACCEPT, reqwest::header::HeaderValue::from_static("*/*"))]
                    .into_iter()
                    .collect(),
            )
            // HTTP/2 would multiplex every segment over one TCP connection,
            // which defeats the point of parallel connections.
            .http1_only()
            .connect_timeout(Duration::from_secs(15))
            .read_timeout(Duration::from_secs(20))
            .build()
            .map_err(|e| format!("http client: {e}"))?;

        let inner = Arc::new(Inner {
            client,
            dns,
            limiter: Arc::new(Limiter::new(settings.speed_limit)),
            torrents: tokio::sync::OnceCell::new(),
            lists: Mutex::new(Default::default()),
            collections: Mutex::new(Default::default()),
            state: Mutex::new(State {
                entries,
                next_id,
                settings,
            }),
            state_file,
            save_lock: Mutex::new(()),
            version: watch::channel(0).0,
            rt: Handle::current(),
        });

        inner.rt.spawn(ticker(Arc::downgrade(&inner)));
        inner.pump();
        Ok(Engine { inner })
    }

    /// Fires whenever anything observable changes.
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.inner.version.subscribe()
    }

    pub fn list(&self) -> Vec<TaskInfo> {
        let mut st = self.inner.state.lock().unwrap();
        st.entries
            .iter_mut()
            .map(|e| {
                e.sync();
                e.info()
            })
            .collect()
    }

    /// The videos or files of a profile or playlist download, each with how
    /// far along it is. Empty for any other kind of download.
    pub fn items(&self, id: u64) -> Vec<ItemInfo> {
        self.inner.items(id)
    }

    /// Whether the helper programs for site downloads are installed.
    pub fn tools() -> ToolStatus {
        ToolStatus {
            yt_dlp: media::find_tool("yt-dlp").is_some(),
            ffmpeg: media::find_tool("ffmpeg").is_some(),
            gallery_dl: media::find_tool("gallery-dl").is_some(),
        }
    }

    /// While secure DNS is on: the proxy on this computer that looks site
    /// names up at Cloudflare. The embedded browser is sent through it.
    pub fn dns_proxy(&self) -> Option<std::net::SocketAddr> {
        let _runtime = self.inner.rt.enter();
        self.inner.dns.proxy()
    }

    /// The sign-ins a browser on this computer keeps, as cookie-file lines.
    /// `browser` is one of the names in `media::COOKIE_BROWSERS`.
    pub async fn browser_cookies(browser: &str) -> Result<Vec<String>, String> {
        media::browser_cookies(browser).await
    }

    /// The file holding sign-ins made inside the app (it may not exist yet).
    pub fn cookies_file(&self) -> PathBuf {
        self.inner.cookies_file()
    }

    /// The profile a link points at: one the app reads from the link itself,
    /// or one the downloading programs have already said is a collection.
    /// Such a link downloads everything on it.
    pub fn profile_of(&self, url: &str) -> Option<ProfileRef> {
        self.inner.resolve(url)
    }

    /// Asks the downloading programs whether a link is a whole collection
    /// (someone's videos, photos or tracks) on any site they know. Asked once
    /// per link; a link the app already reads itself is answered at once.
    /// Takes a few seconds the first time.
    pub async fn collection_of(&self, url: &str) -> Option<ProfileRef> {
        if let Some(known) = self.inner.resolve(url) {
            return Some(known);
        }
        if let Some(answer) = self.inner.collections.lock().unwrap().get(url) {
            return answer.clone();
        }
        let found = profile::find_collection(&self.inner, url).await;
        self.inner.collections.lock().unwrap().insert(url.to_string(), found.clone());
        found
    }

    /// A playlist's name, picture and number of videos, before downloading.
    pub async fn playlist_info(&self, url: &str) -> Result<PlaylistInfo, String> {
        profile::playlist_info(&self.inner, url).await
    }

    /// Whose profile a link points at: the name it shows, its picture and
    /// its line about itself. Quick, and works without being signed in.
    pub async fn profile_face(&self, url: &str) -> ProfileFace {
        profile::profile_face(&self.inner, url).await
    }

    /// Everything a profile shows a visitor who is not signed in, numbers
    /// included (posts, followers, following, joined), for the sites that
    /// tell. Asked with no sign-in, so it never depends on one.
    pub async fn profile_stats(&self, url: &str) -> ProfileFace {
        profile::profile_stats(&self.inner, url).await
    }

    /// Reads what a torrent contains (name, files, sizes) without downloading
    /// it. `source` is a magnet link, a link to a `.torrent` file, or the
    /// path of a `.torrent` file on this computer.
    pub async fn torrent_preview(&self, source: &str) -> Result<TorrentPreview, String> {
        torrent::preview(&self.inner, source).await
    }

    /// How large the file behind a link is, when its server says. Nothing is
    /// downloaded; `headers` are sent along (a Referer, a browser's name).
    pub async fn link_size(&self, url: &str, headers: &[(String, String)]) -> Option<u64> {
        let asked = probe::probe(&self.inner.client, url, headers);
        let found = tokio::time::timeout(Duration::from_secs(10), asked).await.ok()?.ok()?;
        found.total.filter(|size| *size > 0 && !found.is_html)
    }

    /// A video page's title, picture and resolutions, before downloading.
    pub async fn media_info(&self, url: &str) -> Result<MediaInfo, String> {
        media::info(&self.inner, url).await
    }

    /// Deep scan: lists the media links found on a web page.
    pub async fn scan_page(&self, url: &str) -> Result<Vec<Found>, String> {
        scan::scan(&self.inner.client, url).await
    }

    pub fn settings(&self) -> Settings {
        self.inner.state.lock().unwrap().settings.clone()
    }

    pub fn set_settings(&self, settings: Settings) {
        let settings = settings.normalized();
        self.inner.limiter.set_rate(settings.speed_limit);
        self.inner.dns.set(settings.secure_dns);
        self.inner.state.lock().unwrap().settings = settings;
        self.inner.changed();
        self.inner.pump();
    }

    pub fn add(&self, new: NewDownload) -> Result<u64, String> {
        let url = torrent::clean(&new.url);
        let local_torrent = torrent::is_local_torrent(&url);
        let is_torrent = local_torrent || torrent::is_torrent_link(&url);
        let default_name = if local_torrent {
            torrent::local_name(&url)
        } else {
            let parsed =
                reqwest::Url::parse(&url).map_err(|_| "That is not a valid link".to_string())?;
            if !is_torrent && !matches!(parsed.scheme(), "http" | "https") {
                return Err("Only http, https, magnet links and .torrent files are supported".into());
            }
            if is_torrent {
                torrent::default_name(&url)
            } else {
                probe::sanitize(
                    &probe::url_filename(&url)
                        .or_else(|| parsed.host_str().map(str::to_string))
                        .unwrap_or_default(),
                )
            }
        };
        // A link to a whole profile downloads everything on it.
        let profile = if is_torrent || new.via_site {
            None
        } else {
            self.inner.resolve(&url)
        };
        let default_name = match &profile {
            Some(p) => profile::folder_name(p),
            None => default_name,
        };
        let only_files = new
            .torrent_files
            .filter(|files| is_torrent && !files.is_empty());
        let user_name = new
            .filename
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map(probe::sanitize);

        let id = {
            let mut st = self.inner.state.lock().unwrap();
            let id = st.next_id;
            st.next_id += 1;
            // Keep our own copy of a .torrent file opened from disk.
            let torrent_file = if local_torrent {
                let folder = self
                    .inner
                    .state_file
                    .parent()
                    .map(|p| p.join("torrents"))
                    .unwrap_or_default();
                let copy = folder.join(format!("{id}.torrent"));
                std::fs::create_dir_all(&folder)
                    .and_then(|_| std::fs::copy(&url, &copy))
                    .map_err(|e| format!("Could not read the torrent file: {e}"))?;
                Some(copy)
            } else {
                None
            };
            let rec = Record {
                id,
                headers: new.headers,
                dir: new.dir.unwrap_or_else(|| st.settings.download_dir.clone()),
                filename: user_name.clone().unwrap_or(default_name),
                url,
                user_named: user_name.is_some(),
                part_started: false,
                status: Status::Queued,
                error: None,
                total: None,
                resumable: false,
                etag: None,
                connections: new
                    .connections
                    .unwrap_or(st.settings.connections)
                    .clamp(1, MAX_CONNECTIONS),
                created_at: now(),
                segments: Vec::new(),
                downloaded: 0,
                media: new.via_site && !is_torrent,
                media_options: new.media,
                torrent: is_torrent,
                torrent_file,
                only_files,
                media_copy: None,
                named_stem: None,
                profile: profile.is_some(),
                items: 0,
                thumbnail: None,
                deleted_at: None,
                collection: profile.clone().filter(|p| p.by.is_some()),
            };
            st.entries.push(Entry {
                rec,
                live: None,
                speed: 0.0,
                last_bytes: 0,
            });
            id
        };
        self.inner.changed();
        self.inner.pump();
        Ok(id)
    }

    pub fn pause(&self, id: u64) {
        {
            let mut st = self.inner.state.lock().unwrap();
            let Some(e) = st.entries.iter_mut().find(|e| e.rec.id == id) else {
                return;
            };
            if !matches!(
                e.rec.status,
                Status::Queued | Status::Connecting | Status::Downloading
            ) {
                return;
            }
            e.rec.status = Status::Paused;
            if let Some(live) = &e.live {
                live.cancel.cancel();
            }
        }
        self.inner.changed();
        self.inner.pump();
    }

    /// Resumes a paused download or retries a failed one.
    pub fn resume(&self, id: u64) {
        {
            let mut st = self.inner.state.lock().unwrap();
            let Some(e) = st.entries.iter_mut().find(|e| e.rec.id == id) else {
                return;
            };
            if !matches!(e.rec.status, Status::Paused | Status::Failed) {
                return;
            }
            e.rec.status = Status::Queued;
            e.rec.error = None;
        }
        self.inner.changed();
        self.inner.pump();
    }

    /// Removes a download from the list. Unfinished data is always deleted;
    /// a completed file is deleted only when `delete_file` is set.
    pub fn remove(&self, id: u64, delete_file: bool) {
        {
            let mut st = self.inner.state.lock().unwrap();
            let Some(i) = st.entries.iter().position(|e| e.rec.id == id) else {
                return;
            };
            self.inner.lists.lock().unwrap().remove(&id);
            if let Some(live) = st.entries[i].live.clone() {
                // The running task cleans up after itself when it stops.
                live.removed.store(true, Relaxed);
                live.cancel.cancel();
                st.entries[i].rec.status = Status::Paused;
            } else {
                let e = st.entries.remove(i);
                delete_files(&e.rec, delete_file);
            }
        }
        // Gone from the list, gone from the record of what it fetched.
        let _ = std::fs::remove_file(self.inner.remembered_file(id));
        self.inner.changed();
        self.inner.pump();
    }
}

fn delete_files(rec: &Record, delete_completed: bool) {
    if rec.profile {
        if rec.part_started {
            if rec.status != Status::Completed {
                // Finished photos and videos stay; only the leftovers go.
                profile::discard_unfinished(&rec.dir, &rec.filename);
            } else if delete_completed {
                torrent::remove_data(&rec.dir, &rec.filename);
            }
        }
        return;
    }
    if rec.torrent {
        if let Some(copy) = &rec.torrent_file {
            let _ = std::fs::remove_file(copy);
        }
        // Unfinished data always goes; a finished torrent only when asked.
        if rec.part_started && (rec.status != Status::Completed || delete_completed) {
            torrent::remove_data(&rec.dir, &rec.filename);
        }
        return;
    }
    if rec.status == Status::Completed {
        if delete_completed {
            let _ = std::fs::remove_file(rec.dir.join(&rec.filename));
        }
    } else if rec.media {
        media::remove_work_dir(&rec.dir, rec.id);
        // downloads started before each one had its own folder
        if rec.media_copy.is_none() {
            media::cleanup(&rec.dir, &rec.filename);
        }
    } else if rec.part_started {
        let _ = std::fs::remove_file(probe::part_path(&rec.dir, &rec.filename));
    }
}

/// When a site could not be reached at all, says what to try next. Some
/// networks answer DNS lookups for certain sites with an address that never
/// responds; a browser with secure DNS still opens them.
fn with_network_hint(msg: String, secure_dns: bool) -> String {
    let lower = msg.to_ascii_lowercase();
    let unreachable = [
        "timed out",
        "dns error",
        "could not find",
        "getaddrinfo",
        "connection refused",
        "connection reset",
        "unable to download webpage",
        "(connect)",
    ]
    .iter()
    .any(|sign| lower.contains(sign));
    if !unreachable || msg.contains("Secure DNS") {
        return msg;
    }
    if secure_dns {
        format!("{msg}. The site could not be reached even with Secure DNS on; this network may be blocking it")
    } else {
        format!("{msg}. If this site opens in your browser, turn on Secure DNS under Settings, Network, then retry")
    }
}

/// What a finished download's folder holds now, together with what it is
/// remembered to have fetched: anything remembered that is no longer there
/// stays in the list, as deleted. The remembered order is kept.
fn with_remembered(now_there: Vec<ItemInfo>, remembered: Vec<ItemInfo>) -> Vec<ItemInfo> {
    if remembered.is_empty() {
        return now_there;
    }
    // The same item, seen now and remembered: by its file's name, or, where
    // the file is gone and with it the name, by its title.
    let same = |now: &ItemInfo, old: &ItemInfo| match (&now.file, &old.file) {
        (Some(a), Some(b)) => a == b,
        _ => now.title == old.title,
    };
    let mut there: Vec<Option<ItemInfo>> = now_there.into_iter().map(Some).collect();
    let mut out = Vec::with_capacity(there.len().max(remembered.len()));
    for old in remembered {
        let found = there.iter_mut().find(|slot| slot.as_ref().is_some_and(|item| same(item, &old)));
        out.push(match found.and_then(Option::take) {
            // Still there, as it was fetched.
            Some(item) if item.state == ItemState::Done => item,
            // Gone: what is remembered of it says more than what is left.
            _ => ItemInfo { state: ItemState::Deleted, path: None, ..old },
        });
    }
    out.extend(there.into_iter().flatten());
    out
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

impl Inner {
    /// Picks, once, which copy of the same video (same link, same quality) a
    /// site download is, so each copy ends up in a file of its own.
    pub(crate) fn claim_media_copy(&self, id: u64) -> u32 {
        let mut st = self.state.lock().unwrap();
        let Some(me) = st.entries.iter().find(|e| e.rec.id == id).map(|e| e.rec.clone()) else {
            return 0;
        };
        if let Some(copy) = me.media_copy {
            return copy;
        }
        let taken: Vec<u32> = st
            .entries
            .iter()
            .filter(|e| {
                e.rec.id != id
                    && e.rec.url == me.url
                    && e.rec.dir == me.dir
                    && e.rec.media_options == me.media_options
            })
            .filter_map(|e| e.rec.media_copy)
            .collect();
        let copy = (0..).find(|n| !taken.contains(n)).unwrap_or(0);
        if let Some(e) = st.entries.iter_mut().find(|e| e.rec.id == id) {
            e.rec.media_copy = Some(copy);
        }
        copy
    }

    /// Settles, once, the file name of a site download that was given a name.
    /// `base` is that name with the quality in it; a number is added when a
    /// file or another download already has it, so that no two downloads
    /// ever share a file. (yt-dlp treats a file that is already there as
    /// "done" and would hand the old file back instead of downloading.)
    pub(crate) fn claim_named_stem(&self, id: u64, base: &str) -> String {
        let mut st = self.state.lock().unwrap();
        let Some(me) = st.entries.iter().find(|e| e.rec.id == id).map(|e| e.rec.clone()) else {
            return base.to_string();
        };
        if let Some(stem) = me.named_stem {
            return stem;
        }
        let on_disk: Vec<String> = std::fs::read_dir(&me.dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().is_file())
            .filter_map(|e| e.path().file_stem().map(|s| s.to_string_lossy().to_lowercase()))
            .collect();
        let taken = |candidate: &str| {
            let lower = candidate.to_lowercase();
            on_disk.contains(&lower)
                || st.entries.iter().any(|e| {
                    e.rec.id != id
                        && e.rec.dir == me.dir
                        && e.rec.named_stem.as_deref().is_some_and(|other| other.to_lowercase() == lower)
                })
        };
        let stem = (0..10_000u32)
            .map(|n| if n == 0 { base.to_string() } else { format!("{base} ({})", n + 1) })
            .find(|candidate| !taken(candidate))
            .unwrap_or_else(|| base.to_string());
        if let Some(e) = st.entries.iter_mut().find(|e| e.rec.id == id) {
            e.rec.named_stem = Some(stem.clone());
        }
        stem
    }

    pub(crate) fn download_dir(&self) -> PathBuf {
        self.state.lock().unwrap().settings.download_dir.clone()
    }

    pub(crate) fn cookies_browser(&self) -> Option<String> {
        self.state.lock().unwrap().settings.cookies_browser.clone()
    }

    /// Where the sign-ins made inside the app are kept, in the cookie-file
    /// format yt-dlp and gallery-dl read.
    pub(crate) fn cookies_file(&self) -> PathBuf {
        self.state_file
            .parent()
            .map(|p| p.join("cookies.txt"))
            .unwrap_or_else(|| PathBuf::from("cookies.txt"))
    }

    /// Adds the user's sign-in to a yt-dlp or gallery-dl command: the
    /// sign-ins made inside the app if there are any, otherwise the browser
    /// chosen in Settings. The tools write to a cookie file they are given,
    /// so with `scratch` each run gets a copy of its own there.
    pub(crate) fn add_sign_in(&self, cmd: &mut tokio::process::Command, scratch: Option<&Path>) {
        // With secure DNS on, the helper program goes through the local
        // proxy, which looks names up the same way the app does.
        if let Some(proxy) = self.dns.proxy() {
            cmd.arg("--proxy").arg(format!("http://{proxy}"));
        }
        let saved = self.cookies_file();
        if std::fs::metadata(&saved).map(|m| m.len() > 0).unwrap_or(false) {
            let own = scratch.map(|dir| dir.join(".rush-cookies.txt"));
            let file = match own {
                Some(copy) if std::fs::copy(&saved, &copy).is_ok() => copy,
                _ => saved,
            };
            cmd.arg("--cookies").arg(file);
        } else if let Some(browser) = self.cookies_browser() {
            cmd.arg("--cookies-from-browser").arg(browser);
        }
    }

    pub(crate) fn record(&self, id: u64) -> Option<Record> {
        let st = self.state.lock().unwrap();
        st.entries
            .iter()
            .find(|e| e.rec.id == id)
            .map(|e| e.rec.clone())
    }

    pub(crate) fn update(&self, id: u64, f: impl FnOnce(&mut Record)) {
        {
            let mut st = self.state.lock().unwrap();
            if let Some(e) = st.entries.iter_mut().find(|e| e.rec.id == id) {
                f(&mut e.rec);
            }
        }
        self.notify();
    }

    fn notify(&self) {
        self.version.send_modify(|v| *v += 1);
    }

    /// Persists and announces a change.
    fn changed(&self) {
        self.save();
        self.notify();
    }

    /// The profile a link points at: read from the link, or found earlier by
    /// asking the downloading programs (this run, or kept with a download).
    pub(crate) fn resolve(&self, url: &str) -> Option<ProfileRef> {
        if let Some(known) = profile::detect(url) {
            return Some(known);
        }
        let asked = self.collections.lock().unwrap().get(url).cloned().flatten();
        asked.or_else(|| {
            let st = self.state.lock().unwrap();
            st.entries.iter().find(|e| e.rec.url == url).and_then(|e| e.rec.collection.clone())
        })
    }

    /// The videos or files of a profile or playlist download, each with how
    /// far along it is. Once the download has finished, what it fetched is
    /// remembered: a file that has since been deleted is still listed, as
    /// deleted.
    fn items(&self, id: u64) -> Vec<ItemInfo> {
        let (folder, current, finished) = {
            let st = self.state.lock().unwrap();
            let Some(e) = st.entries.iter().find(|e| e.rec.id == id && e.rec.profile) else {
                return Vec::new();
            };
            let current = e.live.as_ref().and_then(|l| l.current.lock().unwrap().clone());
            (
                e.rec.part_started.then(|| e.rec.dir.join(&e.rec.filename)),
                current,
                e.rec.status == Status::Completed && e.live.is_none(),
            )
        };
        let listed = self.lists.lock().unwrap().get(&id).filter(|l| !l.items.is_empty()).cloned();
        let mut items = match listed {
            Some(listed) => listed.show(current.as_ref(), folder.as_deref()),
            None => {
                // Nothing listed (a photo profile, or the app was restarted): what is in the folder.
                let mut items = folder.map(|f| profile::on_disk(&f)).unwrap_or_default();
                if let Some(file) = current.filter(|c| c.total.is_none()) {
                    items.push(ItemInfo {
                        title: file.name,
                        state: ItemState::Downloading,
                        size: Some(file.downloaded),
                        file: None,
                        path: None,
                        thumbnail: None,
                        duration: None,
                        channel: None,
                        url: None,
                    });
                }
                items
            }
        };
        if finished {
            items = with_remembered(items, self.remembered_items(id));
        }
        items
    }

    /// Where the list of what a finished profile or playlist download
    /// fetched is kept, so that it outlives the files themselves.
    fn remembered_file(&self, id: u64) -> PathBuf {
        let dir = self.state_file.parent().map(|p| p.join("history")).unwrap_or_else(|| PathBuf::from("history"));
        dir.join(format!("{id}.json"))
    }

    fn remembered_items(&self, id: u64) -> Vec<ItemInfo> {
        std::fs::read(self.remembered_file(id))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    /// Writes down what a profile or playlist download fetched, when it
    /// finishes. Only what is there is written: a later, shorter look at the
    /// folder must not make the record forget.
    fn remember_items(&self, id: u64) {
        let fetched: Vec<ItemInfo> = self.items(id).into_iter().filter(|item| item.state == ItemState::Done).collect();
        if fetched.is_empty() {
            return;
        }
        let file = self.remembered_file(id);
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(bytes) = serde_json::to_vec(&fetched) {
            let _ = std::fs::write(file, bytes);
        }
    }

    /// Looks at every finished download's file: one that is gone is marked
    /// as deleted from that moment, and one that is back loses the mark.
    /// The files are looked at without holding up anything else.
    fn notice_deleted(&self) {
        let finished: Vec<(u64, PathBuf, bool)> = {
            let st = self.state.lock().unwrap();
            st.entries
                .iter()
                .filter(|e| e.live.is_none() && e.rec.status == Status::Completed)
                .map(|e| (e.rec.id, e.rec.dir.join(&e.rec.filename), e.rec.deleted_at.is_some()))
                .collect()
        };
        let turned: Vec<(u64, bool)> = finished
            .into_iter()
            .filter_map(|(id, path, marked)| {
                let gone = !path.exists();
                (gone != marked).then_some((id, gone))
            })
            .collect();
        if turned.is_empty() {
            return;
        }
        {
            let mut st = self.state.lock().unwrap();
            for (id, gone) in turned {
                if let Some(e) = st.entries.iter_mut().find(|e| e.rec.id == id && e.rec.status == Status::Completed) {
                    e.rec.deleted_at = gone.then(now);
                }
            }
        }
        self.changed();
    }

    fn save(&self) {
        let _guard = self.save_lock.lock().unwrap();
        let saved = {
            let mut st = self.state.lock().unwrap();
            Saved {
                next_id: st.next_id,
                settings: st.settings.clone(),
                tasks: st
                    .entries
                    .iter_mut()
                    .map(|e| {
                        e.sync();
                        e.rec.clone()
                    })
                    .collect(),
            }
        };
        let Ok(bytes) = serde_json::to_vec(&saved) else {
            return;
        };
        let tmp = self.state_file.with_extension("json.tmp");
        if std::fs::write(&tmp, bytes).is_ok() {
            let _ = std::fs::rename(&tmp, &self.state_file);
        }
    }

    /// Starts queued downloads while there are free slots.
    fn pump(self: &Arc<Self>) {
        let mut started = false;
        {
            let mut st = self.state.lock().unwrap();
            let mut running = st.entries.iter().filter(|e| e.live.is_some()).count();
            let max = st.settings.max_concurrent;
            for e in st.entries.iter_mut() {
                if running >= max {
                    break;
                }
                // A paused task may still be winding down (`live` set); it is
                // picked up again by the pump its own exit triggers.
                if e.rec.status != Status::Queued || e.live.is_some() {
                    continue;
                }
                e.rec.status = Status::Connecting;
                e.rec.error = None;
                let live = Arc::new(Live::new());
                e.live = Some(live.clone());
                e.speed = 0.0;
                e.last_bytes = e.rec.downloaded;
                running += 1;
                started = true;
                let inner = self.clone();
                let id = e.rec.id;
                self.rt.spawn(async move {
                    let res = task::run(&inner, id, &live).await;
                    inner.finish(id, &live, res);
                });
            }
        }
        if started {
            self.notify();
        }
    }

    fn finish(self: &Arc<Self>, id: u64, live: &Arc<Live>, res: Result<(), Fail>) {
        // A profile or playlist that ran to its end has its files written
        // down, so the record of them outlives the files.
        let fetched_many = res.is_ok() && !live.removed.load(Relaxed);
        {
            let mut st = self.state.lock().unwrap();
            if let Some(i) = st.entries.iter().position(|e| e.rec.id == id) {
                let e = &mut st.entries[i];
                e.sync();
                e.live = None;
                e.speed = 0.0;
                if live.removed.load(Relaxed) {
                    let e = st.entries.remove(i);
                    delete_files(&e.rec, false);
                } else {
                    match res {
                        Ok(()) => {
                            e.rec.status = Status::Completed;
                            if let Some(t) = e.rec.total {
                                e.rec.downloaded = t;
                            }
                            e.rec.segments.clear();
                        }
                        Err(Fail::Cancelled) => {
                            // Paused (or paused and already re-queued).
                            if matches!(e.rec.status, Status::Connecting | Status::Downloading) {
                                e.rec.status = Status::Paused;
                            }
                        }
                        Err(Fail::Fatal(msg)) | Err(Fail::Retry(msg)) => {
                            // A pause that raced with the failure wins.
                            if matches!(e.rec.status, Status::Connecting | Status::Downloading) {
                                e.rec.status = Status::Failed;
                                e.rec.error = Some(with_network_hint(msg, self.dns.enabled()));
                            }
                        }
                    }
                }
            }
        }
        if fetched_many {
            self.remember_items(id);
        }
        self.changed();
        self.pump();
    }
}

/// Twice a second: refresh speeds and notify; every two seconds: persist.
async fn ticker(inner: Weak<Inner>) {
    let mut interval = tokio::time::interval(Duration::from_millis(500));
    let mut n = 0u32;
    loop {
        interval.tick().await;
        let Some(inner) = inner.upgrade() else { return };
        n = n.wrapping_add(1);
        let mut active = false;
        {
            let mut st = inner.state.lock().unwrap();
            for e in st.entries.iter_mut() {
                let Some(live) = &e.live else { continue };
                active = true;
                let bytes = live.downloaded.load(Relaxed);
                let inst = bytes.saturating_sub(e.last_bytes) as f64 * 2.0;
                e.last_bytes = bytes;
                e.speed = if e.speed == 0.0 {
                    inst
                } else {
                    e.speed * 0.6 + inst * 0.4
                };
            }
        }
        if active {
            if n % 4 == 0 {
                inner.save();
            }
            inner.notify();
        }
        // Every three seconds: has a finished file been deleted, or come back?
        if n % 6 == 1 {
            let look = inner.clone();
            let _ = tokio::task::spawn_blocking(move || look.notice_deleted()).await;
        }
    }
}

#[cfg(test)]
mod history_tests {
    use super::*;

    fn item(name: &str, state: ItemState) -> ItemInfo {
        ItemInfo {
            title: name.into(),
            state,
            size: Some(10),
            file: Some(name.into()),
            path: (state == ItemState::Done).then(|| PathBuf::from("folder").join(name)),
            thumbnail: None,
            duration: None,
            channel: None,
            url: None,
        }
    }

    #[test]
    fn what_was_fetched_is_still_listed_after_its_file_is_deleted() {
        let remembered = vec![item("a.jpg", ItemState::Done), item("b.jpg", ItemState::Done), item("c.mp4", ItemState::Done)];
        // b.jpg has been deleted from the folder; d.jpg was put there since.
        let there = vec![item("a.jpg", ItemState::Done), item("c.mp4", ItemState::Done), item("d.jpg", ItemState::Done)];
        let shown = with_remembered(there.clone(), remembered);
        let said: Vec<(&str, ItemState)> = shown.iter().map(|i| (i.title.as_str(), i.state)).collect();
        assert_eq!(
            said,
            vec![
                ("a.jpg", ItemState::Done),
                ("b.jpg", ItemState::Deleted),
                ("c.mp4", ItemState::Done),
                ("d.jpg", ItemState::Done),
            ]
        );
        // a deleted file cannot be shown; one that is there still can
        assert_eq!(shown[1].path, None);
        assert!(shown[0].path.is_some());
        // the whole folder gone: everything remembered, all of it deleted
        let all_gone = with_remembered(Vec::new(), vec![item("a.jpg", ItemState::Done), item("b.jpg", ItemState::Done)]);
        assert!(all_gone.iter().all(|i| i.state == ItemState::Deleted) && all_gone.len() == 2);
        // nothing remembered (a download from before this was kept): the folder as it is
        assert_eq!(with_remembered(there.clone(), Vec::new()), there);
        // a video the listing still knows, by its title only, once its file is gone:
        // it is the remembered one, deleted, and is not listed a second time
        let mut remembered = item("001 - A video [abc].mp4", ItemState::Done);
        remembered.title = "A video".into();
        let mut listed = item("A video", ItemState::Deleted);
        listed.file = None;
        let shown = with_remembered(vec![listed], vec![remembered]);
        assert_eq!(shown.len(), 1);
        assert_eq!((shown[0].state, shown[0].file.as_deref()), (ItemState::Deleted, Some("001 - A video [abc].mp4")));
    }
}

#[cfg(test)]
mod hint_tests {
    use super::*;

    #[test]
    fn unreachable_sites_get_a_hint_and_other_errors_do_not() {
        let off = with_network_hint("error sending request: operation timed out".into(), false);
        assert!(off.contains("turn on Secure DNS"), "{off}");
        let on = with_network_hint("dns error: secure DNS could not find x.test".into(), true);
        assert!(on.contains("even with Secure DNS on"), "{on}");
        assert_eq!(with_network_hint("HTTP 404 Not Found".into(), false), "HTTP 404 Not Found");
        assert_eq!(with_network_hint(off.clone(), false), off, "the hint is added once");
    }
}
