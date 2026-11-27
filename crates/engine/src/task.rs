//! Runs a single download: probes the server, then either downloads the file
//! over several connections with dynamic segment splitting, or falls back to
//! one sequential stream when the server does not support ranges.

use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use reqwest::header::{CONTENT_RANGE, RANGE};
use reqwest::{Client, StatusCode};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use crate::limiter::Limiter;
use crate::probe::{self, candidate, net_err, part_path, request, sanitize, status_fail};
use crate::{Inner, Status};

/// Initial segments are at least this large.
const MIN_SEG: u64 = 1 << 20;
/// A busy segment is split for an idle connection only while each half stays
/// at least this large. Kept small so no single slow connection is left
/// holding a big tail at the end of a download.
const MIN_SPLIT: u64 = 256 << 10;
/// Consecutive failures a connection tolerates before giving up.
const MAX_FAILS: u32 = 6;
pub(crate) const MAX_CONNECTIONS: usize = 32;

#[derive(Debug)]
pub(crate) enum Fail {
    Cancelled,
    /// Not worth retrying (bad URL, 404, disk error...).
    Fatal(String),
    /// Transient (network drop, 5xx...).
    Retry(String),
}

/// A byte range `[start, end)` of the file; `pos` is the next byte to fetch.
pub(crate) struct Seg {
    pub start: u64,
    pub pos: u64,
    pub end: u64,
    pub owned: bool,
}

/// State shared between a running download and the engine.
pub(crate) struct Live {
    pub cancel: CancellationToken,
    pub segs: Mutex<Vec<Seg>>,
    pub downloaded: AtomicU64,
    pub conns: AtomicUsize,
    pub removed: AtomicBool,
    /// Total size when it is only learned while running (site downloads).
    pub total_hint: AtomicU64,
    /// Connection attempts that have failed in a row while connecting.
    pub connect_failures: AtomicU64,
    /// Files fetched so far, for profile downloads.
    pub items: AtomicU64,
    /// The file being fetched right now, for profile and playlist downloads.
    pub current: Mutex<Option<crate::CurrentFile>>,
    /// Bytes received over the network since this run started, for downloads
    /// whose progress only grows in large checked steps (torrents). The speed
    /// shown is taken from this when `counts_received` is set.
    pub received: AtomicU64,
    pub counts_received: AtomicBool,
    /// Set while what is already on disk is being checked before the
    /// download carries on (a torrent's pieces); `checked` is how far along.
    pub checking: AtomicBool,
    pub checked: AtomicU64,
    /// A torrent's files with how far each is, refreshed while it runs.
    pub files: Mutex<Vec<crate::TorrentFileProgress>>,
    fatal: Mutex<Option<String>>,
}

impl Live {
    pub fn new() -> Self {
        Self {
            cancel: CancellationToken::new(),
            segs: Mutex::new(Vec::new()),
            downloaded: AtomicU64::new(0),
            conns: AtomicUsize::new(0),
            removed: AtomicBool::new(false),
            total_hint: AtomicU64::new(0),
            connect_failures: AtomicU64::new(0),
            items: AtomicU64::new(0),
            current: Mutex::new(None),
            received: AtomicU64::new(0),
            counts_received: AtomicBool::new(false),
            checking: AtomicBool::new(false),
            checked: AtomicU64::new(0),
            files: Mutex::new(Vec::new()),
            fatal: Mutex::new(None),
        }
    }
}

pub(crate) struct ConnGuard<'a>(&'a AtomicUsize);
impl<'a> ConnGuard<'a> {
    pub(crate) fn new(c: &'a AtomicUsize) -> Self {
        c.fetch_add(1, Relaxed);
        Self(c)
    }
}
impl Drop for ConnGuard<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Relaxed);
    }
}

#[cfg(windows)]
pub(crate) fn write_all_at(f: &File, mut buf: &[u8], mut off: u64) -> io::Result<()> {
    use std::os::windows::fs::FileExt;
    while !buf.is_empty() {
        let n = f.seek_write(buf, off)?;
        if n == 0 {
            return Err(io::ErrorKind::WriteZero.into());
        }
        buf = &buf[n..];
        off += n as u64;
    }
    Ok(())
}

#[cfg(unix)]
pub(crate) fn write_all_at(f: &File, buf: &[u8], off: u64) -> io::Result<()> {
    use std::os::unix::fs::FileExt;
    f.write_all_at(buf, off)
}

struct Ctx {
    client: Client,
    url: String,
    headers: Vec<(String, String)>,
    file: File,
    live: Arc<Live>,
    limiter: Arc<Limiter>,
    /// Workers that have either connected once or exited without connecting.
    settled: AtomicUsize,
    /// The server refused an additional connection; stop opening more.
    saturated: AtomicBool,
    /// Nothing left that is worth giving to another connection.
    no_work: AtomicBool,
    /// Largest byte range asked for in one request; 0 for no limit. Some
    /// video hosts slow down a connection that asks for more at once.
    request_cap: u64,
}

impl Ctx {
    /// Picks the next segment for a connection: an abandoned one if any,
    /// otherwise the back half of the busiest connection's remaining range.
    fn claim(&self) -> Option<usize> {
        let mut segs = self.live.segs.lock().unwrap();
        if let Some(i) = segs.iter().position(|s| !s.owned && s.pos < s.end) {
            segs[i].owned = true;
            return Some(i);
        }
        let (i, rem) = segs
            .iter()
            .enumerate()
            .filter(|(_, s)| s.owned)
            .map(|(i, s)| (i, s.end.saturating_sub(s.pos)))
            .max_by_key(|&(_, rem)| rem)?;
        if rem < 2 * MIN_SPLIT {
            return None;
        }
        let mid = segs[i].pos + rem / 2;
        let end = segs[i].end;
        segs[i].end = mid;
        segs.push(Seg {
            start: mid,
            pos: mid,
            end,
            owned: true,
        });
        Some(segs.len() - 1)
    }

    fn pos(&self, i: usize) -> u64 {
        self.live.segs.lock().unwrap()[i].pos
    }

    async fn worker(self: Arc<Self>) -> Result<(), Fail> {
        let mut connected = false;
        let res = self.work(&mut connected).await;
        if !connected {
            self.settled.fetch_add(1, Relaxed);
        }
        res
    }

    async fn work(&self, connected: &mut bool) -> Result<(), Fail> {
        loop {
            let Some(i) = self.claim() else {
                self.no_work.store(true, Relaxed);
                return Ok(());
            };
            let res = self.run_segment(i, connected).await;
            self.live.segs.lock().unwrap()[i].owned = false;
            res?;
        }
    }

    async fn run_segment(&self, i: usize, connected: &mut bool) -> Result<(), Fail> {
        let mut fails = 0u32;
        loop {
            let before = self.pos(i);
            match self.stream(i, connected).await {
                Ok(()) => return Ok(()),
                Err(Fail::Retry(msg)) => {
                    // An extra connection that never got through while others
                    // are transferring means the server caps connections per
                    // client. Bow out instead of hammering it; the segment is
                    // picked up by the connections that do work.
                    if !*connected && fails >= 1 && self.live.conns.load(Relaxed) > 0 {
                        crate::media::debug(|| format!("an extra connection was refused: {msg}"));
                        self.saturated.store(true, Relaxed);
                        return Err(Fail::Retry(msg));
                    }
                    if self.pos(i) > before {
                        fails = 0;
                    }
                    fails += 1;
                    if fails > MAX_FAILS {
                        return Err(Fail::Retry(msg));
                    }
                    let delay = Duration::from_millis(500 << fails.min(5));
                    tokio::select! {
                        _ = self.live.cancel.cancelled() => return Err(Fail::Cancelled),
                        _ = tokio::time::sleep(delay) => {}
                    }
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// One HTTP range request for the unfinished part of segment `i`.
    async fn stream(&self, i: usize, connected: &mut bool) -> Result<(), Fail> {
        let (pos, end) = {
            let segs = self.live.segs.lock().unwrap();
            (segs[i].pos, segs[i].end)
        };
        if pos >= end {
            return Ok(());
        }
        // With a cap, the rest of the segment is fetched by the next request.
        let mut stop = if self.request_cap > 0 {
            end.min(pos + self.request_cap)
        } else {
            end
        };
        // Never leave a sliver for the next request: hosts that cap request
        // sizes also turn away very small ranges.
        if end - stop < 64 * 1024 {
            stop = end;
        }
        let req = request(&self.client, &self.url, &self.headers)
            .header(RANGE, format!("bytes={}-{}", pos, stop - 1));
        let mut resp = tokio::select! {
            _ = self.live.cancel.cancelled() => return Err(Fail::Cancelled),
            r = req.send() => r.map_err(net_err)?,
        };
        let status = resp.status();
        if status == StatusCode::OK {
            return Err(Fail::Fatal("server stopped honouring range requests".into()));
        }
        if status != StatusCode::PARTIAL_CONTENT {
            crate::media::debug(|| {
                format!(
                    "{status} for bytes={pos}-{} of segment {i}; sent headers {:?}; link starts {}",
                    stop - 1,
                    self.headers.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(),
                    self.url.chars().take(60).collect::<String>()
                )
            });
            return Err(status_fail(status));
        }
        let starts_at = resp
            .headers()
            .get(CONTENT_RANGE)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().strip_prefix("bytes"))
            .and_then(|v| v.trim().split('-').next())
            .and_then(|v| v.trim().parse::<u64>().ok());
        if starts_at.is_some_and(|s| s != pos) {
            return Err(Fail::Fatal("server returned the wrong byte range".into()));
        }

        let _conn = ConnGuard::new(&self.live.conns);
        if !*connected {
            *connected = true;
            self.settled.fetch_add(1, Relaxed);
        }
        loop {
            let chunk = tokio::select! {
                _ = self.live.cancel.cancelled() => return Err(Fail::Cancelled),
                c = resp.chunk() => c.map_err(net_err)?,
            };
            let Some(chunk) = chunk else {
                return Err(Fail::Retry("connection closed early".into()));
            };
            // Waiting for the speed limit must not hold up a pause or a cancel.
            tokio::select! {
                _ = self.live.cancel.cancelled() => return Err(Fail::Cancelled),
                _ = self.limiter.acquire(chunk.len()) => {}
            }

            // Reserve the bytes under the lock so a concurrent split never
            // overlaps with what this connection is about to write.
            let (off, n, done) = {
                let mut segs = self.live.segs.lock().unwrap();
                let s = &mut segs[i];
                let n = s.end.min(stop).saturating_sub(s.pos).min(chunk.len() as u64);
                let off = s.pos;
                s.pos += n;
                (off, n as usize, s.pos >= s.end || s.pos >= stop)
            };
            if n > 0 {
                if let Err(e) = write_all_at(&self.file, &chunk[..n], off) {
                    // Give the reservation back and stop the whole download.
                    self.live.segs.lock().unwrap()[i].pos = off;
                    let msg = format!("could not write to disk: {e}");
                    *self.live.fatal.lock().unwrap() = Some(msg.clone());
                    self.live.cancel.cancel();
                    return Err(Fail::Fatal(msg));
                }
                self.live.downloaded.fetch_add(n as u64, Relaxed);
            }
            if done {
                return Ok(());
            }
        }
    }
}

/// Runs the connections of one ranged download until every segment is done.
async fn drive(ctx: Arc<Ctx>, conns: usize, live: &Arc<Live>) -> Result<(), Fail> {
    // Ramp up 1, 2, 4, 8... connections: each batch starts only after
    // the previous one is transferring, and ramp-up stops as soon as the
    // server refuses a connection. Opening everything at once gets
    // clients throttled or temporarily blocked by some hosts.
    let mut set = JoinSet::new();
    let mut spawned = 0;
    'ramp: while spawned < conns {
        while ctx.settled.load(Relaxed) < spawned {
            tokio::select! {
                _ = live.cancel.cancelled() => break 'ramp,
                _ = tokio::time::sleep(Duration::from_millis(25)) => {}
            }
        }
        if ctx.saturated.load(Relaxed) || ctx.no_work.load(Relaxed) {
            break;
        }
        for _ in 0..spawned.max(1).min(conns - spawned) {
            set.spawn(ctx.clone().worker());
            spawned += 1;
        }
    }
    crate::media::debug(|| {
        format!(
            "{spawned} of {conns} connections opened (refused: {}, nothing left to split: {})",
            ctx.saturated.load(Relaxed),
            ctx.no_work.load(Relaxed)
        )
    });
    let mut last_err = None;
    while let Some(res) = set.join_next().await {
        match res {
            Ok(Err(Fail::Fatal(m))) | Ok(Err(Fail::Retry(m))) => {
                crate::media::debug(|| format!("a connection stopped: {m}"));
                last_err = Some(m)
            }
            Err(e) => last_err = Some(format!("internal error: {e}")),
            _ => {}
        }
    }

    if let Some(msg) = live.fatal.lock().unwrap().take() {
        return Err(Fail::Fatal(msg));
    }
    let complete = live.segs.lock().unwrap().iter().all(|s| s.pos >= s.end);
    if !complete {
        if live.cancel.is_cancelled() {
            return Err(Fail::Cancelled);
        }
        return Err(Fail::Fatal(
            last_err.unwrap_or_else(|| "download incomplete".into()),
        ));
    }
    let ctx = Arc::into_inner(ctx).ok_or_else(|| Fail::Fatal("internal error".into()))?;
    ctx.file
        .sync_all()
        .map_err(|e| io_fail("could not flush file", e))
}

/// Downloads one direct link of known size into `path` with several
/// connections. Used for the video and audio streams of site downloads.
/// `resume` holds the segments saved when the same stream was paused.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn fetch_stream(
    inner: &Arc<Inner>,
    live: &Arc<Live>,
    url: &str,
    headers: &[(String, String)],
    path: &Path,
    total: u64,
    conns: usize,
    request_cap: u64,
    resume: Option<Vec<[u64; 3]>>,
) -> Result<(), Fail> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|e| io_fail("could not create file", e))?;
    let conns = conns.clamp(1, MAX_CONNECTIONS);
    let segs: Vec<Seg> = match resume {
        Some(saved) => saved
            .iter()
            .map(|s| Seg {
                start: s[0],
                pos: s[1],
                end: s[2],
                owned: false,
            })
            .collect(),
        None => {
            file.set_len(total)
                .map_err(|e| io_fail("could not allocate disk space", e))?;
            let count = (total / MIN_SEG).clamp(1, conns as u64);
            let size = total / count;
            (0..count)
                .map(|i| {
                    let start = i * size;
                    let end = if i + 1 == count { total } else { start + size };
                    Seg {
                        start,
                        pos: start,
                        end,
                        owned: false,
                    }
                })
                .collect()
        }
    };
    let done: u64 = segs.iter().map(|s| s.pos - s.start).sum();
    live.downloaded.fetch_add(done, Relaxed);
    *live.segs.lock().unwrap() = segs;

    let ctx = Arc::new(Ctx {
        client: inner.client.clone(),
        url: url.to_string(),
        headers: headers.to_vec(),
        file,
        live: live.clone(),
        limiter: inner.limiter.clone(),
        settled: AtomicUsize::new(0),
        saturated: AtomicBool::new(false),
        no_work: AtomicBool::new(false),
        request_cap,
    });
    drive(ctx, conns, live).await
}

fn io_fail(what: &str, e: io::Error) -> Fail {
    Fail::Fatal(format!("{what}: {e}"))
}

/// Reserves a unique file name in `dir` by creating its `.part` file.
fn create_part(dir: &Path, base: &str) -> Result<(String, File), Fail> {
    for n in 0..10_000 {
        let name = candidate(base, n);
        if dir.join(&name).exists() {
            continue;
        }
        match OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(part_path(dir, &name))
        {
            Ok(f) => return Ok((name, f)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(io_fail("could not create file", e)),
        }
    }
    Err(Fail::Fatal("could not find a free file name".into()))
}

/// Moves the finished `.part` file to its final name; returns that name.
fn finalize(dir: &Path, filename: &str) -> Result<String, Fail> {
    let part = part_path(dir, filename);
    for n in 0..10_000 {
        let name = candidate(filename, n);
        let dest = dir.join(&name);
        if dest.exists() {
            continue;
        }
        return std::fs::rename(&part, &dest)
            .map(|_| name)
            .map_err(|e| io_fail("could not move finished file", e));
    }
    Err(Fail::Fatal("could not find a free file name".into()))
}

/// A link to a web page rather than to a file: no file extension, or one
/// that pages use.
fn looks_like_page(link: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(link) else {
        return false;
    };
    let last = url.path_segments().and_then(|mut s| s.next_back()).unwrap_or("");
    match last.rsplit_once('.') {
        Some((_, ext)) if !ext.is_empty() && ext.len() <= 5 => {
            matches!(ext.to_ascii_lowercase().as_str(), "html" | "htm" | "php" | "asp" | "aspx" | "jsp")
        }
        _ => true,
    }
}

pub(crate) async fn run(inner: &Arc<Inner>, id: u64, live: &Arc<Live>) -> Result<(), Fail> {
    let Some(rec) = inner.record(id) else {
        return Err(Fail::Cancelled);
    };
    if !rec.bundle.is_empty() {
        return crate::bundle::run(inner, id, live, &rec).await;
    }
    if rec.profile {
        return crate::profile::run(inner, id, live, &rec).await;
    }
    if rec.torrent {
        return crate::torrent::run(inner, id, live, &rec).await;
    }
    if rec.media {
        return crate::media::run(inner, id, live, &rec).await;
    }

    // Probe, retrying transient failures a few times.
    let mut attempt = 0u32;
    let probe = loop {
        let res = tokio::select! {
            _ = live.cancel.cancelled() => return Err(Fail::Cancelled),
            p = probe::probe(&inner.client, &rec.url, &rec.headers) => p,
        };
        match res {
            Ok(p) => {
                live.connect_failures.store(0, Relaxed);
                break p;
            }
            Err(Fail::Retry(msg)) => {
                attempt += 1;
                crate::media::debug(|| format!("connecting failed (attempt {attempt}): {msg}"));
                // Shown on the connecting bar: trouble first, then still failing.
                live.connect_failures.store(attempt as u64, Relaxed);
                inner.changed();
                // A page that cannot be reached this way may still open for
                // the site downloader, which connects the way a browser does.
                // It gets its try at once instead of after every retry here.
                if looks_like_page(&rec.url) {
                    return match crate::media::run(inner, id, live, &rec).await {
                        Err(Fail::Fatal(m)) if m.contains("no video RushDownload can download") => {
                            Err(Fail::Retry(msg))
                        }
                        other => other,
                    };
                }
                if attempt >= 4 {
                    return Err(Fail::Retry(msg));
                }
                tokio::select! {
                    _ = live.cancel.cancelled() => return Err(Fail::Cancelled),
                    _ = tokio::time::sleep(Duration::from_secs(1 << attempt)) => {}
                }
            }
            // Many sites turn away anything that does not look like a
            // browser. When the link is a page rather than a file, the site
            // downloader still gets its try before the download gives up.
            Err(Fail::Fatal(msg)) if msg.starts_with("HTTP 4") && looks_like_page(&rec.url) => {
                return match crate::media::run(inner, id, live, &rec).await {
                    Err(Fail::Fatal(m)) if m.contains("no video RushDownload can download") => Err(Fail::Fatal(
                        format!(
                            "The site refused the request ({msg}) and no video could be found from the link alone. \
                             Open the page on the Browser page and play the video to catch it"
                        ),
                    )),
                    other => other,
                };
            }
            Err(e) => return Err(e),
        }
    };

    if probe.is_html {
        // A page, not a file: let the site backend find the media on it.
        drop(probe);
        return crate::media::run(inner, id, live, &rec).await;
    }

    std::fs::create_dir_all(&rec.dir).map_err(|e| io_fail("could not create folder", e))?;

    // Can we continue from what is already on disk?
    let old_part = part_path(&rec.dir, &rec.filename);
    let can_resume = rec.part_started
        && probe.resumable
        && rec.resumable
        && rec.total.is_some()
        && rec.total == probe.total
        && !rec.segments.is_empty()
        && (rec.etag.is_none() || probe.etag.is_none() || rec.etag == probe.etag)
        && std::fs::metadata(&old_part).map(|m| m.len()).ok() == rec.total;

    let (filename, file) = if can_resume {
        let f = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&old_part)
            .map_err(|e| io_fail("could not open partial file", e))?;
        (rec.filename.clone(), f)
    } else {
        if rec.part_started {
            let _ = std::fs::remove_file(&old_part);
        }
        let base = if rec.user_named || rec.part_started {
            rec.filename.clone()
        } else {
            sanitize(probe.filename.as_deref().unwrap_or(&rec.filename))
        };
        create_part(&rec.dir, &base)?
    };

    if probe.resumable {
        let total = probe.total.unwrap_or(0);
        let conns = rec.connections.clamp(1, MAX_CONNECTIONS);
        let segs: Vec<Seg> = if can_resume {
            rec.segments
                .iter()
                .map(|s| Seg {
                    start: s[0],
                    pos: s[1],
                    end: s[2],
                    owned: false,
                })
                .collect()
        } else {
            file.set_len(total)
                .map_err(|e| io_fail("could not allocate disk space", e))?;
            let count = (total / MIN_SEG).clamp(1, conns as u64);
            let size = total / count;
            (0..count)
                .map(|i| {
                    let start = i * size;
                    let end = if i + 1 == count { total } else { start + size };
                    Seg {
                        start,
                        pos: start,
                        end,
                        owned: false,
                    }
                })
                .collect()
        };
        let done: u64 = segs.iter().map(|s| s.pos - s.start).sum();
        live.downloaded.store(done, Relaxed);
        *live.segs.lock().unwrap() = segs;
        inner.update(id, |r| {
            r.filename = filename.clone();
            r.part_started = true;
            r.total = Some(total);
            r.resumable = true;
            r.etag = probe.etag.clone();
            r.status = Status::Downloading;
        });
        drop(probe);

        let ctx = Arc::new(Ctx {
            client: inner.client.clone(),
            url: rec.url.clone(),
            headers: rec.headers.clone(),
            file,
            live: live.clone(),
            limiter: inner.limiter.clone(),
            settled: AtomicUsize::new(0),
            saturated: AtomicBool::new(false),
            no_work: AtomicBool::new(false),
            request_cap: 0,
        });
        drive(ctx, conns, live).await?;
    } else {
        // No range support: one sequential stream, restarted from zero.
        file.set_len(0)
            .map_err(|e| io_fail("could not reset file", e))?;
        live.downloaded.store(0, Relaxed);
        live.segs.lock().unwrap().clear();
        let total = probe.total;
        inner.update(id, |r| {
            r.filename = filename.clone();
            r.part_started = true;
            r.total = total;
            r.resumable = false;
            r.etag = probe.etag.clone();
            r.segments.clear();
            r.downloaded = 0;
            r.status = Status::Downloading;
        });

        let mut resp = match probe.body {
            Some(r) => r,
            None => {
                let r = tokio::select! {
                    _ = live.cancel.cancelled() => return Err(Fail::Cancelled),
                    r = request(&inner.client, &rec.url, &rec.headers).send() => r.map_err(net_err)?,
                };
                if !r.status().is_success() {
                    return Err(status_fail(r.status()));
                }
                r
            }
        };
        let _conn = ConnGuard::new(&live.conns);
        let mut off = 0u64;
        loop {
            let chunk = tokio::select! {
                _ = live.cancel.cancelled() => return Err(Fail::Cancelled),
                c = resp.chunk() => c.map_err(net_err)?,
            };
            let Some(chunk) = chunk else { break };
            inner.limiter.acquire(chunk.len()).await;
            write_all_at(&file, &chunk, off).map_err(|e| io_fail("could not write to disk", e))?;
            off += chunk.len() as u64;
            live.downloaded.store(off, Relaxed);
        }
        if total.is_some_and(|t| t != off) {
            return Err(Fail::Retry("connection closed early".into()));
        }
        file.sync_all()
            .map_err(|e| io_fail("could not flush file", e))?;
        drop(file);
        inner.update(id, |r| r.total = Some(off));
    }

    let final_name = finalize(&rec.dir, &filename)?;
    inner.update(id, |r| r.filename = final_name.clone());
    Ok(())
}
