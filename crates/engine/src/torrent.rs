//! BitTorrent downloads: magnet links and links to `.torrent` files, through
//! librqbit. Each torrent gets a folder of its own inside the download
//! folder, so pausing, resuming and removing act on exactly that folder.

use std::net::Ipv6Addr;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering::Relaxed;
use std::sync::Arc;
use std::time::Duration;

use librqbit::limits::LimitsConfig;
use librqbit::{
    AddTorrent, AddTorrentOptions, AddTorrentResponse, DhtSessionConfig, ListenerMode, ListenerOptions, Session,
    SessionOptions, TorrentStatsState,
};
use percent_encoding::percent_decode_str;
use serde::Serialize;

use crate::probe::{candidate, sanitize};
use crate::task::{ConnGuard, Fail, Live, Seg};
use crate::webseed::{self, Plan, WebSeeds};
use crate::{Inner, Record, Status, TorrentFileProgress};

/// One file inside a torrent, as shown before the download starts.
#[derive(Clone, Debug, Serialize)]
pub struct TorrentFile {
    /// Position in the torrent; this is what a selection refers to.
    pub index: usize,
    /// Folders then file name.
    pub path: Vec<String>,
    pub size: u64,
}

/// What a torrent contains, read without downloading any of it.
#[derive(Clone, Debug, Serialize)]
pub struct TorrentPreview {
    pub name: String,
    pub total: u64,
    pub files: Vec<TorrentFile>,
}

/// A link or path as typed or pasted: trimmed, without surrounding quotes.
pub(crate) fn clean(source: &str) -> String {
    source.trim().trim_matches('"').trim().to_string()
}

/// A `.torrent` file on this computer.
pub(crate) fn is_local_torrent(source: &str) -> bool {
    let lower = source.to_ascii_lowercase();
    lower.ends_with(".torrent")
        && !lower.starts_with("http://")
        && !lower.starts_with("https://")
        && Path::new(source).is_file()
}

/// Name for a local `.torrent` file before its contents are read.
pub(crate) fn local_name(path: &str) -> String {
    let stem = Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    sanitize(if stem.is_empty() { "Torrent" } else { &stem })
}

/// Where the torrent comes from: bytes of a `.torrent` file, or a link.
/// The bytes are returned too, where there are any, for the web seeds they list.
/// `client`: the connection to fetch a `.torrent` link over.
pub(crate) async fn open(client: &reqwest::Client, source: &str, saved_copy: Option<&Path>) -> Result<(AddTorrent<'static>, Option<Vec<u8>>), Fail> {
    let file = match saved_copy {
        Some(copy) if copy.is_file() => Some(copy.to_path_buf()),
        _ if is_local_torrent(source) => Some(PathBuf::from(source)),
        _ => None,
    };
    if let Some(path) = file {
        let bytes = std::fs::read(&path).map_err(|e| Fail::Fatal(format!("could not read the torrent file: {e}")))?;
        return Ok((AddTorrent::from_bytes(bytes.clone()), Some(bytes)));
    }
    // A link to a .torrent file is fetched here, so its web seeds are known
    // (and it is looked up the way the app's other downloads are).
    if !source.to_ascii_lowercase().starts_with("magnet:") {
        if let Some(bytes) = fetch_torrent_file(client, source).await {
            return Ok((AddTorrent::from_bytes(bytes.clone()), Some(bytes)));
        }
    }
    Ok((AddTorrent::from_url(source.to_string()), None))
}

/// A .torrent file from the web, or `None` (the torrent library then tries).
async fn fetch_torrent_file(client: &reqwest::Client, url: &str) -> Option<Vec<u8>> {
    const MAX: usize = 32 << 20;
    let mut answer = client.get(url).send().await.ok()?.error_for_status().ok()?;
    let mut bytes = Vec::new();
    while let Some(chunk) = tokio::time::timeout(Duration::from_secs(30), answer.chunk()).await.ok()?.ok()? {
        bytes.extend_from_slice(&chunk);
        if bytes.len() > MAX {
            return None;
        }
    }
    (bytes.first() == Some(&b'd')).then_some(bytes)
}

/// Reads a torrent's name and file list without downloading its contents.
pub(crate) async fn preview(inner: &Arc<Inner>, source: &str) -> Result<TorrentPreview, String> {
    let source = clean(source);
    let fail = |f: Fail| match f {
        Fail::Fatal(m) | Fail::Retry(m) => m,
        Fail::Cancelled => "cancelled".to_string(),
    };
    // Through the default connection: a SOCKS5 proxy, or none.
    let proxy = torrent_proxy(inner, &crate::ProxyChoice::Default).map_err(fail)?;
    let (add, _) = open(&inner.client()?, &source, None).await.map_err(fail)?;
    let session = match &proxy {
        None => session(inner).await.map_err(fail)?,
        Some(url) => proxied_session(inner, url).await.map_err(fail)?,
    };
    let options = AddTorrentOptions {
        list_only: true,
        ..Default::default()
    };
    // A magnet link has to find peers before it can say what it contains.
    let listed = tokio::time::timeout(Duration::from_secs(90), session.add_torrent(add, Some(options)))
        .await
        .map_err(|_| {
            "No one sharing this torrent answered, so its file list could not be read. Try again later".to_string()
        })?
        .map_err(|e| format!("Could not read this torrent: {e:#}"))?;
    let AddTorrentResponse::ListOnly(listed) = listed else {
        return Err("Could not read this torrent".into());
    };
    let files: Vec<TorrentFile> = listed
        .info
        .iter_file_details()
        .enumerate()
        .map(|(index, f)| TorrentFile {
            index,
            path: f.filename.to_vec(),
            size: f.len,
        })
        .collect();
    let name = listed
        .info
        .name()
        .map(|n| n.to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| {
            if is_local_torrent(&source) {
                local_name(&source)
            } else {
                default_name(&source)
            }
        });
    Ok(TorrentPreview {
        name: sanitize(&name),
        total: files.iter().map(|f| f.size).sum(),
        files,
    })
}

/// Magnet links, and web links that point at a `.torrent` file.
pub(crate) fn is_torrent_link(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    if lower.starts_with("magnet:?") {
        return true;
    }
    (lower.starts_with("http://") || lower.starts_with("https://"))
        && lower
            .split(['?', '#'])
            .next()
            .is_some_and(|path| path.ends_with(".torrent"))
}

/// A readable name before the torrent's own details are known: the magnet
/// link's display name, or the `.torrent` file's name.
pub(crate) fn default_name(url: &str) -> String {
    let url = url.trim();
    let name = if url.to_ascii_lowercase().starts_with("magnet:?") {
        url[8..].split('&').find_map(|pair| {
            let (key, value) = pair.split_once('=')?;
            (key == "dn").then(|| {
                percent_decode_str(&value.replace('+', " "))
                    .decode_utf8_lossy()
                    .to_string()
            })
        })
    } else {
        crate::probe::url_filename(url).map(|n| {
            n.strip_suffix(".torrent")
                .or_else(|| n.strip_suffix(".TORRENT"))
                .unwrap_or(&n)
                .to_string()
        })
    };
    sanitize(name.as_deref().unwrap_or("Torrent"))
}

/// Deletes a torrent's own folder. Refuses anything that is not a plain
/// child of `dir`, so a bad name can never reach outside it.
pub(crate) fn remove_data(dir: &Path, folder: &str) {
    if folder.is_empty() || folder == "." || folder == ".." || folder.contains(['/', '\\']) {
        return;
    }
    let target = dir.join(folder);
    if target.parent() == Some(dir) && target.is_dir() {
        let _ = std::fs::remove_dir_all(&target);
    }
}

/// The port peers are asked to connect to. A fixed one, the way other
/// clients keep theirs, so a router's forwarding of it keeps working.
const LISTEN_PORT: u16 = 51381;

/// Peers kept per torrent; the library's own default is 128.
const PEER_LIMIT: usize = 200;

/// `listen`: the port to take connections from other peers on (0 for any
/// free one), or `None` to only connect out.
fn session_options(dht: bool, listen: Option<u16>) -> SessionOptions {
    SessionOptions {
        // Needed to find peers for magnet links. Nothing is kept between runs.
        // RUSHDOWNLOAD_NO_DHT turns it off (trackers only), for tests.
        dht: (dht && std::env::var_os("RUSHDOWNLOAD_NO_DHT").is_none()).then(|| DhtSessionConfig {
            bootstrap_addrs: None,
            port: None,
            persistence: None,
        }),
        // Peers that find this one connect to it, as they do to any other
        // client; a peer that can only be reached that way is otherwise
        // lost. The router is asked to pass the port on (UPnP).
        listen: listen.map(|port| ListenerOptions {
            mode: ListenerMode::TcpOnly,
            listen_addr: (Ipv6Addr::UNSPECIFIED, port).into(),
            enable_upnp_port_forwarding: true,
            ..Default::default()
        }),
        peer_limit: Some(PEER_LIMIT),
        persistence: None,
        fastresume: false,
        client_name_and_version: Some(concat!("RushDownload ", env!("CARGO_PKG_VERSION")).into()),
        ..Default::default()
    }
}

/// Starts a torrent engine, taking connections on the usual port, else on
/// any free one, else only connecting out. Tests (RUSHDOWNLOAD_NO_DHT) do not
/// listen, so Windows does not ask about the firewall.
async fn new_session(inner: &Arc<Inner>, dht: bool) -> Result<Arc<Session>, String> {
    let ports: &[Option<u16>] = if std::env::var_os("RUSHDOWNLOAD_NO_DHT").is_some() {
        &[None]
    } else {
        &[Some(LISTEN_PORT), Some(0), None]
    };
    let mut last = String::new();
    for listen in ports {
        match Session::new_with_opts(inner.download_dir(), session_options(dht, *listen)).await {
            Ok(s) => return Ok(s),
            Err(e) => last = format!("{e:#}"),
        }
    }
    Err(last)
}

/// An engine of its own for one file of a torrent downloaded file by file
/// (see torrent_split.rs): any free port, as the usual one belongs to the
/// app's main engine.
pub(crate) async fn worker_session(inner: &Arc<Inner>) -> Result<Arc<Session>, Fail> {
    let quiet = std::env::var_os("RUSHDOWNLOAD_NO_DHT").is_some();
    let ports: &[Option<u16>] = if quiet { &[None] } else { &[Some(0), None] };
    let mut last = String::new();
    for dht in [true, false] {
        for listen in ports {
            match Session::new_with_opts(inner.download_dir(), session_options(dht, *listen)).await {
                Ok(s) => return Ok(s),
                Err(e) => last = format!("{e:#}"),
            }
        }
    }
    Err(Fail::Fatal(format!("could not start the torrent engine: {last}")))
}

/// The SOCKS5 proxy a torrent goes through, `None` for a direct
/// connection, or why the chosen proxy cannot carry torrents.
fn torrent_proxy(inner: &Inner, choice: &crate::ProxyChoice) -> Result<Option<String>, Fail> {
    match inner.upstream(choice).map_err(Fail::Fatal)? {
        Some(up) => up.torrent_url().map(Some).map_err(Fail::Fatal),
        None => Ok(None),
    }
}

/// A torrent engine that reaches peers and HTTP trackers only through a
/// SOCKS5 proxy. Nothing that would go around the proxy runs: no DHT, no
/// local peer discovery, no listening port and no UPnP. (UDP trackers are
/// asked directly: the library cannot send them through a proxy.)
async fn proxied_session(inner: &Arc<Inner>, proxy_url: &str) -> Result<Arc<Session>, Fail> {
    let mut options = session_options(false, None);
    options.disable_local_service_discovery = true;
    options.connect = Some(librqbit::ConnectionOptions {
        proxy_url: Some(proxy_url.to_string()),
        ..Default::default()
    });
    Session::new_with_opts(inner.download_dir(), options)
        .await
        .map_err(|e| Fail::Fatal(format!("could not start the torrent engine with the proxy: {e:#}")))
}

async fn session(inner: &Arc<Inner>) -> Result<Arc<Session>, Fail> {
    inner
        .torrents
        .get_or_try_init(|| new_session(inner, true))
        .await
        .cloned()
        .map_err(|e| Fail::Fatal(format!("could not start the torrent engine: {e}")))
}

/// A torrent engine of its own for a second copy of a torrent that is already
/// running: one engine holds a torrent only once, so the copy gets a separate
/// one and the two never share anything.
async fn private_session(inner: &Arc<Inner>) -> Result<Arc<Session>, Fail> {
    match new_session(inner, true).await {
        Ok(s) => Ok(s),
        // If a second peer finder cannot start, trackers alone still work.
        Err(_) => new_session(inner, false)
            .await
            .map_err(|e| Fail::Fatal(format!("could not start the torrent engine: {e}"))),
    }
}

/// Sets a torrent engine's download limit to its share of `rate` (bytes per
/// second; 0 = none). The library reads it for every block it asks peers
/// for, so a change applies to a running torrent at once.
fn set_session_limit(session: &Session, rate: u64, share: u64) {
    let per = if rate == 0 { 0 } else { (rate / share.max(1)).max(1) };
    session.ratelimits.set_download_bps(NonZeroU32::new(u32::try_from(per).unwrap_or(u32::MAX)));
}

/// A torrent engine now working for this download: it takes the download's
/// limit (its own and the global one, the lower), and keeps to it as it
/// changes (`retune`).
pub(crate) fn hold(live: &Live, session: Arc<Session>, share: u64) {
    set_session_limit(&session, live.limiter.effective_rate(), share);
    live.torrent_sessions.lock().unwrap().push((session, share));
}

/// The download's limit changed (its own, or the global one): every torrent
/// engine working for it takes the new one now. Blocks already asked of
/// peers still arrive at the old speed; on a fast swarm the speed comes down
/// to the new limit over some seconds (about fifteen, measured), and holds
/// there. (Pausing the torrent to drop them stalls it: its peers come back
/// slowly.)
pub(crate) fn retune(live: &Live) {
    let rate = live.limiter.effective_rate();
    for (session, share) in live.torrent_sessions.lock().unwrap().iter() {
        set_session_limit(session, rate, *share);
    }
}

/// Bytes from a torrent's web seeds counted in the same budget as its peers'
/// (the engine's limit), so the two together keep to the download's limit.
/// The library takes at most a second's worth at a time.
pub(crate) async fn take(session: &Session, n: usize) {
    let Some(bps) = session.ratelimits.get_download_bps() else {
        return;
    };
    let mut left = n as u64;
    while left > 0 {
        let step = left.min(u64::from(bps.get()));
        if let Some(step) = NonZeroU32::new(step as u32) {
            let _ = session.ratelimits.prepare_for_download(step).await;
        }
        left -= step.max(1);
    }
}

/// Picks and creates this torrent's folder the first time it runs.
pub(crate) fn claim_folder(dir: &Path, base: &str) -> Result<String, Fail> {
    for n in 0..10_000 {
        let name = candidate(base, n);
        match std::fs::create_dir(dir.join(&name)) {
            Ok(()) => return Ok(name),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(Fail::Fatal(format!("could not create folder: {e}"))),
        }
    }
    Err(Fail::Fatal("could not find a free folder name".into()))
}

pub(crate) async fn run(
    inner: &Arc<Inner>,
    id: u64,
    live: &Arc<Live>,
    rec: &Record,
) -> Result<(), Fail> {
    std::fs::create_dir_all(&rec.dir)
        .map_err(|e| Fail::Fatal(format!("could not create folder: {e}")))?;

    // Resuming reuses the folder; the pieces already on disk are checked and kept.
    let (folder_name, folder_is_new) = if rec.part_started && rec.dir.join(&rec.filename).is_dir() {
        (rec.filename.clone(), false)
    } else {
        (claim_folder(&rec.dir, &rec.filename)?, true)
    };
    let folder: PathBuf = rec.dir.join(&folder_name);
    inner.update(id, |r| {
        r.torrent = true;
        r.resumable = true;
        r.part_started = true;
        r.filename = folder_name.clone();
        if folder_is_new {
            r.segments.clear();
        }
    });

    // What was downloaded before stays shown until the check of the folder
    // says otherwise (a new folder has nothing in it).
    // The amount itself may already read 0 by the time this runs, so it is
    // also worked out from the pieces each part had, which are kept.
    let had = if folder_is_new {
        0
    } else {
        let (done, all) = rec.segments.iter().fold((0u64, 0u64), |(d, a), s| (d + (s[1] - s[0]), a + (s[2] - s[0])));
        let from_pieces = match rec.total {
            Some(total) if all > 0 => (total as u128 * done as u128 / all as u128) as u64,
            _ => 0,
        };
        rec.downloaded.max(from_pieces)
    };
    // The speed follows bytes received from the start, so putting back the
    // amount it had is not taken for a burst of speed.
    live.counts_received.store(true, Relaxed);
    live.downloaded.store(had, Relaxed);

    // Through a SOCKS5 proxy when one is chosen for it; then always on one
    // engine of its own, set up so nothing goes around the proxy.
    let proxy = torrent_proxy(inner, &rec.proxy)?;
    let client = live.client();
    let own_session = || async {
        match &proxy {
            None => private_session(inner).await,
            Some(url) => proxied_session(inner, url).await,
        }
    };

    // A torrent of several files can download each file on its own engine,
    // side by side (Settings; see torrent_split.rs).
    if proxy.is_none() && inner.state.lock().unwrap().settings.torrent_files_apart {
        let shared = session(inner).await?;
        let listed = tokio::select! {
            _ = live.cancel.cancelled() => return Err(Fail::Cancelled),
            l = crate::torrent_split::list(&client, &shared, rec) => l,
        };
        if let Some(listed) = listed {
            return crate::torrent_split::run(inner, id, live, rec, &folder, &folder_name, listed, had).await;
        }
    }

    // Each torrent download runs in a torrent engine of its own: the library
    // can change an engine's speed limit while it runs, but not a single
    // torrent's, so this is how a new limit reaches the peers at once.
    let mut session = own_session().await?;
    let private = true;
    hold(live, session.clone(), 1);
    let options = || AddTorrentOptions {
        overwrite: true,
        // `None` downloads everything; otherwise only the chosen files.
        only_files: rec.only_files.clone(),
        output_folder: Some(folder.to_string_lossy().into_owned()),
        // the engine's own limit applies (see `hold`)
        ratelimits: LimitsConfig::default(),
        ..Default::default()
    };

    // For a magnet link this first has to find peers and fetch the torrent's
    // details from them, which can take a while.
    let (source, torrent_bytes) = open(&client, &rec.url, rec.torrent_file.as_deref()).await?;
    let mut added = tokio::select! {
        _ = live.cancel.cancelled() => return Err(Fail::Cancelled),
        r = session.add_torrent(source, Some(options())) => r,
    };
    if matches!(added, Ok(AddTorrentResponse::AlreadyManaged(..))) {
        // The same torrent is already running under another row. This row
        // must not take hold of that one (stopping or removing this row would
        // stop it), so the copy runs in a torrent engine of its own.
        session.stop().await;
        session = own_session().await?;
        hold(live, session.clone(), 1);
        let (source, _) = open(&client, &rec.url, rec.torrent_file.as_deref()).await?;
        added = tokio::select! {
            _ = live.cancel.cancelled() => {
                session.stop().await;
                return Err(Fail::Cancelled);
            }
            r = session.add_torrent(source, Some(options())) => r,
        };
    }
    let (torrent_id, handle) = match added {
        Ok(AddTorrentResponse::Added(i, h)) => (i, h),
        Ok(AddTorrentResponse::AlreadyManaged(..)) => {
            if folder_is_new {
                remove_data(&rec.dir, &folder_name);
            }
            inner.update(id, |r| r.part_started = false);
            return Err(Fail::Fatal("this torrent is already downloading".into()));
        }
        Ok(AddTorrentResponse::ListOnly(_)) => {
            return Err(Fail::Fatal("the torrent could not be started".into()))
        }
        Err(e) => return Err(Fail::Retry(format!("could not open the torrent: {e:#}"))),
    };

    // Reads which pieces are done, for the progress bars.
    let pieces = librqbit::Api::new(session.clone(), None);

    // Web servers the .torrent file lists as holding its files are fetched
    // from as well, the way other clients do (see webseed.rs).
    let plan = torrent_bytes
        .as_deref()
        .and_then(WebSeeds::read)
        .map(|seeds| Arc::new(Plan::new(seeds, folder.clone(), rec.only_files.as_deref())));
    let seeds_stop = live.cancel.child_token();
    let mut fetchers = Vec::new();

    let _conn = ConnGuard::new(&live.conns);
    let mut started = false;
    let mut total = 0u64;
    let mut by_swarm = false;
    let mut peer_bytes = 0u64;
    // Each chosen file's place in the torrent, path and size, read once.
    let mut names: Option<Vec<(usize, Vec<String>, u64)>> = None;
    let outcome = loop {
        tokio::select! {
            _ = live.cancel.cancelled() => break Err(Fail::Cancelled),
            _ = tokio::time::sleep(Duration::from_millis(500)) => {}
        }
        let stats = handle.stats();
        total = stats.total_bytes;
        // How far each file is, for the list in the download's detail. All
        // the files download at once: the pieces are fetched from every
        // peer side by side, wherever in the torrent they fall.
        if names.is_none() {
            names = handle.with_metadata(|m| file_names(&m.file_infos, rec.only_files.as_deref())).ok();
        }
        if let Some(names) = &names {
            let files = files_progress(names, &stats.file_progress);
            if !files.is_empty() {
                *live.files.lock().unwrap() = files;
            }
        }
        // Before carrying on, the torrent library reads back what is already
        // on disk and checks every piece. Until it is done, the download is
        // shown as checking, with what it had before, and no speed.
        if matches!(stats.state, TorrentStatsState::Initializing { .. }) {
            if let Some(error) = stats.error {
                break Err(Fail::Fatal(format!("torrent error: {error}")));
            }
            live.checking.store(true, Relaxed);
            live.checked.store(stats.progress_bytes.min(total), Relaxed);
            if total > 0 {
                live.total_hint.store(total, Relaxed);
            }
            continue;
        }
        live.checking.store(false, Relaxed);
        // The speed shown follows the bytes as they arrive from peers and web
        // seeds, not the checked pieces, which land 4 MB or so at a time. The
        // library's count is only there while the torrent is live, so the
        // last one seen is kept, and the total never goes back.
        if let Some(l) = &stats.live {
            peer_bytes = peer_bytes.max(l.snapshot.fetched_bytes);
        }
        let seed_bytes = plan.as_ref().map_or(0, |p| p.received());
        live.received.fetch_max(peer_bytes + seed_bytes, Relaxed);
        live.counts_received.store(true, Relaxed);

        let haves = pieces.api_dump_haves(torrent_id.into()).ok();
        let plan = plan.as_ref().filter(|p| haves.as_ref().is_some_and(|(_, count)| *count as usize == p.piece_count()));
        if let (Some(plan), Some((have, _))) = (plan, &haves) {
            plan.set_swarm(|i| have.get(i).map(|b| *b).unwrap_or(false));
        }
        let extra = plan.map_or(0, |p| p.extra_bytes());
        live.downloaded.store((stats.progress_bytes + extra).min(total.max(stats.progress_bytes)), Relaxed);
        if total > 0 {
            live.total_hint.store(total, Relaxed);
        }
        // One bar for each worker there really is: the swarm's lane and each
        // web seed fetcher's (webseed.rs). With no web seeds the swarm is the
        // only worker, and the torrent shows as one bar.
        let lanes = plan.map(|p| p.lanes()).filter(|l| !l.is_empty());
        if let Some(lanes) = lanes {
            *live.segs.lock().unwrap() = lanes
                .into_iter()
                .map(|(start, pos, end)| Seg { start: start as u64, pos: pos as u64, end: end as u64, owned: false })
                .collect();
        } else if let Some((have, count)) = &haves {
            let count = *count as usize;
            let done = (0..count).filter(|i| have.get(*i).map(|b| *b).unwrap_or(false)).count();
            if count > 0 {
                *live.segs.lock().unwrap() = vec![Seg { start: 0, pos: done as u64, end: count as u64, owned: false }];
            }
        }
        if let Some(error) = stats.error {
            break Err(Fail::Fatal(format!("torrent error: {error}")));
        }
        if !started && matches!(stats.state, TorrentStatsState::Live) {
            started = true;
            inner.update(id, |r| r.status = Status::Downloading);
            // The files exist once the torrent is live; the web seeds start then.
            if let Some(plan) = plan.filter(|_| !stats.finished) {
                // One fetcher per web seed at most: when the swarm already
                // fills the line, more only split the same bandwidth.
                let count = rec.connections.clamp(2, 16).min(plan.mirror_count());
                plan.assign(count);
                for worker in 0..count {
                    fetchers.push(tokio::spawn(webseed::fetcher(
                        plan.clone(),
                        worker,
                        client.clone(),
                        webseed::Gate::Session(session.clone()),
                        live.clone(),
                        seeds_stop.clone(),
                    )));
                }
            }
        }
        if stats.finished {
            by_swarm = true;
            break Ok(());
        }
        if started && plan.is_some_and(|p| p.complete()) {
            break Ok(());
        }
    };
    seeds_stop.cancel();
    for fetcher in fetchers {
        let _ = fetcher.await;
    }

    // Stop the torrent in every case; the app does not keep uploading after a
    // download ends. Files are left for a pause and removed only when the
    // download itself is being removed from the list.
    let removing = outcome.is_err() && live.removed.load(Relaxed);
    let _ = session.delete(torrent_id.into(), removing).await;
    if private {
        session.stop().await;
    }
    if removing {
        remove_data(&rec.dir, &folder_name);
    }
    // Finished with the web seeds' help: what was only fetched from them is
    // checked once more now the torrent library has let go of the files.
    let outcome = match (outcome, &plan) {
        (Ok(()), Some(plan)) if !by_swarm => {
            webseed::settle(plan.clone(), client.clone(), webseed::Gate::Limiter(live.limiter.clone()), live.cancel.clone())
                .await
                .map_err(|e| if live.cancel.is_cancelled() { Fail::Cancelled } else { Fail::Retry(e) })
        }
        (outcome, _) => outcome,
    };
    if outcome.is_ok() {
        live.downloaded.store(total, Relaxed);
        // Finished: every chosen file is whole, web-seeded parts included.
        let files: Vec<TorrentFileProgress> = {
            let mut files = live.files.lock().unwrap();
            for f in files.iter_mut() {
                f.done = f.len;
            }
            files.clone()
        };
        inner.update(id, |r| {
            r.total = Some(total);
            if !files.is_empty() {
                r.files = files;
            }
        });
    }
    outcome
}

/// The torrent's files that are downloaded (the chosen ones, without the
/// padding some torrents put between files): place, path and size.
fn file_names(infos: &librqbit::FileInfos, only: Option<&[usize]>) -> Vec<(usize, Vec<String>, u64)> {
    infos
        .iter()
        .enumerate()
        .filter(|(i, f)| !f.attrs.padding && only.is_none_or(|only| only.contains(i)))
        .map(|(i, f)| {
            let path = f
                .relative_filename
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect();
            (i, path, f.len)
        })
        .collect()
}

/// Each file with how much of it is downloaded, from the library's count per
/// file; empty until the library has one.
fn files_progress(names: &[(usize, Vec<String>, u64)], progress: &[u64]) -> Vec<TorrentFileProgress> {
    if progress.is_empty() {
        return Vec::new();
    }
    names
        .iter()
        .map(|(i, path, len)| TorrentFileProgress {
            path: path.clone(),
            len: *len,
            done: progress.get(*i).copied().unwrap_or(0).min(*len),
            net: None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_file_takes_its_own_count() {
        let names = vec![(0, vec!["a.mkv".to_string()], 100), (2, vec!["sub".to_string(), "b.srt".to_string()], 10)];
        assert!(files_progress(&names, &[]).is_empty());
        let files = files_progress(&names, &[40, 0, 99]);
        assert_eq!(files[0].done, 40);
        assert_eq!(files[1].path, vec!["sub", "b.srt"]);
        // never more than the file holds
        assert_eq!(files[1].done, 10);
    }

    #[test]
    fn recognises_torrent_links() {
        assert!(is_torrent_link("magnet:?xt=urn:btih:ABC&dn=Some+Name"));
        assert!(is_torrent_link("https://example.test/files/thing.torrent?x=1"));
        assert!(!is_torrent_link("https://example.test/files/thing.zip"));
        assert!(!is_torrent_link("https://example.test/watch?v=torrent"));
    }

    #[test]
    fn names_come_from_the_link() {
        assert_eq!(default_name("magnet:?xt=urn:btih:ABC&dn=Some+File%20Name&tr=x"), "Some File Name");
        assert_eq!(default_name("magnet:?xt=urn:btih:ABC"), "Torrent");
        assert_eq!(default_name("https://example.test/a/ubuntu-24.04.iso.torrent"), "ubuntu-24.04.iso");
    }

    #[test]
    fn remove_data_stays_inside_the_folder() {
        let base = std::env::temp_dir().join(format!("rush-torrent-test-{}", std::process::id()));
        let inside = base.join("downloads");
        std::fs::create_dir_all(inside.join("mine")).unwrap();
        std::fs::create_dir_all(base.join("other")).unwrap();
        remove_data(&inside, "../other");
        remove_data(&inside, "..");
        remove_data(&inside, "");
        assert!(base.join("other").is_dir() && inside.is_dir());
        remove_data(&inside, "mine");
        assert!(!inside.join("mine").exists());
        let _ = std::fs::remove_dir_all(&base);
    }
}
