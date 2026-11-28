//! RushDownload download engine.
//!
//! UI-agnostic: the desktop and mobile apps, tests and any future CLI all
//! drive downloads through [`Engine`]. It owns the queue, persists progress
//! so downloads survive restarts, and reports changes through a watch channel.

mod batch;
mod bundle;
mod checksum;
mod deps;
mod dns;
mod gateway;
mod instagram;
mod limiter;
mod organize;
mod media;
mod probe;
mod profile;
mod proxy;
mod refresh;
mod remote;
mod request;
mod scan;
mod schedule;
mod secrets;
mod vault;
mod sites;
mod torrent;
mod torrent_split;
mod webseed;
mod task;

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering::Relaxed;
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::runtime::Handle;
use tokio::sync::watch;

pub use batch::BatchInfo;
pub use checksum::{parse_expected, CheckState, Checksum, HashAlgo};
pub use deps::{ToolInfo, ToolPhase, ToolProgress, ToolSource, MANAGED_TOOLS};
pub use media::MediaInfo;
pub use organize::{FileCategory, Organize};
pub use profile::{Fetcher, PlaylistInfo, ProfileFace, ProfileRef};
pub use proxy::{PasswordChange, ProxyBook, ProxyChoice, ProxyKind, ProxyProfile};
pub use refresh::{LinkCheck, LinkProblem};
pub use remote::{RemoteOptions, ServerKey};
pub use request::{AuthKind, RequestBook, RequestOptions, RequestProfile};
pub use secrets::{Keyring, MemoryStore, SecretStore};
pub use scan::{Found, FoundKind};
pub use schedule::{Event, EventKind, Repeat, ScheduleInfo, ScheduleSpec, ScheduleStatus};
pub use sites::{SiteFile, SiteFiles, SiteKind, Support, Why};
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
    /// Download each file of a torrent of several files on a torrent engine
    /// of its own, side by side, instead of the whole torrent on one.
    #[serde(default = "on")]
    pub torrent_files_apart: bool,
    /// When the app opens: start the downloads of a schedule whose start
    /// came while it was closed and whose stop time has not come yet.
    #[serde(default = "on")]
    pub start_scheduled_on_launch: bool,
    /// What happens once every download has finished (carried out by the app,
    /// not the engine).
    #[serde(default)]
    pub after_all: AfterAll,
    /// Seconds to wait, with a way to cancel, before it is done.
    #[serde(default = "one_minute")]
    pub after_countdown: u32,
    /// Done even when a download of the batch failed or was cancelled.
    #[serde(default)]
    pub after_failures: bool,
    /// Sorting finished files into folders by their kind (organize.rs).
    #[serde(default)]
    pub organize: Organize,
    /// Install yt-dlp and gallery-dl when they are missing, and keep the
    /// app's own copies up to date (deps.rs). Copies the user installed are
    /// never touched.
    #[serde(default = "on")]
    pub auto_update_tools: bool,
    /// Work out this kind of checksum for every file once it has
    /// downloaded, to see and compare; `None` = only for downloads given a
    /// checksum to check against.
    #[serde(default)]
    pub auto_checksum: Option<HashAlgo>,
}

/// What happens once every download has finished.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AfterAll {
    #[default]
    Nothing,
    Notify,
    /// Close the window, as its close button does (to the tray if that is on).
    Close,
    /// Quit the app.
    Exit,
    Sleep,
    Hibernate,
    Shutdown,
}

fn one_minute() -> u32 {
    60
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
            torrent_files_apart: true,
            start_scheduled_on_launch: true,
            after_all: AfterAll::Nothing,
            after_countdown: 60,
            after_failures: false,
            organize: Organize::default(),
            auto_update_tools: true,
            auto_checksum: None,
        }
    }

    fn normalized(mut self) -> Self {
        self.max_concurrent = self.max_concurrent.clamp(1, 10);
        self.connections = self.connections.clamp(1, MAX_CONNECTIONS);
        self.after_countdown = self.after_countdown.clamp(5, 600);
        self.organize = self.organize.normalized();
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
    /// This download's own speed limit, bytes per second; 0 or none = only
    /// the global limit applies.
    #[serde(default)]
    pub speed_limit: Option<u64>,
    /// Which connection it uses: the default, a direct one, or a saved proxy.
    #[serde(default)]
    pub proxy: ProxyChoice,
    /// The checksum the file should have, as pasted ("sha256:…", or the
    /// hex alone); it is checked once the file has downloaded.
    #[serde(default)]
    pub checksum: Option<String>,
    /// How its requests are made (headers, cookies, sign-in, timeouts),
    /// private values included; they are kept in the vault, not the list.
    #[serde(default)]
    pub request: Option<RequestOptions>,
    /// A saved request profile to use instead; neither = the default one.
    #[serde(default)]
    pub request_profile: Option<u64>,
    /// For an FTP or SFTP link: how to sign in (the password goes to the
    /// vault). A sign-in written into the link is used too.
    #[serde(default)]
    pub remote: Option<RemoteOptions>,
}

/// What testing a proxy found.
#[derive(Clone, Debug, Serialize)]
pub struct ProxyTest {
    /// A web server answered through the proxy.
    pub ok: bool,
    /// The address asked for.
    pub url: String,
    /// The HTTP status that web server answered with.
    pub status: Option<u16>,
    /// How long the answer took, in milliseconds.
    pub millis: u64,
    /// Why it failed, when it did.
    pub error: Option<String>,
}

/// Where a proxy is tested when no address is given: a page that answers
/// with an empty "204", made for checking connections.
pub const PROXY_TEST_URL: &str = "https://cp.cloudflare.com/generate_204";

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
    /// While what is already on disk is being checked before the download
    /// carries on: how many bytes of `total` have been checked.
    pub checking: Option<u64>,
    /// Files picked from a site link (a release, a shared folder), saved
    /// together in `path`, each listed with how far it is.
    pub bundle: bool,
    /// This download's own speed limit in bytes per second; 0 = none.
    pub speed_limit: u64,
    /// For a download inside a batch: the batch's id. It is listed in the
    /// batch's details, not on its own.
    pub parent: Option<u64>,
    /// For a batch (many links saved together in `path`): how its downloads
    /// stand. Status, sizes and speed are those of its downloads together.
    pub batch: Option<BatchInfo>,
    /// A download of a batch that was cancelled on its own; its status is
    /// `failed`, and it can be retried.
    pub cancelled: bool,
    /// When it ended (finished, failed or was cancelled), seconds since 1970.
    pub finished_at: Option<u64>,
    /// For a finished download: how long it took, in seconds. Only the time
    /// it was running counts, not time spent queued or paused.
    pub took: Option<u64>,
    /// Which connection it uses.
    pub proxy: ProxyChoice,
    /// Its file's checksum and how it compares, once one is wanted.
    pub checksum: Option<Checksum>,
    /// Its own request options (private values left out), if any.
    pub request: Option<RequestOptions>,
    /// The saved request profile it uses, if any.
    pub request_profile: Option<u64>,
    /// For an FTP or SFTP download: how it signs in (no password).
    pub remote: Option<RemoteOptions>,
    /// An SFTP server key that was refused, for the user to trust or not.
    pub server_key: Option<ServerKey>,
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
    /// How much of it is downloaded, while it is not finished, for the
    /// files of a torrent, which all download at once; `size` is then its
    /// full size.
    #[serde(default)]
    pub downloaded: Option<u64>,
    /// For a torrent's file on an engine of its own: its peers and speeds.
    #[serde(default)]
    pub net: Option<FileNet>,
}

/// One file of a torrent and how much of it is downloaded. Kept with the
/// download, so a paused or finished torrent still lists its files.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TorrentFileProgress {
    /// Where it goes inside the torrent's folder: folders, then its name.
    pub path: Vec<String>,
    pub len: u64,
    pub done: u64,
    /// Its own engine's peers and speeds, while it downloads on one (see
    /// torrent_split.rs).
    #[serde(default, skip_serializing)]
    pub net: Option<FileNet>,
}

/// How one file's own torrent engine is doing: whom it is connected to and
/// how fast data moves, to see where a torrent's speed goes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileNet {
    /// Peers it is connected to and exchanging with.
    pub peers: u32,
    /// Of those, the ones that have the whole torrent.
    pub seeders: u32,
    /// Peers it has heard of, connected or not.
    pub found: u32,
    /// Bytes per second coming in, and going out to other peers.
    pub down: u64,
    pub up: u64,
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
    /// The files of a torrent (only those chosen), with how far each is.
    #[serde(default)]
    pub files: Vec<TorrentFileProgress>,
    /// Files picked from a site link, downloaded together into the folder
    /// `filename`; their progress is kept in `files`.
    #[serde(default)]
    pub bundle: Vec<sites::SiteFile>,
    /// This download's own speed limit, bytes per second; 0 = none.
    #[serde(default)]
    pub speed_limit: u64,
    /// For a download inside a batch: the batch's id.
    #[serde(default)]
    pub parent: Option<u64>,
    /// For a batch: the links it was made from. It never runs itself; its
    /// downloads are the records whose `parent` is its id (batch.rs).
    #[serde(default)]
    pub batch: Option<batch::BatchMeta>,
    /// A download of a batch the user cancelled on its own.
    #[serde(default)]
    pub cancelled: bool,
    /// When it ended, in seconds since 1970.
    #[serde(default)]
    pub finished_at: Option<u64>,
    /// Seconds it has spent running, over all its runs; `None` for downloads
    /// saved before this was counted.
    #[serde(default)]
    pub active_secs: Option<u64>,
    /// Saved to the download folder with no folder of its own chosen: once
    /// finished, its file is sorted into the folder of its kind.
    #[serde(default)]
    pub sort: bool,
    /// Which connection it uses; downloads from before proxies existed use
    /// the default, which is a direct connection until one is chosen.
    #[serde(default)]
    pub proxy: ProxyChoice,
    /// Its file's checksum: the one to check against, and the outcome.
    #[serde(default)]
    pub checksum: Option<Checksum>,
    /// Its own request options; private values are in the vault under
    /// `request:<id>`.
    #[serde(default)]
    pub request: Option<RequestOptions>,
    #[serde(default)]
    pub request_profile: Option<u64>,
    /// For an FTP or SFTP download: how it signs in; the password is in the
    /// vault under `remote:<id>`.
    #[serde(default)]
    pub remote: Option<RemoteOptions>,
    /// The SFTP server key it last met and refused.
    #[serde(default)]
    pub server_key: Option<ServerKey>,
}

impl Record {
    /// A queued download with nothing decided yet beyond where it goes.
    fn new(id: u64, url: String, dir: PathBuf, filename: String, connections: usize) -> Record {
        Record {
            id,
            url,
            headers: Vec::new(),
            dir,
            filename,
            user_named: false,
            part_started: false,
            status: Status::Queued,
            error: None,
            total: None,
            resumable: false,
            etag: None,
            connections,
            created_at: now(),
            segments: Vec::new(),
            downloaded: 0,
            media: false,
            media_options: MediaOptions::default(),
            torrent: false,
            torrent_file: None,
            only_files: None,
            media_copy: None,
            named_stem: None,
            profile: false,
            items: 0,
            thumbnail: None,
            deleted_at: None,
            collection: None,
            files: Vec::new(),
            bundle: Vec::new(),
            speed_limit: 0,
            parent: None,
            batch: None,
            cancelled: false,
            finished_at: None,
            active_secs: Some(0),
            sort: false,
            proxy: ProxyChoice::Default,
            checksum: None,
            request: None,
            request_profile: None,
            remote: None,
            server_key: None,
        }
    }

    /// Forgets what was downloaded, after its unfinished data was deleted.
    fn reset_progress(&mut self) {
        self.part_started = false;
        self.segments.clear();
        self.downloaded = 0;
        self.items = 0;
        self.files.iter_mut().for_each(|f| f.done = 0);
        self.media_copy = None;
        self.named_stem = None;
        self.active_secs = Some(0);
    }

    /// How long a finished download took. Downloads saved before running
    /// time was counted are timed from when they were added.
    fn took(&self) -> Option<u64> {
        if self.status != Status::Completed {
            return None;
        }
        self.active_secs.or_else(|| self.finished_at.map(|f| f.saturating_sub(self.created_at)))
    }
}

struct Entry {
    rec: Record,
    live: Option<Arc<Live>>,
    speed: f64,
    last_bytes: u64,
    /// Network bytes counted at the last tick; each run counts from zero.
    last_received: u64,
}

impl Entry {
    fn new(rec: Record) -> Entry {
        Entry { rec, live: None, speed: 0.0, last_bytes: 0, last_received: 0 }
    }

    /// Copies progress from the running task into the record.
    fn sync(&mut self) {
        if let Some(live) = &self.live {
            self.rec.downloaded = live.downloaded.load(Relaxed);
            if self.rec.profile {
                self.rec.items = live.items.load(Relaxed);
            }
            if self.rec.torrent || !self.rec.bundle.is_empty() {
                let files = live.files.lock().unwrap();
                if !files.is_empty() {
                    self.rec.files = files.clone();
                }
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
            checking: self
                .live
                .as_ref()
                .filter(|l| l.checking.load(Relaxed))
                .map(|l| l.checked.load(Relaxed)),
            bundle: !r.bundle.is_empty(),
            speed_limit: r.speed_limit,
            parent: r.parent,
            batch: None,
            cancelled: r.cancelled,
            finished_at: r.finished_at,
            took: r.took(),
            proxy: r.proxy.clone(),
            checksum: r.checksum.clone(),
            request: r.request.clone(),
            request_profile: r.request_profile,
            remote: r.remote.clone(),
            server_key: r.server_key.clone(),
        }
    }
}

struct State {
    entries: Vec<Entry>,
    next_id: u64,
    settings: Settings,
    proxies: ProxyBook,
    requests: RequestBook,
}

#[derive(Serialize, Deserialize)]
struct Saved {
    next_id: u64,
    settings: Settings,
    tasks: Vec<Record>,
    #[serde(default)]
    proxies: ProxyBook,
    #[serde(default)]
    requests: RequestBook,
}

/// How a client reaches the web.
enum Route {
    /// As before proxies could be set in the app: through a proxy set for
    /// the whole system (HTTP_PROXY and the like), if there is one.
    System,
    /// Never through a proxy.
    Direct,
    Proxy(reqwest::Proxy),
}

/// What else a client is built with, for downloads with request options.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ClientOpts {
    /// Seconds without data before a connection gives up; none = 20.
    read_timeout: Option<u32>,
    /// Seconds to wait for an answer; none = 15.
    connect_timeout: Option<u32>,
    /// Redirects to another host are not followed, but handed back, so the
    /// download's own headers do not go along (see `prepare_request`).
    same_site_only: bool,
}

impl ClientOpts {
    pub(crate) fn of(request: Option<&RequestOptions>) -> ClientOpts {
        match request {
            Some(r) => ClientOpts { read_timeout: r.read_timeout, connect_timeout: r.connect_timeout, same_site_only: true },
            None => ClientOpts::default(),
        }
    }

    fn key(&self) -> String {
        format!("{:?}|{:?}|{}", self.read_timeout, self.connect_timeout, self.same_site_only)
    }
}

/// The engine's HTTP client: HTTP/1 only (HTTP/2 would carry every segment
/// over one TCP connection, which defeats parallel connections), site names
/// looked up the way Settings says.
fn build_client(dns: &Arc<dns::SecureDns>, route: Route) -> Result<reqwest::Client, String> {
    build_client_with(dns, route, ClientOpts::default())
}

fn build_client_with(dns: &Arc<dns::SecureDns>, route: Route, opts: ClientOpts) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder()
        .dns_resolver(Arc::new(dns::Resolver(dns.clone())))
        .user_agent(USER_AGENT)
        .default_headers(
            [(reqwest::header::ACCEPT, reqwest::header::HeaderValue::from_static("*/*"))]
                .into_iter()
                .collect(),
        )
        .http1_only()
        .connect_timeout(Duration::from_secs(opts.connect_timeout.unwrap_or(15).into()))
        .read_timeout(Duration::from_secs(opts.read_timeout.unwrap_or(20).into()));
    if opts.same_site_only {
        builder = builder.redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 10 {
                return attempt.error("too many redirects");
            }
            let from = attempt.previous().last().and_then(|u| u.host_str()).map(str::to_ascii_lowercase);
            let to = attempt.url().host_str().map(str::to_ascii_lowercase);
            if from == to {
                attempt.follow()
            } else {
                attempt.stop()
            }
        }));
    }
    builder = match route {
        Route::System => builder,
        Route::Direct => builder.no_proxy(),
        Route::Proxy(proxy) => builder.proxy(proxy),
    };
    builder.build().map_err(|e| format!("http client: {e}"))
}

pub(crate) struct Inner {
    /// The client used when no proxy is chosen in the app; see
    /// [`Inner::client_for`] for the one a download should use.
    system: reqwest::Client,
    /// The client for downloads set to a direct connection.
    direct: reqwest::Client,
    pub(crate) dns: Arc<dns::SecureDns>,
    /// Where proxy passwords are kept.
    secrets: Arc<dyn SecretStore>,
    /// A client per proxy in use, by its fingerprint (proxy.rs).
    routes: Mutex<std::collections::HashMap<String, reqwest::Client>>,
    /// A gateway per proxy the helper programs use, by its fingerprint.
    gateways: Mutex<std::collections::HashMap<String, gateway::Gateway>>,
    /// The helper programs the app installs and updates itself.
    deps: deps::Deps,
    /// Checksums being worked out, by download, each with its way to stop.
    hash_jobs: Mutex<std::collections::HashMap<u64, Arc<std::sync::atomic::AtomicBool>>>,
    /// How many files are read for their checksum at the same time.
    hash_slots: Arc<tokio::sync::Semaphore>,
    /// Private request values (vault.rs).
    vault: vault::Vault,
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
    /// Download schedules (schedule.rs), saved in `schedules.json`.
    schedules: Mutex<schedule::Book>,
    schedule_file: PathBuf,
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
        let secrets = Arc::new(Keyring::for_state_dir(state_dir.as_ref()));
        Self::with_secrets(state_dir, defaults, secrets)
    }

    /// As [`Engine::new`], with secrets (proxy passwords) kept in `secrets`
    /// rather than in the system's credential store.
    pub fn with_secrets(state_dir: impl AsRef<Path>, defaults: Settings, secrets: Arc<dyn SecretStore>) -> Result<Engine, String> {
        let state_dir = state_dir.as_ref();
        std::fs::create_dir_all(state_dir).map_err(|e| format!("state folder: {e}"))?;
        let state_file = state_dir.join("downloads.json");
        let schedule_file = state_dir.join("schedules.json");
        let book: schedule::Book = std::fs::read(&schedule_file)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();

        let saved: Option<Saved> = std::fs::read(&state_file)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok());
        let (entries, next_id, settings, proxies, requests) = match saved {
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
                            last_received: 0,
                        }
                    })
                    .collect();
                (entries, s.next_id, s.settings.normalized(), s.proxies.normalized(), s.requests.normalized())
            }
            None => (Vec::new(), 1, defaults.normalized(), ProxyBook::default().normalized(), RequestBook::default().normalized()),
        };

        let dns = dns::SecureDns::new(settings.secure_dns);
        let system = build_client(&dns, Route::System)?;
        let direct = build_client(&dns, Route::Direct)?;

        let inner = Arc::new(Inner {
            system,
            direct,
            dns,
            secrets: secrets.clone(),
            routes: Mutex::new(Default::default()),
            gateways: Mutex::new(Default::default()),
            deps: deps::Deps::new(state_dir),
            hash_jobs: Mutex::new(Default::default()),
            hash_slots: Arc::new(tokio::sync::Semaphore::new(2)),
            vault: vault::Vault::new(state_dir.join("secrets.vault"), secrets.clone()),
            limiter: Arc::new(Limiter::new(settings.speed_limit)),
            torrents: tokio::sync::OnceCell::new(),
            lists: Mutex::new(Default::default()),
            collections: Mutex::new(Default::default()),
            state: Mutex::new(State {
                entries,
                next_id,
                settings,
                proxies,
                requests,
            }),
            state_file,
            save_lock: Mutex::new(()),
            schedules: Mutex::new(book),
            schedule_file,
            version: watch::channel(0).0,
            rt: Handle::current(),
        });

        inner.rt.spawn(ticker(Arc::downgrade(&inner)));
        // Schedules first: a download whose window closed while the app was
        // closed is paused again before the queue starts it.
        inner.check_schedules(true);
        inner.rt.spawn(schedule_loop(Arc::downgrade(&inner)));
        inner.rt.spawn(tools_loop(Arc::downgrade(&inner)));
        // Checksums being worked out when the app closed start again.
        let unfinished: Vec<u64> = {
            let st = inner.state.lock().unwrap();
            st.entries
                .iter()
                .filter(|e| e.rec.checksum.as_ref().is_some_and(|c| c.state == CheckState::Calculating))
                .map(|e| e.rec.id)
                .collect()
        };
        for id in unfinished {
            inner.start_check(id);
        }
        inner.pump();
        Ok(Engine { inner })
    }

    /// Fires whenever anything observable changes.
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.inner.version.subscribe()
    }

    /// Every download, the ones inside a batch included (each names its
    /// batch in `parent`).
    pub fn list(&self) -> Vec<TaskInfo> {
        let mut st = self.inner.state.lock().unwrap();
        st.entries.iter_mut().for_each(Entry::sync);
        let mut sums = batch::roll_up(&mut st.entries, now());
        st.entries
            .iter()
            .map(|e| {
                let mut info = e.info();
                if e.rec.batch.is_some() {
                    let sum = sums.remove(&e.rec.id).unwrap_or_default();
                    info.speed = sum.speed as u64;
                    info.active_connections = sum.conns;
                    info.current = sum.info.current.clone().map(|name| CurrentFile {
                        id: String::new(),
                        name,
                        downloaded: 0,
                        total: None,
                        index: None,
                        count: None,
                    });
                    info.batch = Some(sum.info);
                }
                info
            })
            .collect()
    }

    /// Downloads many links together as one batch named `name`, saved in a
    /// folder of that name made now inside `dir` (the download folder when
    /// none). Each link goes through the same detection as a link pasted on
    /// its own (a file, a site video, a playlist...). A link that cannot be
    /// added is listed in the batch as failed, with why; the rest go ahead.
    pub fn add_batch(&self, name: &str, urls: Vec<String>, dir: Option<PathBuf>, media: MediaOptions) -> Result<u64, String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("Give the batch a name".into());
        }
        let base = probe::sanitize(name);
        let urls = batch::clean_links(urls);
        if urls.is_empty() {
            return Err("There are no web links to download".into());
        }
        // Asked before the list is locked: it may look at the list itself.
        let profiles: Vec<Option<ProfileRef>> = urls.iter().map(|u| self.inner.resolve(u)).collect();
        let id = {
            let mut st = self.inner.state.lock().unwrap();
            let dir = dir.unwrap_or_else(|| st.settings.download_dir.clone());
            // A folder of its own: never one already there or another download's.
            let folder = (0..10_000)
                .map(|n| if n == 0 { base.clone() } else { format!("{base} ({})", n + 1) })
                .find(|f| !dir.join(f).exists() && !st.entries.iter().any(|e| e.rec.dir == dir && &e.rec.filename == f))
                .ok_or("Could not find a free folder name")?;
            let path = dir.join(&folder);
            std::fs::create_dir_all(&path).map_err(|e| format!("Could not create the folder {}: {e}", path.display()))?;
            let pid = st.next_id;
            st.next_id += 1;
            let connections = st.settings.connections;
            let mut parent = Record::new(pid, String::new(), dir, folder, connections);
            parent.part_started = true;
            parent.media_options = media;
            parent.batch = Some(batch::BatchMeta { urls: urls.clone(), media });
            st.entries.push(Entry::new(parent));
            for (url, profile) in urls.into_iter().zip(profiles) {
                let new = NewDownload { url: url.clone(), dir: Some(path.clone()), media, ..Default::default() };
                let mut rec = match self.new_record(&mut st, new, profile) {
                    Ok(rec) => rec,
                    Err(why) => {
                        // Listed anyway, so the batch says which link and why.
                        let id = st.next_id;
                        st.next_id += 1;
                        let name = probe::sanitize(&probe::url_filename(&url).unwrap_or_else(|| url.clone()));
                        let mut rec = Record::new(id, url, path.clone(), name, connections);
                        rec.status = Status::Failed;
                        rec.error = Some(why);
                        rec.finished_at = Some(now());
                        rec
                    }
                };
                rec.parent = Some(pid);
                st.entries.push(Entry::new(rec));
            }
            pid
        };
        self.inner.changed();
        self.inner.pump();
        Ok(id)
    }

    /// Puts every failed (or cancelled) download of a batch back in the queue.
    pub fn retry_failed(&self, id: u64) {
        let ids: Vec<u64> = {
            let st = self.inner.state.lock().unwrap();
            st.entries
                .iter()
                .filter(|e| e.rec.parent == Some(id) && e.rec.status == Status::Failed)
                .map(|e| e.rec.id)
                .collect()
        };
        for child in ids {
            self.resume(child);
        }
    }

    /// Cancels one download of a batch: it stops, what it had downloaded is
    /// deleted, and it stays in the batch as cancelled, to be retried if
    /// wanted. Any other download is left alone (removing it cancels it).
    pub fn cancel(&self, id: u64) {
        {
            let mut st = self.inner.state.lock().unwrap();
            let Some(e) = st.entries.iter_mut().find(|e| e.rec.id == id && e.rec.parent.is_some()) else {
                return;
            };
            if e.rec.status == Status::Completed || e.rec.cancelled {
                return;
            }
            e.rec.cancelled = true;
            e.rec.status = Status::Failed;
            e.rec.error = Some("Cancelled".into());
            e.rec.finished_at = Some(now());
            match &e.live {
                // The task deletes its data once it has stopped (finish).
                Some(live) => live.cancel.cancel(),
                None => {
                    delete_files(&e.rec, false);
                    e.rec.reset_progress();
                }
            }
        }
        self.inner.changed();
        self.inner.pump();
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

    /// Trusts the SFTP server key a download was refused for (shown in its
    /// details), and tries the download again.
    pub fn trust_server_key(&self, id: u64) -> Result<(), String> {
        let key = {
            let st = self.inner.state.lock().unwrap();
            let e = st.entries.iter().find(|e| e.rec.id == id).ok_or("That download is no longer in the list")?;
            e.rec.server_key.clone().ok_or("This download has no server key waiting to be trusted")?
        };
        let state_dir = self.inner.state_file.parent().map(Path::to_path_buf).unwrap_or_default();
        remote::trust(&state_dir, &key)?;
        self.inner.update(id, |r| r.server_key = None);
        self.resume(id);
        Ok(())
    }

    /// Checks a finished file against a checksum: `expected` as pasted (its
    /// kind told by its form or `algorithm`), or none to work out the file's
    /// own sum of `algorithm` (SHA-256 if none) to see. Runs in the
    /// background; the download's `checksum` follows it.
    pub fn verify_checksum(&self, id: u64, expected: Option<String>, algorithm: Option<HashAlgo>) -> Result<(), String> {
        let wanted = match expected.as_deref().map(str::trim).filter(|e| !e.is_empty()) {
            Some(text) => {
                let (algorithm, hex) = checksum::parse_expected(text, algorithm)?;
                Checksum::wanted(algorithm, Some(hex))
            }
            None => Checksum::wanted(algorithm.unwrap_or_default(), None),
        };
        {
            let mut st = self.inner.state.lock().unwrap();
            let e = st.entries.iter_mut().find(|e| e.rec.id == id).ok_or("That download is no longer in the list")?;
            if !checkable(&e.rec) {
                return Err("Only a single file can be checked; a folder of files cannot".into());
            }
            if e.rec.status != Status::Completed {
                return Err("The file can be checked once it has finished downloading".into());
            }
            e.rec.checksum = Some(wanted);
        }
        self.inner.start_check(id);
        Ok(())
    }

    /// Checks a batch's files (or one download's file) against a sums file
    /// ("<hex>  <name>" lines, as SHA256SUMS and the like are written), by
    /// file name. Returns how many files had a sum in it.
    pub fn verify_with_sums(&self, id: u64, sums: &str) -> Result<usize, String> {
        let wanted: Vec<(u64, Checksum)> = {
            let st = self.inner.state.lock().unwrap();
            st.entries
                .iter()
                .filter(|e| e.rec.id == id || e.rec.parent == Some(id))
                .filter(|e| checkable(&e.rec) && e.rec.status == Status::Completed)
                .filter_map(|e| {
                    let hex = checksum::sum_for(sums, &e.rec.filename)?;
                    let (algorithm, hex) = checksum::parse_expected(&hex, None).ok()?;
                    Some((e.rec.id, Checksum::wanted(algorithm, Some(hex))))
                })
                .collect()
        };
        if wanted.is_empty() {
            return Err("The sums do not name any finished file of this download".into());
        }
        {
            let mut st = self.inner.state.lock().unwrap();
            for (child, c) in &wanted {
                if let Some(e) = st.entries.iter_mut().find(|e| e.rec.id == *child) {
                    e.rec.checksum = Some(c.clone());
                }
            }
        }
        for (child, _) in &wanted {
            self.inner.start_check(*child);
        }
        Ok(wanted.len())
    }

    /// The helper programs the app can install and update (yt-dlp,
    /// gallery-dl): which copy is in use, its version, the newest release
    /// seen, and what is being done about it now.
    pub fn tools_info(&self) -> Vec<ToolInfo> {
        MANAGED_TOOLS.iter().map(|name| self.inner.deps.info(name)).collect()
    }

    /// Goes up whenever [`Engine::tools_info`] may say something new.
    pub fn tools_revision(&self) -> u64 {
        self.inner.deps.revision()
    }

    /// Looks for new releases of every tool now, whatever the time of the
    /// last check. A tool whose check fails says why in its `error`.
    pub async fn check_tools(&self) -> Vec<ToolInfo> {
        for name in MANAGED_TOOLS {
            let _ = self.inner.check_tool(name).await;
        }
        self.tools_info()
    }

    /// Installs a tool's latest release, or updates the app's copy to it,
    /// checked against the project's published sums first. A failure leaves
    /// the copy in use as it was.
    pub async fn install_tool(&self, name: &str) -> Result<ToolInfo, String> {
        self.inner.install_tool(name).await
    }

    /// Goes back to the version of a tool used before its last update.
    pub fn rollback_tool(&self, name: &str) -> Result<ToolInfo, String> {
        self.inner.rollback_tool(name)
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
        let client = self.inner.client().ok()?;
        let asked = probe::probe(&client, url, headers);
        let found = tokio::time::timeout(Duration::from_secs(10), asked).await.ok()?.ok()?;
        found.total.filter(|size| *size > 0 && !found.is_html)
    }

    /// A video page's title, picture and resolutions, before downloading.
    pub async fn media_info(&self, url: &str) -> Result<MediaInfo, String> {
        media::info(&self.inner, url).await
    }

    /// What the app does with a link of a site it knows: lists its files
    /// for the user to pick from, or says why it cannot. `None` for any
    /// other link. Read from the link alone.
    pub fn site_kind(url: &str) -> Option<SiteKind> {
        sites::kind(url)
    }

    /// The files a share link or project page holds (a release's assets, a
    /// shared folder, a data set), with names and sizes, to pick from.
    pub async fn site_files(&self, url: &str) -> Result<SiteFiles, String> {
        sites::resolve(&self.inner, url).await
    }

    /// Downloads files picked from a site link. One plain file becomes a
    /// usual download; several (or any that needs the site's help each
    /// time) download together into a folder named `title`, listed file by
    /// file in the download's details.
    pub fn add_files(&self, page: &str, title: &str, files: Vec<SiteFile>, dir: Option<PathBuf>) -> Result<u64, String> {
        if files.is_empty() {
            return Err("Choose at least one file".into());
        }
        if let [one] = files.as_slice() {
            if one.late.is_none() && one.path.len() == 1 {
                return self.add(NewDownload {
                    url: one.url.clone(),
                    dir,
                    filename: one.path.last().cloned(),
                    headers: one.headers.clone(),
                    ..Default::default()
                });
            }
        }
        let id = {
            let mut st = self.inner.state.lock().unwrap();
            let dir = dir.unwrap_or_else(|| st.settings.download_dir.clone());
            let base = probe::sanitize(title.trim());
            let base = if base.is_empty() { "download".to_string() } else { base };
            // A folder of its own: not one already there or another download's.
            let folder = (0..10_000)
                .map(|n| probe::candidate(&base, n))
                .find(|name| {
                    !dir.join(name).exists() && !st.entries.iter().any(|e| e.rec.dir == dir && &e.rec.filename == name)
                })
                .unwrap_or(base);
            let id = st.next_id;
            st.next_id += 1;
            let mut rec = Record::new(id, page.trim().to_string(), dir, folder, st.settings.connections);
            rec.total = files.iter().map(|f| f.size).sum::<Option<u64>>();
            rec.resumable = true;
            rec.files = files
                .iter()
                .map(|f| TorrentFileProgress { path: f.path.clone(), len: f.size.unwrap_or(0), done: 0, net: None })
                .collect();
            rec.bundle = files;
            st.entries.push(Entry::new(rec));
            id
        };
        self.inner.changed();
        self.inner.pump();
        Ok(id)
    }

    /// Deep scan: lists the media links found on a web page.
    pub async fn scan_page(&self, url: &str) -> Result<Vec<Found>, String> {
        scan::scan(&self.inner.client()?, url).await
    }

    pub fn settings(&self) -> Settings {
        self.inner.state.lock().unwrap().settings.clone()
    }

    pub fn set_settings(&self, settings: Settings) {
        let settings = settings.normalized();
        self.inner.limiter.set_rate(settings.speed_limit);
        // running torrents take a new global limit at once too
        for e in self.inner.state.lock().unwrap().entries.iter() {
            if let Some(live) = &e.live {
                torrent::retune(live);
            }
        }
        self.inner.dns.set(settings.secure_dns);
        self.inner.state.lock().unwrap().settings = settings;
        self.inner.changed();
        self.inner.pump();
    }

    /// The saved proxies and which one is the default.
    pub fn proxies(&self) -> ProxyBook {
        self.inner.state.lock().unwrap().proxies.clone()
    }

    /// Saves a new proxy (`profile.id` 0) or changes one; its password is
    /// kept in the system's credential store. Returns its id. Downloads that
    /// start from now on use the change; running ones keep their connection
    /// until they are paused and resumed.
    pub fn save_proxy(&self, mut profile: ProxyProfile, password: PasswordChange) -> Result<u64, String> {
        let existing = {
            let st = self.inner.state.lock().unwrap();
            match profile.id {
                0 => None,
                id => Some(st.proxies.get(id).cloned().ok_or("That proxy has been deleted")?),
            }
        };
        profile.has_password = match &password {
            PasswordChange::Set(p) => !p.is_empty(),
            PasswordChange::Clear => false,
            PasswordChange::Keep => existing.as_ref().is_some_and(|p| p.has_password),
        };
        let mut profile = profile.checked()?;
        let id = {
            let mut st = self.inner.state.lock().unwrap();
            if profile.id == 0 {
                profile.id = st.proxies.next_id.max(1);
                st.proxies.next_id = profile.id + 1;
            }
            profile.id
        };
        let key = ProxyProfile::secret_key(id);
        match &password {
            PasswordChange::Set(p) if !p.is_empty() => self.inner.secrets.set(&key, p)?,
            PasswordChange::Set(_) | PasswordChange::Clear => self.inner.secrets.delete(&key)?,
            PasswordChange::Keep => {}
        }
        {
            let mut st = self.inner.state.lock().unwrap();
            match st.proxies.items.iter_mut().find(|p| p.id == id) {
                Some(slot) => *slot = profile,
                None => st.proxies.items.push(profile),
            }
        }
        self.inner.routes.lock().unwrap().clear();
        tracing::info!(id, "proxy saved");
        self.inner.changed();
        Ok(id)
    }

    /// Deletes a proxy and its password. Refused while an unfinished
    /// download is set to use it, so none of them silently connects some
    /// other way. Downloads on the default connection connect directly from
    /// then on, when it was the default.
    pub fn delete_proxy(&self, id: u64) -> Result<(), String> {
        {
            let st = self.inner.state.lock().unwrap();
            let users = st
                .entries
                .iter()
                .filter(|e| e.rec.proxy == ProxyChoice::Profile(id) && e.rec.status != Status::Completed)
                .count();
            if users > 0 {
                return Err(match users {
                    1 => "A download that has not finished uses this proxy. Remove it or let it finish first".to_string(),
                    n => format!("{n} downloads that have not finished use this proxy. Remove them or let them finish first"),
                });
            }
        }
        self.inner.secrets.delete(&ProxyProfile::secret_key(id))?;
        {
            let mut st = self.inner.state.lock().unwrap();
            st.proxies.items.retain(|p| p.id != id);
            if st.proxies.default == Some(id) {
                st.proxies.default = None;
            }
        }
        self.inner.routes.lock().unwrap().clear();
        tracing::info!(id, "proxy deleted");
        self.inner.changed();
        Ok(())
    }

    /// The proxy address a helper program (or the embedded browser) is
    /// given for a connection choice: a gateway on this computer to the
    /// chosen proxy, the secure-DNS relay, or `None` for no proxy at all.
    pub fn helper_proxy(&self, choice: &ProxyChoice) -> Result<Option<String>, String> {
        self.inner.helper_proxy(choice)
    }

    /// Makes a saved proxy the default connection, or `None` for direct.
    pub fn set_default_proxy(&self, id: Option<u64>) -> Result<(), String> {
        {
            let mut st = self.inner.state.lock().unwrap();
            if id.is_some_and(|id| st.proxies.get(id).is_none()) {
                return Err("That proxy has been deleted".into());
            }
            st.proxies.default = id;
        }
        self.inner.changed();
        Ok(())
    }

    /// Changes which connection a download uses. It takes effect the next
    /// time the download starts; a running one is restarted with it and
    /// carries on where it was.
    pub fn set_download_proxy(&self, id: u64, choice: ProxyChoice) -> Result<(), String> {
        {
            let mut st = self.inner.state.lock().unwrap();
            if let ProxyChoice::Profile(p) = choice {
                if st.proxies.get(p).is_none() {
                    return Err("That proxy has been deleted".into());
                }
            }
            let ids = expand(&st, &[id]);
            for e in st.entries.iter_mut().filter(|e| ids.contains(&e.rec.id) || e.rec.id == id) {
                if e.rec.proxy == choice {
                    continue;
                }
                e.rec.proxy = choice.clone();
                if let Some(live) = &e.live {
                    if matches!(e.rec.status, Status::Connecting | Status::Downloading) {
                        // back in the queue as it stops, so it starts again at once
                        e.rec.status = Status::Queued;
                        live.cancel.cancel();
                    }
                }
            }
        }
        self.inner.changed();
        self.inner.pump();
        Ok(())
    }

    /// Tries a proxy as it is filled in (saved or not): asks `url` (a page
    /// made for checking connections when none is given) through it, always
    /// through the proxy, whatever its bypass list says. `password` Keep
    /// uses the one saved for `profile.id`.
    pub async fn test_proxy(&self, profile: ProxyProfile, password: PasswordChange, url: Option<String>) -> ProxyTest {
        let url = url.map(|u| u.trim().to_string()).filter(|u| !u.is_empty()).unwrap_or_else(|| PROXY_TEST_URL.to_string());
        let failed = |error: String| ProxyTest { ok: false, url: url.clone(), status: None, millis: 0, error: Some(error) };
        if reqwest::Url::parse(&url).map_or(true, |u| !matches!(u.scheme(), "http" | "https")) {
            return failed("Enter a web address (http or https) to test with".into());
        }
        let secret = match password {
            PasswordChange::Set(p) => Some(p).filter(|p| !p.is_empty()),
            PasswordChange::Clear => None,
            PasswordChange::Keep if profile.id != 0 => match self.inner.secrets.get(&ProxyProfile::secret_key(profile.id)) {
                Ok(p) => p,
                Err(e) => return failed(e),
            },
            PasswordChange::Keep => None,
        };
        let mut profile = profile;
        profile.has_password = secret.is_some();
        let profile = match profile.checked() {
            Ok(p) => p,
            Err(e) => return failed(e),
        };
        let mut up = proxy::Upstream::new(&profile, secret);
        up.bypass = Arc::new(proxy::Bypass::default());
        let client = match up.reqwest_proxy().and_then(|p| build_client(&self.inner.dns, Route::Proxy(p))) {
            Ok(c) => c,
            Err(e) => return failed(e),
        };
        let started = std::time::Instant::now();
        let asked = client.get(&url).timeout(Duration::from_secs(20)).send().await;
        let millis = started.elapsed().as_millis() as u64;
        tracing::info!(proxy = ?up, ok = asked.is_ok(), millis, "proxy tested");
        match asked {
            Ok(resp) => ProxyTest { ok: true, url, status: Some(resp.status().as_u16()), millis, error: None },
            Err(e) => ProxyTest { millis, ..failed(proxy::error_text(&e)) },
        }
    }

    pub fn add(&self, new: NewDownload) -> Result<u64, String> {
        let profile = self.profile_for(&new);
        let id = {
            let mut st = self.inner.state.lock().unwrap();
            let rec = self.new_record(&mut st, new, profile)?;
            let id = rec.id;
            st.entries.push(Entry::new(rec));
            id
        };
        self.inner.changed();
        self.inner.pump();
        Ok(id)
    }

    /// The profile a new download's link points at, which makes it a
    /// download of everything on it. Asked before the list is locked.
    fn profile_for(&self, new: &NewDownload) -> Option<ProfileRef> {
        let url = torrent::clean(&new.url);
        let is_torrent = torrent::is_local_torrent(&url) || torrent::is_torrent_link(&url);
        if is_torrent || new.via_site {
            None
        } else {
            self.inner.resolve(&url)
        }
    }

    /// The record of a new download, with the next id; not yet in the list.
    fn new_record(&self, st: &mut State, new: NewDownload, profile: Option<ProfileRef>) -> Result<Record, String> {
        if let ProxyChoice::Profile(proxy) = new.proxy {
            if st.proxies.get(proxy).is_none() {
                return Err("That proxy has been deleted; choose another connection".into());
            }
        }
        if let Some(profile) = new.request_profile {
            if st.requests.get(profile).is_none() {
                return Err("That request profile has been deleted; choose another".into());
            }
        }
        let request = match new.request.clone().filter(RequestOptions::is_set) {
            Some(options) => Some(options.checked()?),
            None => None,
        };
        let checksum = match new.checksum.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
            Some(text) => {
                let (algorithm, hex) = checksum::parse_expected(text, None)?;
                Some(Checksum::wanted(algorithm, Some(hex)))
            }
            None => None,
        };
        let mut url = torrent::clean(&new.url);
        // An FTP or SFTP link: a sign-in written into it comes out of it.
        let mut remote_options = None;
        if remote::proto_of(&url).is_some() {
            let (bare, user, password) = remote::split_credentials(&url)?;
            let mut options = new.remote.clone().unwrap_or_default();
            if options.username.trim().is_empty() {
                options.username = user.unwrap_or_default();
            }
            if options.password.as_deref().unwrap_or_default().is_empty() {
                options.password = password;
            }
            remote_options = Some(options.checked()?);
            url = bare;
        }
        let local_torrent = torrent::is_local_torrent(&url);
        let is_torrent = local_torrent || torrent::is_torrent_link(&url);
        let default_name = if local_torrent {
            torrent::local_name(&url)
        } else {
            let parsed =
                reqwest::Url::parse(&url).map_err(|_| "That is not a valid link".to_string())?;
            if !is_torrent && remote_options.is_none() && !matches!(parsed.scheme(), "http" | "https") {
                return Err("Only http, https, ftp, ftps, ftpes, sftp and magnet links, and .torrent files, are supported".into());
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
        let profile = if is_torrent || new.via_site { None } else { profile };
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
        // Only a single file put in the download folder is sorted by kind.
        let sort = new.dir.is_none() && !is_torrent && profile.is_none();
        let dir = new.dir.unwrap_or_else(|| st.settings.download_dir.clone());
        let connections = new.connections.unwrap_or(st.settings.connections).clamp(1, MAX_CONNECTIONS);
        let mut rec = Record::new(id, url, dir, user_name.clone().unwrap_or(default_name), connections);
        rec.headers = new.headers;
        rec.user_named = user_name.is_some();
        rec.media = new.via_site && !is_torrent;
        rec.media_options = new.media;
        rec.torrent = is_torrent;
        rec.torrent_file = torrent_file;
        rec.only_files = only_files;
        rec.profile = profile.is_some();
        rec.collection = profile.filter(|p| p.by.is_some());
        rec.speed_limit = new.speed_limit.unwrap_or(0);
        rec.sort = sort;
        rec.proxy = new.proxy;
        rec.checksum = checksum;
        rec.request_profile = new.request_profile;
        if let Some(mut options) = remote_options {
            if let Some(password) = options.password.take().filter(|p| !p.is_empty()) {
                self.inner.vault.set(&format!("remote:{id}"), &password)?;
                options.has_password = true;
            }
            rec.remote = Some(options);
        }
        if let Some(mut options) = request {
            let private = options.split_private();
            if private != request::Private::default() {
                let text = serde_json::to_string(&private).map_err(|e| e.to_string())?;
                self.inner.vault.set(&format!("request:{id}"), &text)?;
            }
            rec.request = Some(options);
        }
        Ok(rec)
    }

    /// The downloads an action on `id` acts on: a batch's own downloads, or
    /// the download itself.
    fn targets(&self, id: u64) -> Vec<u64> {
        let st = self.inner.state.lock().unwrap();
        expand(&st, &[id])
    }

    /// Pauses a download; for a batch, every one of its downloads that is
    /// running or waiting.
    pub fn pause(&self, id: u64) {
        let ids = self.targets(id);
        {
            let mut st = self.inner.state.lock().unwrap();
            for e in st.entries.iter_mut().filter(|e| ids.contains(&e.rec.id)) {
                if !matches!(e.rec.status, Status::Queued | Status::Connecting | Status::Downloading) {
                    continue;
                }
                e.rec.status = Status::Paused;
                if let Some(live) = &e.live {
                    live.cancel.cancel();
                }
            }
        }
        self.inner.changed();
        self.inner.pump();
    }

    /// Resumes a paused download or retries a failed one. For a batch: its
    /// paused downloads, or when none is paused, its failed ones.
    pub fn resume(&self, id: u64) {
        let ids = self.targets(id);
        {
            let mut st = self.inner.state.lock().unwrap();
            let batch = st.entries.iter().any(|e| e.rec.id == id && e.rec.batch.is_some());
            let any_paused = st.entries.iter().any(|e| ids.contains(&e.rec.id) && e.rec.status == Status::Paused);
            for e in st.entries.iter_mut().filter(|e| ids.contains(&e.rec.id)) {
                let wanted = if batch && any_paused {
                    e.rec.status == Status::Paused
                } else {
                    matches!(e.rec.status, Status::Paused | Status::Failed)
                };
                if !wanted {
                    continue;
                }
                e.rec.status = Status::Queued;
                e.rec.error = None;
                e.rec.cancelled = false;
                e.rec.finished_at = None;
            }
        }
        self.inner.changed();
        self.inner.pump();
    }

    /// Sets a download's own speed limit, bytes per second (0: none; the
    /// global limit always applies too). A running HTTP download, file
    /// bundle or torrent's web seeds keep to it at once. A site video or a
    /// profile is fetched by a program given its rate as it starts, so a
    /// running one is restarted with the new rate; it carries on where it
    /// was. A torrent's peers take it the next time the torrent starts.
    pub fn set_speed_limit(&self, id: u64, rate: u64) {
        let mut ids = self.targets(id);
        if !ids.contains(&id) {
            // the batch keeps the limit too, to show it
            ids.push(id);
        }
        {
            let mut st = self.inner.state.lock().unwrap();
            for e in st.entries.iter_mut().filter(|e| ids.contains(&e.rec.id)) {
            if e.rec.speed_limit == rate {
                continue;
            }
            e.rec.speed_limit = rate;
            if let Some(live) = &e.live {
                live.limiter.set_rate(rate);
                // a torrent's engines take it now (torrent.rs)
                torrent::retune(live);
                let by_program = e.rec.media || e.rec.profile;
                if by_program && matches!(e.rec.status, Status::Connecting | Status::Downloading) {
                    // back in the queue as it stops, so it starts again at once
                    e.rec.status = Status::Queued;
                    live.cancel.cancel();
                }
            }
            }
        }
        self.inner.changed();
        self.inner.pump();
    }

    /// A download that can take a new link now: a direct file, not running.
    fn linkable(&self, id: u64) -> Result<Record, String> {
        let st = self.inner.state.lock().unwrap();
        let e = st.entries.iter().find(|e| e.rec.id == id).ok_or("That download is no longer in the list")?;
        if !refresh::takes_new_link(&e.rec) {
            return Err("Only a direct file download can take a new link".into());
        }
        if matches!(e.rec.status, Status::Queued | Status::Connecting | Status::Downloading) || e.live.is_some() {
            return Err("Pause the download first".into());
        }
        if e.rec.status == Status::Completed {
            return Err("That download has finished".into());
        }
        Ok(e.rec.clone())
    }

    /// What a new link for a download would do: carry on from what is on
    /// disk, or start over, and why (refresh.rs). Nothing is changed.
    /// `headers`: request headers for the new link; none keeps the old ones.
    pub async fn check_new_link(&self, id: u64, url: &str, headers: Option<Vec<(String, String)>>) -> Result<LinkCheck, String> {
        let rec = self.linkable(id)?;
        let headers = headers.unwrap_or_else(|| rec.headers.clone());
        refresh::check(&self.inner, &rec, url, &headers).await
    }

    /// Gives a download a new link. It carries on from what is on disk only
    /// when the new link is checked (again, now) to give the same file;
    /// otherwise this fails, unless `restart`, which starts it over.
    pub async fn use_new_link(
        &self,
        id: u64,
        url: &str,
        headers: Option<Vec<(String, String)>>,
        restart: bool,
    ) -> Result<LinkCheck, String> {
        let rec = self.linkable(id)?;
        let sent = headers.clone().unwrap_or_else(|| rec.headers.clone());
        let check = refresh::check(&self.inner, &rec, url, &sent).await?;
        if !check.resume && !restart {
            return Err("The new link does not give the same file".into());
        }
        {
            let mut st = self.inner.state.lock().unwrap();
            let e = st.entries.iter_mut().find(|e| e.rec.id == id).ok_or("That download is no longer in the list")?;
            if e.live.is_some() || matches!(e.rec.status, Status::Queued | Status::Connecting | Status::Downloading) {
                return Err("Pause the download first".into());
            }
            refresh::apply(&mut e.rec, url, headers, &check, restart);
        }
        self.inner.changed();
        self.inner.pump();
        Ok(check)
    }

    /// Moves a finished download (its file, or its folder) into `dir`. A
    /// file of the same name there is never written over: a number is added.
    /// Returns where it is now.
    pub async fn move_to(&self, id: u64, dir: PathBuf) -> Result<PathBuf, String> {
        let (src, batch) = {
            let st = self.inner.state.lock().unwrap();
            let e = st.entries.iter().find(|e| e.rec.id == id).ok_or("That download is no longer in the list")?;
            if e.rec.status != Status::Completed || e.live.is_some() {
                return Err("Only a finished download can be moved".into());
            }
            (e.rec.dir.join(&e.rec.filename), e.rec.batch.is_some())
        };
        if !src.exists() {
            return Err("Its file is no longer where it was saved".into());
        }
        let from = src.clone();
        let to = dir.clone();
        let moved = tokio::task::spawn_blocking(move || organize::move_into(&from, &to))
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| format!("Could not move it: {e}"))?;
        self.inner.relocated(id, &src, &moved, batch);
        Ok(moved)
    }

    /// Every schedule, with where it stands and when it next starts.
    pub fn schedules(&self) -> Vec<ScheduleInfo> {
        let (known, running): (std::collections::HashSet<u64>, std::collections::HashSet<u64>) = {
            let st = self.inner.state.lock().unwrap();
            (
                st.entries.iter().map(|e| e.rec.id).collect(),
                st.entries
                    .iter()
                    .filter(|e| matches!(e.rec.status, Status::Queued | Status::Connecting | Status::Downloading))
                    .map(|e| e.rec.id)
                    .collect(),
            )
        };
        let now = schedule::local_now();
        let book = self.inner.schedules.lock().unwrap();
        book.items
            .iter()
            .map(|s| {
                let mut info = s.info(now, s.spec.downloads.iter().any(|id| running.contains(id)));
                // downloads removed from the list since are left out
                info.spec.downloads.retain(|id| known.contains(id));
                info
            })
            .collect()
    }

    /// Makes a schedule (`id` None) or changes one. Its downloads are held
    /// paused until it starts them, unless it is already inside its window.
    pub fn save_schedule(&self, id: Option<u64>, mut spec: ScheduleSpec) -> Result<u64, String> {
        spec.name = spec.name.trim().to_string();
        {
            let st = self.inner.state.lock().unwrap();
            let mut seen = std::collections::HashSet::new();
            spec.downloads
                .retain(|id| seen.insert(*id) && st.entries.iter().any(|e| e.rec.id == *id));
        }
        schedule::check(&spec).map_err(String::from)?;
        let now = schedule::local_now();
        let id = {
            let mut book = self.inner.schedules.lock().unwrap();
            match id {
                Some(id) => {
                    let s = book.items.iter_mut().find(|s| s.id == id).ok_or("no such schedule")?;
                    s.spec = spec;
                    s.forget_past(now);
                    id
                }
                None => {
                    book.next_id = book.next_id.max(1);
                    let id = book.next_id;
                    book.next_id += 1;
                    book.items.push(schedule::Schedule::new(id, spec, now, now_secs()));
                    id
                }
            }
        };
        self.inner.hold(id);
        self.inner.save_schedules();
        self.inner.check_schedules(false);
        self.inner.notify();
        Ok(id)
    }

    /// Pauses a schedule (it starts nothing until resumed) or resumes it.
    /// A start that went by while it was paused is not caught up on.
    pub fn set_schedule_enabled(&self, id: u64, enabled: bool) {
        {
            let mut book = self.inner.schedules.lock().unwrap();
            let Some(s) = book.items.iter_mut().find(|s| s.id == id) else {
                return;
            };
            if s.enabled == enabled {
                return;
            }
            s.enabled = enabled;
            if enabled {
                s.forget_past(schedule::local_now());
            }
        }
        if enabled {
            self.inner.hold(id);
        }
        self.inner.save_schedules();
        self.inner.check_schedules(false);
        self.inner.notify();
    }

    /// Deletes a schedule. Its downloads stay as they are.
    pub fn delete_schedule(&self, id: u64) {
        self.inner.schedules.lock().unwrap().items.retain(|s| s.id != id);
        self.inner.save_schedules();
        self.inner.notify();
    }

    /// Removes a download from the list. Unfinished data is always deleted;
    /// a completed file is deleted only when `delete_file` is set.
    /// A batch goes with all its downloads, and its folder once empty.
    pub fn remove(&self, id: u64, delete_file: bool) {
        let mut ids = self.targets(id);
        if !ids.contains(&id) {
            ids.push(id);
        }
        // A checksum being worked out for it stops.
        {
            let mut jobs = self.inner.hash_jobs.lock().unwrap();
            for id in &ids {
                if let Some(stop) = jobs.remove(id) {
                    stop.store(true, Relaxed);
                }
            }
        }
        let mut folder = None;
        {
            let mut st = self.inner.state.lock().unwrap();
            for id in &ids {
                let Some(i) = st.entries.iter().position(|e| e.rec.id == *id) else {
                    continue;
                };
                self.inner.lists.lock().unwrap().remove(id);
                if let Some(live) = st.entries[i].live.clone() {
                    // The running task cleans up after itself when it stops.
                    live.removed.store(true, Relaxed);
                    live.cancel.cancel();
                    st.entries[i].rec.status = Status::Paused;
                } else {
                    let e = st.entries.remove(i);
                    if e.rec.batch.is_some() {
                        folder = Some(e.rec.dir.join(&e.rec.filename));
                    } else {
                        delete_files(&e.rec, delete_file);
                    }
                }
            }
        }
        // An empty batch folder goes too; one with files in it stays.
        if let Some(folder) = folder {
            let _ = std::fs::remove_dir(folder);
        }
        // Gone from the list, gone from the record of what it fetched, and
        // its private request values from the vault.
        for id in ids {
            let _ = std::fs::remove_file(self.inner.remembered_file(id));
            for key in [format!("request:{id}"), format!("remote:{id}")] {
                if let Err(e) = self.inner.vault.delete(&key) {
                    tracing::warn!(id, "could not remove the download's private values: {e}");
                }
            }
        }
        self.inner.changed();
        self.inner.pump();
    }
}

/// The downloads actions on `ids` act on: each batch stands for its own
/// downloads, anything else for itself.
fn expand(st: &State, ids: &[u64]) -> Vec<u64> {
    let mut out = Vec::new();
    for id in ids {
        if st.entries.iter().any(|e| e.rec.id == *id && e.rec.batch.is_some()) {
            out.extend(st.entries.iter().filter(|e| e.rec.parent == Some(*id)).map(|e| e.rec.id));
        } else {
            out.push(*id);
        }
    }
    out
}

fn delete_files(rec: &Record, delete_completed: bool) {
    if !rec.bundle.is_empty() {
        if rec.part_started {
            bundle::remove(rec, rec.status == Status::Completed && delete_completed);
        }
        return;
    }
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

fn now_secs() -> i64 {
    now() as i64
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

    /// The proxy a connection choice comes to, with its password, or `None`
    /// for no proxy chosen in the app.
    pub(crate) fn upstream(&self, choice: &ProxyChoice) -> Result<Option<proxy::Upstream>, String> {
        let profile = {
            let st = self.state.lock().unwrap();
            match st.proxies.pick(choice)? {
                Some(p) => p.clone(),
                None => return Ok(None),
            }
        };
        let password = if profile.has_password {
            let saved = self.secrets.get(&ProxyProfile::secret_key(profile.id))?;
            Some(saved.ok_or_else(|| {
                format!("The password of the proxy \"{}\" is missing. Enter it again under Settings, Network", profile.name)
            })?)
        } else {
            None
        };
        Ok(Some(proxy::Upstream::new(&profile, password)))
    }

    /// The HTTP client for a connection choice. Fails, rather than quietly
    /// connecting another way, when the chosen proxy cannot be used.
    pub(crate) fn client_for(&self, choice: &ProxyChoice) -> Result<reqwest::Client, String> {
        self.client_with(choice, ClientOpts::default())
    }

    /// As [`Inner::client_for`], built with a download's request options.
    pub(crate) fn client_with(&self, choice: &ProxyChoice, opts: ClientOpts) -> Result<reqwest::Client, String> {
        let up = match choice {
            ProxyChoice::Direct => None,
            other => self.upstream(other)?,
        };
        if opts == ClientOpts::default() {
            match (&up, choice) {
                (None, ProxyChoice::Direct) => return Ok(self.direct.clone()),
                (None, _) => return Ok(self.system.clone()),
                _ => {}
            }
        }
        let key = format!(
            "{}|{}",
            up.as_ref().map(|u| u.fingerprint()).unwrap_or_else(|| format!("none:{}", *choice == ProxyChoice::Direct)),
            opts.key()
        );
        if let Some(client) = self.routes.lock().unwrap().get(&key) {
            return Ok(client.clone());
        }
        let route = match up {
            Some(up) => Route::Proxy(up.reqwest_proxy()?),
            None if *choice == ProxyChoice::Direct => Route::Direct,
            None => Route::System,
        };
        let client = build_client_with(&self.dns, route, opts)?;
        self.routes.lock().unwrap().insert(key, client.clone());
        Ok(client)
    }

    /// The HTTP client for the app's own look-ups that belong to no
    /// download (a page's details, a scan): the default connection.
    pub(crate) fn client(&self) -> Result<reqwest::Client, String> {
        self.client_for(&ProxyChoice::Default)
    }

    /// What to give a helper program's `--proxy` for a connection choice:
    /// a gateway to the chosen proxy, the secure-DNS relay when no proxy is
    /// chosen and secure DNS is on, or nothing.
    pub(crate) fn helper_proxy(&self, choice: &ProxyChoice) -> Result<Option<String>, String> {
        let up = match choice {
            ProxyChoice::Direct => None,
            other => self.upstream(other)?,
        };
        let Some(up) = up else {
            let _runtime = self.rt.enter();
            return Ok(self.dns.proxy().map(|addr| format!("http://{addr}")));
        };
        let key = up.fingerprint();
        let mut gateways = self.gateways.lock().unwrap();
        if let Some(gate) = gateways.get(&key) {
            return Ok(Some(gate.proxy_arg()));
        }
        let _runtime = self.rt.enter();
        let gate = gateway::Gateway::start(up, self.dns.clone()).map_err(|e| format!("could not start the proxy gateway: {e}"))?;
        gateways.insert(key, gate.clone());
        Ok(Some(gate.proxy_arg()))
    }

    /// Sends a helper program through the connection a choice comes to. If
    /// the chosen proxy cannot be used, the program is pointed at an address
    /// where nothing answers, so it fails instead of connecting directly.
    pub(crate) fn add_proxy(&self, cmd: &mut tokio::process::Command, choice: &ProxyChoice) {
        match self.helper_proxy(choice) {
            Ok(Some(proxy)) => {
                cmd.arg("--proxy").arg(proxy);
            }
            Ok(None) => {}
            Err(why) => {
                tracing::warn!("a helper program could not be given its proxy: {why}");
                cmd.arg("--proxy").arg("http://127.0.0.1:9");
            }
        }
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
    /// `route`: the connection to go through (see [`Inner::add_proxy`]).
    pub(crate) fn add_sign_in(&self, cmd: &mut tokio::process::Command, scratch: Option<&Path>, route: &ProxyChoice) {
        // Through the chosen proxy, or with secure DNS on, through the local
        // relay that looks names up the same way the app does.
        self.add_proxy(cmd, route);
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

    fn save_schedules(&self) {
        let Ok(bytes) = serde_json::to_vec(&*self.schedules.lock().unwrap()) else {
            return;
        };
        let tmp = self.schedule_file.with_extension("json.tmp");
        if std::fs::write(&tmp, bytes).is_ok() {
            let _ = std::fs::rename(&tmp, &self.schedule_file);
        }
    }

    /// Runs every schedule's check at the time now, saves what changed and
    /// starts or pauses the downloads they name. `launch`: the first check
    /// after the app opened.
    fn check_schedules(self: &Arc<Self>, launch: bool) {
        let on_launch = self.state.lock().unwrap().settings.start_scheduled_on_launch;
        let now = schedule::local_now();
        let at = now_secs();
        let mut acts = Vec::new();
        let mut changed = false;
        {
            let mut book = self.schedules.lock().unwrap();
            for s in book.items.iter_mut() {
                let (a, c) = s.step(now, at, launch, on_launch);
                acts.extend(a);
                changed |= c;
            }
        }
        // Written down before anything is started, so nothing starts twice.
        if changed {
            self.save_schedules();
        }
        for act in &acts {
            match act {
                schedule::Act::Start(ids) => self.set_running(ids, true),
                schedule::Act::Stop(ids) => self.set_running(ids, false),
            }
        }
        if changed && acts.is_empty() {
            self.notify();
        }
    }

    /// Lets the downloads run (paused or failed ones go back in the queue)
    /// or pauses them.
    fn set_running(self: &Arc<Self>, ids: &[u64], run: bool) {
        let mut any = false;
        {
            let mut st = self.state.lock().unwrap();
            let ids = expand(&st, ids);
            for id in &ids {
                let Some(e) = st.entries.iter_mut().find(|e| e.rec.id == *id) else {
                    continue;
                };
                if run {
                    if matches!(e.rec.status, Status::Paused | Status::Failed) {
                        e.rec.status = Status::Queued;
                        e.rec.error = None;
                        e.rec.cancelled = false;
                        e.rec.finished_at = None;
                        any = true;
                    }
                } else if matches!(e.rec.status, Status::Queued | Status::Connecting | Status::Downloading) {
                    e.rec.status = Status::Paused;
                    if let Some(live) = &e.live {
                        live.cancel.cancel();
                    }
                    any = true;
                }
            }
        }
        if any {
            self.changed();
            self.pump();
        } else {
            self.notify();
        }
    }

    /// Holds a schedule's downloads paused until it starts them, unless it
    /// is paused itself or inside its window now.
    fn hold(self: &Arc<Self>, id: u64) {
        let ids = {
            let book = self.schedules.lock().unwrap();
            let Some(s) = book.items.iter().find(|s| s.id == id) else {
                return;
            };
            if !s.enabled || s.done || s.in_window(schedule::local_now()) {
                return;
            }
            s.spec.downloads.clone()
        };
        self.set_running(&ids, false);
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
        if let Some(files) = self.torrent_items(id) {
            return files;
        }
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
                        downloaded: None,
                        net: None,
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

    /// The files of a torrent download, each with how much of it is
    /// downloaded; `None` for any other download.
    fn torrent_items(&self, id: u64) -> Option<Vec<ItemInfo>> {
        let st = self.state.lock().unwrap();
        let e = st.entries.iter().find(|e| e.rec.id == id && (e.rec.torrent || !e.rec.bundle.is_empty()))?;
        let running = e.live.is_some();
        let files = match &e.live {
            Some(live) => {
                let files = live.files.lock().unwrap();
                if files.is_empty() { e.rec.files.clone() } else { files.clone() }
            }
            None => e.rec.files.clone(),
        };
        let folder = e.rec.dir.join(&e.rec.filename);
        Some(torrent_files_as_items(&files, &folder, running))
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
            st.entries.iter_mut().for_each(Entry::sync);
            batch::roll_up(&mut st.entries, now());
            Saved {
                next_id: st.next_id,
                settings: st.settings.clone(),
                proxies: st.proxies.clone(),
                requests: st.requests.clone(),
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
                // A batch never runs itself; its downloads do.
                if e.rec.status != Status::Queued || e.live.is_some() || e.rec.batch.is_some() {
                    continue;
                }
                e.rec.status = Status::Connecting;
                e.rec.error = None;
                let mut live = Live::new();
                live.limiter = Arc::new(Limiter::under(self.limiter.clone(), e.rec.speed_limit));
                let live = Arc::new(live);
                e.live = Some(live.clone());
                e.speed = 0.0;
                e.last_bytes = e.rec.downloaded;
                e.last_received = 0;
                running += 1;
                started = true;
                let inner = self.clone();
                let id = e.rec.id;
                self.rt.spawn(async move {
                    let res = task::run(&inner, id, &live).await;
                    let fetched = res.is_ok() && !live.removed.load(Relaxed);
                    if fetched {
                        inner.sort_finished(id).await;
                    }
                    inner.finish(id, &live, res);
                    // Its checksum, worked out now it is in its final place.
                    if fetched {
                        inner.start_check(id);
                    }
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
                let ran = live.started.elapsed().as_secs_f64().round() as u64;
                e.rec.active_secs = Some(e.rec.active_secs.unwrap_or(0) + ran);
                if live.removed.load(Relaxed) {
                    let e = st.entries.remove(i);
                    delete_files(&e.rec, false);
                    // The last download of a removed batch takes its empty folder along.
                    if let Some(parent) = e.rec.parent {
                        if !st.entries.iter().any(|x| x.rec.id == parent || x.rec.parent == Some(parent)) {
                            let _ = std::fs::remove_dir(&e.rec.dir);
                        }
                    }
                } else if e.rec.cancelled {
                    // Cancelled on its own inside a batch: what it had goes.
                    delete_files(&e.rec, false);
                    e.rec.reset_progress();
                } else {
                    match res {
                        Ok(()) => {
                            e.rec.status = Status::Completed;
                            e.rec.finished_at = Some(now());
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
                                e.rec.finished_at = Some(now());
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

    /// Once a download has fetched everything, and before it is shown as
    /// finished: its file goes to the folder of its kind, or its schedule's
    /// (sort_target). If that fails it stays where it is, still finished.
    async fn sort_finished(self: &Arc<Self>, id: u64) {
        let queue_folder = self.queue_folder(id);
        let target = {
            let st = self.state.lock().unwrap();
            let organize = &st.settings.organize;
            let base = &st.settings.download_dir;
            st.entries.iter().find(|e| e.rec.id == id).and_then(|e| sort_target(&e.rec, queue_folder.as_deref(), organize, base))
        };
        let Some((src, dir)) = target else { return };
        let from = src.clone();
        match tokio::task::spawn_blocking(move || organize::move_into(&from, &dir)).await {
            Ok(Ok(to)) => self.relocated(id, &src, &to, false),
            Ok(Err(e)) => media::debug(|| format!("could not sort {}: {e}", src.display())),
            Err(e) => media::debug(|| format!("could not sort {}: {e}", src.display())),
        }
    }

    /// The folder a schedule holding this download sends its files to.
    fn queue_folder(&self, id: u64) -> Option<String> {
        let book = self.schedules.lock().unwrap();
        book.items
            .iter()
            .filter(|s| s.spec.downloads.contains(&id))
            .find_map(|s| s.spec.folder.clone().filter(|f| !f.trim().is_empty()))
    }

    /// Writes down where a download's file or folder was moved to; for a
    /// batch, its downloads' places too.
    fn relocated(&self, id: u64, from: &Path, to: &Path, batch: bool) {
        {
            let mut st = self.state.lock().unwrap();
            if let Some(e) = st.entries.iter_mut().find(|e| e.rec.id == id) {
                if let (Some(dir), Some(name)) = (to.parent(), to.file_name()) {
                    e.rec.dir = dir.to_path_buf();
                    e.rec.filename = name.to_string_lossy().into_owned();
                    e.rec.deleted_at = None;
                }
            }
            if batch {
                for e in st.entries.iter_mut().filter(|e| e.rec.parent == Some(id)) {
                    if let Ok(rest) = e.rec.dir.strip_prefix(from) {
                        e.rec.dir = to.join(rest);
                    }
                }
            }
        }
        self.changed();
    }
}

/// Whether a download is one file whose checksum can be worked out: not a
/// torrent's, profile's or batch's folder, nor a set of files.
fn checkable(rec: &Record) -> bool {
    !rec.torrent && !rec.profile && rec.bundle.is_empty() && rec.batch.is_none()
}

impl Inner {
    /// Works out a finished download's checksum in the background, when it
    /// has one to check against or Settings ask for one for every file. A
    /// check already running for it stops and starts over.
    fn start_check(self: &Arc<Self>, id: u64) {
        let job = {
            let mut st = self.state.lock().unwrap();
            let auto = st.settings.auto_checksum;
            let Some(e) = st.entries.iter_mut().find(|e| e.rec.id == id) else { return };
            if e.rec.status != Status::Completed || !checkable(&e.rec) {
                return;
            }
            if e.rec.checksum.is_none() {
                match auto {
                    Some(algorithm) => e.rec.checksum = Some(Checksum::wanted(algorithm, None)),
                    None => return,
                }
            }
            let c = e.rec.checksum.as_mut().expect("set above");
            c.state = CheckState::Calculating;
            c.done = 0;
            c.actual = None;
            c.error = None;
            (e.rec.dir.join(&e.rec.filename), c.algorithm)
        };
        let (path, algorithm) = job;
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        if let Some(old) = self.hash_jobs.lock().unwrap().insert(id, stop.clone()) {
            old.store(true, Relaxed);
        }
        self.changed();
        let inner = self.clone();
        self.rt.spawn(async move {
            let Ok(_turn) = inner.hash_slots.clone().acquire_owned().await else { return };
            if stop.load(Relaxed) {
                return;
            }
            let worker = inner.clone();
            let flag = stop.clone();
            let file = path.clone();
            let outcome = tokio::task::spawn_blocking(move || {
                checksum::hash_file(&file, algorithm, &flag, |done| {
                    worker.update(id, |r| {
                        if let Some(c) = r.checksum.as_mut() {
                            c.done = done;
                        }
                    })
                })
            })
            .await
            .unwrap_or_else(|e| Err(format!("the check stopped: {e}")));
            if stop.load(Relaxed) {
                return;
            }
            inner.hash_jobs.lock().unwrap().remove(&id);
            {
                let mut st = inner.state.lock().unwrap();
                let Some(c) = st.entries.iter_mut().find(|e| e.rec.id == id).and_then(|e| e.rec.checksum.as_mut()) else { return };
                match outcome {
                    Ok(Some(actual)) => c.settle(actual),
                    Ok(None) => return,
                    Err(why) => {
                        c.state = CheckState::Error;
                        c.error = Some(why);
                    }
                }
                match c.state {
                    CheckState::Failed => tracing::warn!(id, file = %path.display(), "checksum does not match"),
                    CheckState::Error => tracing::warn!(id, file = %path.display(), error = ?c.error, "checksum could not be worked out"),
                    _ => tracing::info!(id, state = ?c.state, "checksum worked out"),
                }
            }
            inner.changed();
        });
    }
}

/// Where a download that just finished is moved to, if anywhere: the folder
/// of its schedule, else the folder of its kind of file when sorting is on
/// and it was put in the download folder with no folder of its own.
/// Profiles, playlists, torrents, file sets and batches keep their folders.
fn sort_target(rec: &Record, queue_folder: Option<&str>, organize: &Organize, base: &Path) -> Option<(PathBuf, PathBuf)> {
    let single_file = !rec.torrent && !rec.profile && rec.bundle.is_empty() && rec.batch.is_none() && rec.parent.is_none();
    if !single_file {
        return None;
    }
    let src = rec.dir.join(&rec.filename);
    let dir = match queue_folder {
        Some(folder) => organize::resolve(base, folder),
        None if rec.sort && organize.enabled && rec.dir == base => organize::resolve(base, organize.folder_for(&rec.filename)),
        None => return None,
    };
    (dir != rec.dir).then_some((src, dir))
}

/// Twice a second: refresh speeds and notify; every two seconds: persist.
/// Checks the schedules every few seconds. The time is read from the clock
/// at each check, so a computer that slept, or a clock or time zone that
/// changed, is noticed at the next one.
async fn schedule_loop(inner: Weak<Inner>) {
    let mut interval = tokio::time::interval(Duration::from_secs(5));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    interval.tick().await;
    loop {
        interval.tick().await;
        let Some(inner) = inner.upgrade() else { return };
        inner.check_schedules(false);
    }
}

/// Looks after the helper programs (deps.rs): shortly after the app opens,
/// then every hour; each is checked for a new release at most once a day.
/// RUSHDOWNLOAD_NO_TOOL_UPKEEP keeps tests from downloading anything.
async fn tools_loop(inner: Weak<Inner>) {
    if std::env::var_os("RUSHDOWNLOAD_NO_TOOL_UPKEEP").is_some() {
        return;
    }
    tokio::time::sleep(Duration::from_secs(15)).await;
    let mut interval = tokio::time::interval(Duration::from_secs(60 * 60));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        interval.tick().await;
        let Some(inner) = inner.upgrade() else { return };
        inner.tend_tools().await;
    }
}

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
                // Bytes as they arrive where the download counts them (a
                // torrent, whose progress only grows a whole piece at a
                // time); otherwise what has been written.
                let (bytes, last) = if live.counts_received.load(Relaxed) {
                    (live.received.load(Relaxed), &mut e.last_received)
                } else {
                    (live.downloaded.load(Relaxed), &mut e.last_bytes)
                };
                let inst = bytes.saturating_sub(*last) as f64 * 2.0;
                *last = bytes;
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

/// A torrent's files as items: finished ones as done; while the torrent
/// runs, one with some of it in as downloading; the rest as waiting.
fn torrent_files_as_items(files: &[TorrentFileProgress], folder: &Path, running: bool) -> Vec<ItemInfo> {
    files
        .iter()
        .map(|f| {
            let done = f.len > 0 && f.done >= f.len;
            let name = f.path.last().cloned().unwrap_or_default();
            ItemInfo {
                title: f.path.join("/"),
                state: if done {
                    ItemState::Done
                } else if running && f.done > 0 {
                    ItemState::Downloading
                } else {
                    ItemState::Waiting
                },
                size: Some(f.len),
                file: done.then(|| name.clone()),
                path: done.then(|| f.path.iter().fold(folder.to_path_buf(), |p, part| p.join(part))),
                thumbnail: None,
                duration: None,
                channel: None,
                url: None,
                downloaded: (!done).then_some(f.done.min(f.len)),
                net: if running && !done { f.net } else { None },
            }
        })
        .collect()
}

#[cfg(test)]
mod torrent_item_tests {
    use super::*;

    fn file(path: &[&str], len: u64, done: u64) -> TorrentFileProgress {
        TorrentFileProgress { path: path.iter().map(|p| p.to_string()).collect(), len, done, net: None }
    }

    #[test]
    fn each_file_shows_how_far_it_is() {
        let files = [file(&["a.mkv"], 100, 100), file(&["extras", "b.srt"], 50, 20), file(&["c.txt"], 10, 0)];
        let items = torrent_files_as_items(&files, Path::new("folder"), true);
        assert_eq!(items[0].state, ItemState::Done);
        assert_eq!(items[0].downloaded, None);
        assert_eq!(items[0].path, Some(PathBuf::from("folder").join("a.mkv")));
        assert_eq!(items[1].title, "extras/b.srt");
        assert_eq!(items[1].state, ItemState::Downloading);
        assert_eq!(items[1].downloaded, Some(20));
        assert_eq!(items[1].size, Some(50));
        // nothing of it in yet
        assert_eq!(items[2].state, ItemState::Waiting);
        let paused = torrent_files_as_items(&files, Path::new("folder"), false);
        assert_eq!(paused[1].state, ItemState::Waiting);
        assert_eq!(paused[0].state, ItemState::Done);
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
            downloaded: None,
            net: None,
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

#[cfg(test)]
mod sort_tests {
    use super::*;

    #[test]
    fn a_schedule_folder_wins_and_collections_keep_their_own_folders() {
        let base = PathBuf::from("D:/Downloads");
        let organize = Organize::default();
        let mut rec = Record::new(1, "https://a.test/x.mp4".into(), base.clone(), "x.mp4".into(), 8);
        rec.sort = true;
        let target = |rec: &Record, queue: Option<&str>| sort_target(rec, queue, &organize, &base).map(|(_, dir)| dir);
        assert_eq!(target(&rec, None), Some(base.join("Videos")));
        assert_eq!(target(&rec, Some("Night")), Some(base.join("Night")), "the schedule's folder wins");
        // a folder chosen for it: left alone, unless its schedule has a folder
        let mut chosen = rec.clone();
        chosen.sort = false;
        chosen.dir = PathBuf::from("E:/Films");
        assert_eq!(target(&chosen, None), None);
        assert_eq!(target(&chosen, Some("Night")), Some(base.join("Night")));
        // profiles, torrents, batches and their downloads keep their folders
        for change in [
            (|r: &mut Record| r.profile = true) as fn(&mut Record),
            |r| r.torrent = true,
            |r| r.parent = Some(9),
            |r| r.batch = Some(batch::BatchMeta::default()),
        ] {
            let mut r = rec.clone();
            change(&mut r);
            assert_eq!(target(&r, Some("Night")), None);
        }
        let mut off = Organize::default();
        off.enabled = false;
        assert_eq!(sort_target(&rec, None, &off, &base), None);
    }
}
