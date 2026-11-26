//! BitTorrent downloads: magnet links and links to `.torrent` files, through
//! librqbit. Each torrent gets a folder of its own inside the download
//! folder, so pausing, resuming and removing act on exactly that folder.

use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering::Relaxed;
use std::sync::Arc;
use std::time::Duration;

use librqbit::limits::LimitsConfig;
use librqbit::{
    AddTorrent, AddTorrentOptions, AddTorrentResponse, DhtSessionConfig, Session, SessionOptions,
    TorrentStatsState,
};
use percent_encoding::percent_decode_str;
use serde::Serialize;

use crate::probe::{candidate, sanitize};
use crate::task::{ConnGuard, Fail, Live, Seg};
use crate::{Inner, Record, Status};

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
fn open(source: &str, saved_copy: Option<&Path>) -> Result<AddTorrent<'static>, Fail> {
    let file = match saved_copy {
        Some(copy) if copy.is_file() => Some(copy.to_path_buf()),
        _ if is_local_torrent(source) => Some(PathBuf::from(source)),
        _ => None,
    };
    match file {
        Some(path) => std::fs::read(&path)
            .map(AddTorrent::from_bytes)
            .map_err(|e| Fail::Fatal(format!("could not read the torrent file: {e}"))),
        None => Ok(AddTorrent::from_url(source.to_string())),
    }
}

/// Reads a torrent's name and file list without downloading its contents.
pub(crate) async fn preview(inner: &Arc<Inner>, source: &str) -> Result<TorrentPreview, String> {
    let source = clean(source);
    let fail = |f: Fail| match f {
        Fail::Fatal(m) | Fail::Retry(m) => m,
        Fail::Cancelled => "cancelled".to_string(),
    };
    let add = open(&source, None).map_err(fail)?;
    let session = session(inner).await.map_err(fail)?;
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

fn session_options(dht: bool) -> SessionOptions {
    SessionOptions {
        // Needed to find peers for magnet links. Nothing is kept
        // between runs, and no port is opened for incoming connections.
        // RUSHDOWNLOAD_NO_DHT turns it off (trackers only), for tests.
        dht: (dht && std::env::var_os("RUSHDOWNLOAD_NO_DHT").is_none()).then(|| DhtSessionConfig {
            bootstrap_addrs: None,
            port: None,
            persistence: None,
        }),
        persistence: None,
        fastresume: false,
        client_name_and_version: Some(concat!("RushDownload ", env!("CARGO_PKG_VERSION")).into()),
        ..Default::default()
    }
}

async fn session(inner: &Arc<Inner>) -> Result<Arc<Session>, Fail> {
    inner
        .torrents
        .get_or_try_init(|| async {
            Session::new_with_opts(inner.download_dir(), session_options(true)).await
        })
        .await
        .cloned()
        .map_err(|e| Fail::Fatal(format!("could not start the torrent engine: {e:#}")))
}

/// A torrent engine of its own for a second copy of a torrent that is already
/// running: one engine holds a torrent only once, so the copy gets a separate
/// one and the two never share anything.
async fn private_session(inner: &Arc<Inner>) -> Result<Arc<Session>, Fail> {
    match Session::new_with_opts(inner.download_dir(), session_options(true)).await {
        Ok(s) => Ok(s),
        // If a second peer finder cannot start, trackers alone still work.
        Err(_) => Session::new_with_opts(inner.download_dir(), session_options(false))
            .await
            .map_err(|e| Fail::Fatal(format!("could not start the torrent engine: {e:#}"))),
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

    let mut session = session(inner).await?;
    let mut private = false;
    let limit = u32::try_from(inner.limiter.rate()).unwrap_or(u32::MAX);
    let options = || AddTorrentOptions {
        overwrite: true,
        // `None` downloads everything; otherwise only the chosen files.
        only_files: rec.only_files.clone(),
        output_folder: Some(folder.to_string_lossy().into_owned()),
        ratelimits: LimitsConfig {
            upload_bps: None,
            download_bps: NonZeroU32::new(limit),
        },
        ..Default::default()
    };

    // For a magnet link this first has to find peers and fetch the torrent's
    // details from them, which can take a while.
    let source = open(&rec.url, rec.torrent_file.as_deref())?;
    let mut added = tokio::select! {
        _ = live.cancel.cancelled() => return Err(Fail::Cancelled),
        r = session.add_torrent(source, Some(options())) => r,
    };
    if matches!(added, Ok(AddTorrentResponse::AlreadyManaged(..))) {
        // The same torrent is already running under another row. This row
        // must not take hold of that one (stopping or removing this row would
        // stop it), so the copy runs in a torrent engine of its own.
        session = private_session(inner).await?;
        private = true;
        let source = open(&rec.url, rec.torrent_file.as_deref())?;
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

    // Reads which pieces are done, for the per-part progress bars.
    let pieces = librqbit::Api::new(session.clone(), None);
    let part_count = rec.connections.clamp(2, 16);

    let _conn = ConnGuard::new(&live.conns);
    let mut started = false;
    let mut total = 0u64;
    let outcome = loop {
        tokio::select! {
            _ = live.cancel.cancelled() => break Err(Fail::Cancelled),
            _ = tokio::time::sleep(Duration::from_millis(500)) => {}
        }
        let stats = handle.stats();
        total = stats.total_bytes;
        live.downloaded.store(stats.progress_bytes, Relaxed);
        if total > 0 {
            live.total_hint.store(total, Relaxed);
        }
        // A torrent's pieces arrive from many peers at once, all over the
        // file. The file is cut into equal stretches and each one reports how
        // many of its pieces are done, so the bars fill side by side.
        if let Ok((have, count)) = pieces.api_dump_haves(torrent_id.into()) {
            let count = count as usize;
            let parts = part_count.min(count);
            if parts > 1 {
                let segs: Vec<Seg> = (0..parts)
                    .map(|p| {
                        let from = p * count / parts;
                        let to = (p + 1) * count / parts;
                        let done = (from..to)
                            .filter(|i| have.get(*i).map(|b| *b).unwrap_or(false))
                            .count();
                        Seg {
                            start: from as u64,
                            pos: (from + done) as u64,
                            end: to as u64,
                            owned: false,
                        }
                    })
                    .collect();
                *live.segs.lock().unwrap() = segs;
            }
        }
        if let Some(error) = stats.error {
            break Err(Fail::Fatal(format!("torrent error: {error}")));
        }
        if !started && matches!(stats.state, TorrentStatsState::Live) {
            started = true;
            inner.update(id, |r| r.status = Status::Downloading);
        }
        if stats.finished {
            break Ok(());
        }
    };

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
    if outcome.is_ok() {
        inner.update(id, |r| r.total = Some(total));
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

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
