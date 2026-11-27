//! A torrent of several files, downloaded as one torrent per file, side by
//! side: each chosen file gets a torrent engine of its own (its own peer id,
//! its own connections and its own search for peers), all of them at once
//! (up to `AT_ONCE`), all writing into the torrent's one folder.
//!
//! One engine fetches a torrent's pieces in file order from every peer at
//! once. Separate engines can open more than one connection to the same
//! seeder, which helps where a single connection is what holds the speed
//! back. Many seeding programs accept only one connection from an address,
//! though, so this can also be no faster; Settings turns it off
//! (`torrent_files_apart`), and the torrent then runs as one.
//!
//! A piece that falls across the end of one file and the start of the next
//! may be fetched by both files' engines: a little extra download, written
//! the same both times.
//!
//! What keeps the engines fast: each one is started once and goes from file
//! to file, so it looks for peers once, not for every file; the peers any of
//! them finds are handed to every file started after; and the largest files
//! start first, as the longest of them decides when the torrent is done.

use std::collections::{HashSet, VecDeque};
use std::net::SocketAddr;
use std::num::NonZeroU32;
use std::path::Path;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use librqbit::limits::LimitsConfig;
use librqbit::{AddTorrent, AddTorrentOptions, AddTorrentResponse, Session, TorrentStatsState};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use crate::task::{ConnGuard, Fail, Live, Seg};
use crate::torrent::{open, remove_data, worker_session};
use crate::webseed::WebSeeds;
use crate::{FileNet, Inner, Record, Status, TorrentFileProgress};

/// Files downloaded at the same time, each by an engine of its own. A torrent
/// with more files than this fetches the rest as engines come free.
const AT_ONCE: usize = 12;

/// How long a magnet link may take to say what it holds.
const LIST_WAIT: Duration = Duration::from_secs(120);

/// What a torrent holds, read before it is downloaded file by file.
pub(crate) struct Listed {
    /// The whole `.torrent`, which every file's engine starts from.
    torrent: Vec<u8>,
    /// Each chosen file: its place in the torrent, folders then name, size.
    files: Vec<(usize, Vec<String>, u64)>,
    /// Peers met while reading what it holds (for a magnet link).
    peers: Vec<SocketAddr>,
}

/// Reads the torrent's chosen files through the app's own torrent engine
/// (for a magnet link this asks peers first). `None` where downloading file
/// by file does not apply: one file only, web seeds to fetch from (the
/// single-engine download uses them), or the torrent could not be read here
/// (the single-engine download then reports why).
pub(crate) async fn list(inner: &Arc<Inner>, session: &Arc<Session>, rec: &Record) -> Option<Listed> {
    let (source, _) = open(inner, &rec.url, rec.torrent_file.as_deref()).await.ok()?;
    let options = AddTorrentOptions {
        list_only: true,
        ..Default::default()
    };
    let listed = tokio::time::timeout(LIST_WAIT, session.add_torrent(source, Some(options))).await.ok()?.ok()?;
    let AddTorrentResponse::ListOnly(listed) = listed else {
        return None;
    };
    let torrent = listed.torrent_bytes.to_vec();
    if WebSeeds::read(&torrent).is_some() {
        return None;
    }
    let only = rec.only_files.as_deref();
    let files: Vec<(usize, Vec<String>, u64)> = listed
        .info
        .iter_file_details()
        .enumerate()
        .filter(|(i, f)| !f.attrs().padding && only.is_none_or(|only| only.contains(i)))
        .map(|(i, f)| {
            (i, f.filename.to_vec(), f.len)
        })
        .filter(|(_, path, _)| !path.is_empty())
        .collect();
    let peers = listed.seen_peers;
    (files.len() >= 2).then_some(Listed { torrent, files, peers })
}

/// How far one file's own engine has got.
#[derive(Clone, Copy, Default)]
struct Slot {
    /// Bytes of the file checked and written.
    done: u64,
    /// Bytes its engine has received from peers since it started.
    fetched: u64,
    /// Its engine has finished checking what was on disk and is fetching.
    live: bool,
    finished: bool,
    /// Its engine's peers and speeds right now.
    net: Option<FileNet>,
}

/// Downloads the listed files, `AT_ONCE` at a time, into `folder`, keeping
/// the download's row up to date with the whole and with each file.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run(
    inner: &Arc<Inner>,
    id: u64,
    live: &Arc<Live>,
    rec: &Record,
    folder: &Path,
    folder_name: &str,
    listed: Listed,
    had: u64,
) -> Result<(), Fail> {
    let Listed { torrent, files, peers } = listed;
    let total: u64 = files.iter().map(|(_, _, len)| len).sum();
    live.total_hint.store(total, Relaxed);
    inner.update(id, |r| r.total = Some(total));

    let slots = Arc::new(Mutex::new(vec![Slot::default(); files.len()]));
    let queue = Arc::new(Mutex::new(largest_first(&files)));
    // Every peer found so far, by any engine; each file starts with all of them.
    let known: Arc<Mutex<HashSet<SocketAddr>>> = Arc::new(Mutex::new(peers.into_iter().collect()));
    let stop = live.cancel.child_token();
    let torrent = Arc::new(torrent);
    // The speed limit is shared out between the files running at once.
    let workers = AT_ONCE.min(files.len());
    let limit = u32::try_from(inner.limiter.rate() / workers as u64).unwrap_or(u32::MAX);

    let _conn = ConnGuard::new(&live.conns);
    let mut running = JoinSet::new();
    for _ in 0..workers {
        let (inner, slots, queue, stop, torrent, known) =
            (inner.clone(), slots.clone(), queue.clone(), stop.clone(), torrent.clone(), known.clone());
        let places: Vec<usize> = files.iter().map(|(i, _, _)| *i).collect();
        let folder = folder.to_path_buf();
        running.spawn(async move {
            // One engine for this worker, kept from file to file.
            let session = tokio::select! {
                _ = stop.cancelled() => return Err(Fail::Cancelled),
                s = worker_session(&inner) => s?,
            };
            let outcome = loop {
                let Some(pos) = queue.lock().unwrap().pop_front() else {
                    break Ok(());
                };
                let work = File { torrent: &torrent, index: places[pos], folder: &folder, pos, limit };
                if let Err(fail) = one_file(&session, work, &slots, &known, &stop).await {
                    break Err(fail);
                }
                slots.lock().unwrap()[pos].finished = true;
            };
            session.stop().await;
            outcome
        });
    }

    let mut started = false;
    let outcome: Result<(), Fail> = loop {
        tokio::select! {
            _ = live.cancel.cancelled() => break Err(Fail::Cancelled),
            ended = running.join_next() => match ended {
                // Every worker has run out of files: all of them are done.
                None => break Ok(()),
                Some(Ok(Ok(()))) => continue,
                Some(Ok(Err(fail))) => break Err(fail),
                Some(Err(e)) => break Err(Fail::Retry(format!("a file's download stopped: {e}"))),
            },
            _ = tokio::time::sleep(Duration::from_millis(500)) => {}
        }
        let now = slots.lock().unwrap().clone();
        let done: u64 = files
            .iter()
            .zip(&now)
            .map(|((_, _, len), s)| if s.finished { *len } else { s.done.min(*len) })
            .sum();
        let fetched: u64 = now.iter().map(|s| s.fetched).sum();
        if now.iter().any(|s| s.live || s.finished) && !started {
            started = true;
            inner.update(id, |r| r.status = Status::Downloading);
        }
        // Until the engines have checked what is on disk, what was there
        // before stays shown.
        live.downloaded.store(if started { done } else { done.max(had) }, Relaxed);
        live.received.fetch_max(fetched, Relaxed);
        live.counts_received.store(true, Relaxed);
        *live.files.lock().unwrap() = progress(&files, &now);
        // One bar per file, side by side, for the row's view of its parts.
        if files.len() <= 64 {
            *live.segs.lock().unwrap() = file_segments(&files, &now);
        }
    };

    // Every file's engine stops, whatever the outcome, before anything else.
    stop.cancel();
    while running.join_next().await.is_some() {}

    if outcome.is_err() && live.removed.load(Relaxed) {
        remove_data(&rec.dir, folder_name);
    }
    if outcome.is_ok() {
        let finished: Vec<TorrentFileProgress> = files
            .iter()
            .map(|(_, path, len)| TorrentFileProgress { path: path.clone(), len: *len, done: *len, net: None })
            .collect();
        live.downloaded.store(total, Relaxed);
        *live.files.lock().unwrap() = finished.clone();
        inner.update(id, |r| {
            r.total = Some(total);
            r.files = finished;
        });
    }
    outcome
}

/// One file for a worker's engine to fetch.
struct File<'a> {
    torrent: &'a [u8],
    /// Its place in the torrent.
    index: usize,
    folder: &'a Path,
    /// Its place in the list being downloaded.
    pos: usize,
    /// Bytes per second for it, 0 for no limit.
    limit: u32,
}

/// Fetches one file on a worker's engine, watched until it is whole; the
/// torrent then leaves the engine, which stays up for the worker's next file.
async fn one_file(
    session: &Arc<Session>,
    file: File<'_>,
    slots: &Mutex<Vec<Slot>>,
    known: &Mutex<HashSet<SocketAddr>>,
    stop: &CancellationToken,
) -> Result<(), Fail> {
    let initial: Vec<SocketAddr> = known.lock().unwrap().iter().copied().collect();
    let options = AddTorrentOptions {
        overwrite: true,
        only_files: Some(vec![file.index]),
        output_folder: Some(file.folder.to_string_lossy().into_owned()),
        initial_peers: (!initial.is_empty()).then_some(initial),
        ratelimits: LimitsConfig {
            upload_bps: None,
            download_bps: NonZeroU32::new(file.limit),
        },
        ..Default::default()
    };
    let added = tokio::select! {
        _ = stop.cancelled() => return Err(Fail::Cancelled),
        r = session.add_torrent(AddTorrent::from_bytes(file.torrent.to_vec()), Some(options)) => r,
    };
    let (torrent_id, handle) = match added {
        Ok(AddTorrentResponse::Added(i, h)) => (i, h),
        Ok(_) => return Err(Fail::Fatal("the torrent could not be started".into())),
        Err(e) => return Err(Fail::Retry(format!("could not open the torrent: {e:#}"))),
    };
    let mut tick = 0u32;
    let outcome = loop {
        tokio::select! {
            _ = stop.cancelled() => break Err(Fail::Cancelled),
            _ = tokio::time::sleep(Duration::from_millis(500)) => {}
        }
        tick += 1;
        let stats = handle.stats();
        if let Some(error) = stats.error {
            break Err(Fail::Fatal(format!("torrent error: {error}")));
        }
        {
            let mut slots = slots.lock().unwrap();
            let slot = &mut slots[file.pos];
            slot.done = stats.file_progress.get(file.index).copied().unwrap_or(0);
            if let Some(l) = &stats.live {
                slot.fetched = slot.fetched.max(l.snapshot.fetched_bytes);
                let peers = &l.snapshot.peer_stats;
                slot.net = Some(FileNet {
                    peers: peers.live,
                    seeders: peers.live_seeders,
                    found: peers.seen,
                    down: l.download_speed.as_bytes(),
                    up: l.upload_speed.as_bytes(),
                });
            }
            slot.live = matches!(stats.state, TorrentStatsState::Live);
        }
        // The peers this engine is connected to are passed on, for the files
        // that start later: every second while it is getting going, then
        // every five.
        if (tick <= 60 && tick % 2 == 0) || tick % 10 == 0 {
            if let Some(live) = handle.live() {
                let peers = live.per_peer_stats_snapshot(Default::default()).peers;
                let mut known = known.lock().unwrap();
                known.extend(peers.keys().filter_map(|addr| addr.parse::<SocketAddr>().ok()));
            }
        }
        if stats.finished {
            break Ok(());
        }
    };
    // The app does not keep uploading after a file is done; its data stays.
    let _ = session.delete(torrent_id.into(), false).await;
    outcome
}

/// The order the files are fetched in: the largest first, since the longest
/// of them decides when the torrent is done; equal sizes keep torrent order.
fn largest_first(files: &[(usize, Vec<String>, u64)]) -> VecDeque<usize> {
    let mut order: Vec<usize> = (0..files.len()).collect();
    order.sort_by(|a, b| files[*b].2.cmp(&files[*a].2).then(a.cmp(b)));
    order.into()
}

/// Each file with how much of it is in.
fn progress(files: &[(usize, Vec<String>, u64)], slots: &[Slot]) -> Vec<TorrentFileProgress> {
    files
        .iter()
        .zip(slots)
        .map(|((_, path, len), s)| TorrentFileProgress {
            path: path.clone(),
            len: *len,
            done: if s.finished { *len } else { s.done.min(*len) },
            net: if s.finished { None } else { s.net },
        })
        .collect()
}

/// One part per file, laid end to end in torrent order: where it starts,
/// how far it is, where it ends, in bytes of the whole.
fn file_segments(files: &[(usize, Vec<String>, u64)], slots: &[Slot]) -> Vec<Seg> {
    let mut at = 0u64;
    files
        .iter()
        .zip(slots)
        .map(|((_, _, len), s)| {
            let done = if s.finished { *len } else { s.done.min(*len) };
            let seg = Seg {
                start: at,
                pos: at + done,
                end: at + len,
                owned: false,
            };
            at += len;
            seg
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files() -> Vec<(usize, Vec<String>, u64)> {
        vec![(0, vec!["a.bin".into()], 100), (2, vec!["b.bin".into()], 50)]
    }

    #[test]
    fn the_largest_file_starts_first() {
        let files = vec![
            (0, vec!["small.txt".into()], 5),
            (1, vec!["huge.bin".into()], 900),
            (2, vec!["mid.bin".into()], 50),
            (3, vec!["mid2.bin".into()], 50),
        ];
        assert_eq!(Vec::from(largest_first(&files)), vec![1, 2, 3, 0]);
    }

    #[test]
    fn each_file_has_its_own_part() {
        let slots = [Slot { done: 40, ..Slot::default() }, Slot { finished: true, ..Slot::default() }];
        let segs = file_segments(&files(), &slots);
        assert_eq!((segs[0].start, segs[0].pos, segs[0].end), (0, 40, 100));
        // a finished file is whole, whatever its engine last said
        assert_eq!((segs[1].start, segs[1].pos, segs[1].end), (100, 150, 150));
        let shown = progress(&files(), &slots);
        assert_eq!(shown[0].done, 40);
        assert_eq!(shown[1].done, 50);
    }
}
