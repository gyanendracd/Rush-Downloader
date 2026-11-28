//! A set of files picked from one site link (a release's assets, a shared
//! folder, a data set) downloaded as one download: each file on the usual
//! segmented engine, a few at a time, into the download's own folder, with
//! how far each file is kept for the detail view.

use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering::Relaxed;
use std::sync::Arc;
use std::time::Duration;

use tokio::task::JoinSet;

use crate::probe::{self, net_err, request, status_fail};
use crate::sites::{self, SiteFile};
use crate::task::{fetch_stream, write_all_at, ConnGuard, Fail, Live};
use crate::{Inner, Record, Status, TorrentFileProgress};

/// Files downloading at the same time.
const AT_ONCE: usize = 3;
/// Tries per file before it is given up on.
const TRIES: u32 = 3;

/// Where a file of the set is saved.
pub(crate) fn file_path(folder: &Path, file: &SiteFile) -> PathBuf {
    file.path.iter().fold(folder.to_path_buf(), |p, part| p.join(part))
}

fn part_of(path: &Path) -> PathBuf {
    let mut name = path.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(".part");
    path.with_file_name(name)
}

/// What is on disk already: a file whose finished copy is there counts as
/// done, with its size as found.
fn progress(rec: &Record, folder: &Path) -> Vec<TorrentFileProgress> {
    rec.bundle
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let path = file_path(folder, f);
            let before = rec.files.get(i);
            let finished_before = before.is_some_and(|b| b.len > 0 && b.done >= b.len);
            let on_disk = std::fs::metadata(&path).ok().filter(|m| m.is_file()).map(|m| m.len());
            match on_disk {
                Some(len) if finished_before || f.size == Some(len) => {
                    TorrentFileProgress { path: f.path.clone(), len, done: len, net: None }
                }
                _ => TorrentFileProgress {
                    path: f.path.clone(),
                    len: before.map(|b| b.len).filter(|l| *l > 0).or(f.size).unwrap_or(0),
                    done: 0,
                    net: None,
                },
            }
        })
        .collect()
}

pub(crate) async fn run(inner: &Arc<Inner>, id: u64, live: &Arc<Live>, rec: &Record) -> Result<(), Fail> {
    let folder = rec.dir.join(&rec.filename);
    std::fs::create_dir_all(&folder).map_err(|e| Fail::Fatal(format!("could not create folder: {e}")))?;
    let mut files = progress(rec, &folder);
    let publish = |files: &[TorrentFileProgress]| {
        live.downloaded.store(files.iter().map(|f| f.done).sum(), Relaxed);
        live.total_hint.store(files.iter().map(|f| f.len.max(f.done)).sum(), Relaxed);
        *live.files.lock().unwrap() = files.to_vec();
    };
    publish(&files);
    inner.update(id, |r| {
        r.part_started = true;
        r.status = Status::Downloading;
        r.files = files.clone();
    });

    let todo: Vec<usize> = (0..files.len()).filter(|&i| files[i].len == 0 || files[i].done < files[i].len).collect();
    let conns = if todo.len() <= 1 { rec.connections } else { (rec.connections / AT_ONCE.min(todo.len())).max(2) };
    let mut waiting = todo.into_iter();
    let mut running: Vec<(usize, Arc<Live>)> = Vec::new();
    let mut set: JoinSet<(usize, Result<u64, Fail>)> = JoinSet::new();
    let mut failed: Vec<String> = Vec::new();

    loop {
        while running.len() < AT_ONCE {
            let Some(i) = waiting.next() else { break };
            let mut child = Live::new();
            child.cancel = live.cancel.child_token();
            // every file of the bundle counts against the download's own limit
            child.limiter = live.limiter.clone();
            child.set_connection(live.client(), live.route());
            let child = Arc::new(child);
            running.push((i, child.clone()));
            let (file, dest) = (rec.bundle[i].clone(), file_path(&folder, &rec.bundle[i]));
            let parent = live.clone();
            set.spawn(async move { (i, fetch(&file, &dest, conns, &child, &parent).await) });
        }
        if set.is_empty() {
            break;
        }
        tokio::select! {
            done = set.join_next() => {
                let Some(done) = done else { break };
                let (i, res) = done.map_err(|e| Fail::Fatal(format!("internal error: {e}")))?;
                running.retain(|(j, _)| *j != i);
                match res {
                    Ok(len) => {
                        files[i].len = len;
                        files[i].done = len;
                    }
                    Err(Fail::Cancelled) => {
                        files[i].done = 0;
                    }
                    Err(Fail::Fatal(m)) | Err(Fail::Retry(m)) => {
                        files[i].done = 0;
                        failed.push(format!("{}: {m}", rec.bundle[i].path.join("/")));
                    }
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(300)) => {}
        }
        // How far each running file is, and the set as a whole.
        for (i, child) in &running {
            files[*i].done = child.downloaded.load(Relaxed);
            let total = child.total_hint.load(Relaxed);
            if total > 0 {
                files[*i].len = total;
            }
        }
        live.conns.store(running.iter().map(|(_, c)| c.conns.load(Relaxed)).sum(), Relaxed);
        publish(&files);
    }
    live.conns.store(0, Relaxed);
    publish(&files);
    inner.update(id, |r| {
        r.files = files.clone();
        r.total = Some(files.iter().map(|f| f.len).sum());
    });
    if live.cancel.is_cancelled() {
        return Err(Fail::Cancelled);
    }
    match failed.as_slice() {
        [] => Ok(()),
        [one] => Err(Fail::Fatal(one.clone())),
        [first, rest @ ..] => Err(Fail::Fatal(format!("{} of {} files failed. {first}", rest.len() + 1, files.len()))),
    }
}

/// Downloads one file of the set to `dest`; returns its size.
async fn fetch(file: &SiteFile, dest: &Path, conns: usize, child: &Arc<Live>, parent: &Arc<Live>) -> Result<u64, Fail> {
    let mut last = String::new();
    for attempt in 0..TRIES {
        if attempt > 0 {
            tokio::select! {
                _ = child.cancel.cancelled() => return Err(Fail::Cancelled),
                _ = tokio::time::sleep(Duration::from_secs(2 << attempt)) => {}
            }
        }
        child.downloaded.store(0, Relaxed);
        match fetch_once(file, dest, conns, child, parent).await {
            Ok(n) => return Ok(n),
            Err(Fail::Retry(m)) => last = m,
            Err(other) => return Err(other),
        }
    }
    Err(Fail::Retry(last))
}

async fn fetch_once(file: &SiteFile, dest: &Path, conns: usize, child: &Arc<Live>, parent: &Arc<Live>) -> Result<u64, Fail> {
    let (url, headers) = tokio::select! {
        _ = child.cancel.cancelled() => return Err(Fail::Cancelled),
        d = sites::direct(child.client(), file) => d.map_err(|m| {
            if m.contains("too long") || m.contains("could not reach") || m.contains("busy") { Fail::Retry(m) } else { Fail::Fatal(m) }
        })?,
    };
    let client = child.client();
    let probe = tokio::select! {
        _ = child.cancel.cancelled() => return Err(Fail::Cancelled),
        p = probe::probe(&client, &url, &headers) => p?,
    };
    if probe.is_html {
        return Err(Fail::Fatal("the site sent a web page instead of the file; it may need a sign-in".into()));
    }
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir).map_err(|e| Fail::Fatal(format!("could not create folder: {e}")))?;
    }
    let part = part_of(dest);
    let _ = std::fs::remove_file(&part);
    let total = probe.total.filter(|t| *t > 0);
    if let Some(t) = total {
        child.total_hint.store(t, Relaxed);
    }
    let size = match (probe.resumable, total) {
        (true, Some(total)) => {
            drop(probe);
            fetch_stream(child, &url, &headers, &part, total, conns, 0, None).await?;
            total
        }
        _ => stream(probe.body, &url, &headers, &part, total, child, parent).await?,
    };
    if let Some((key, nonce)) = sites::mega_key(file) {
        let path = part.clone();
        tokio::task::spawn_blocking(move || sites::mega_decrypt(&path, key, nonce))
            .await
            .map_err(|e| Fail::Fatal(format!("internal error: {e}")))?
            .map_err(|e| Fail::Fatal(format!("could not decrypt the MEGA file: {e}")))?;
    }
    let _ = std::fs::remove_file(dest);
    std::fs::rename(&part, dest).map_err(|e| Fail::Fatal(format!("could not move finished file: {e}")))?;
    Ok(size)
}

/// One plain stream, for a server that does not take ranges.
#[allow(clippy::too_many_arguments)]
async fn stream(
    body: Option<reqwest::Response>,
    url: &str,
    headers: &[(String, String)],
    part: &Path,
    total: Option<u64>,
    child: &Arc<Live>,
    parent: &Arc<Live>,
) -> Result<u64, Fail> {
    let mut resp = match body {
        Some(r) => r,
        None => {
            let r = tokio::select! {
                _ = child.cancel.cancelled() => return Err(Fail::Cancelled),
                r = request(&child.client(), url, headers).send() => r.map_err(net_err)?,
            };
            if !r.status().is_success() {
                return Err(status_fail(r.status()));
            }
            r
        }
    };
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(part)
        .map_err(|e| Fail::Fatal(format!("could not create file: {e}")))?;
    let _conn = ConnGuard::new(&child.conns);
    let _ = parent;
    let mut off = 0u64;
    loop {
        let chunk = tokio::select! {
            _ = child.cancel.cancelled() => return Err(Fail::Cancelled),
            c = resp.chunk() => c.map_err(net_err)?,
        };
        let Some(chunk) = chunk else { break };
        tokio::select! {
            _ = child.cancel.cancelled() => return Err(Fail::Cancelled),
            _ = child.limiter.acquire(chunk.len()) => {}
        }
        write_all_at(&file, &chunk, off).map_err(|e| Fail::Fatal(format!("could not write to disk: {e}")))?;
        off += chunk.len() as u64;
        child.downloaded.store(off, Relaxed);
    }
    if total.is_some_and(|t| t != off) {
        return Err(Fail::Retry("connection closed early".into()));
    }
    file.sync_all().map_err(|e| Fail::Fatal(format!("could not flush file: {e}")))?;
    Ok(off)
}

/// Removes what a set left behind: unfinished pieces always, and the
/// finished files too when asked.
pub(crate) fn remove(rec: &Record, finished_too: bool) {
    let folder = rec.dir.join(&rec.filename);
    for f in &rec.bundle {
        let path = file_path(&folder, f);
        let _ = std::fs::remove_file(part_of(&path));
        if finished_too {
            let _ = std::fs::remove_file(&path);
        }
    }
    // The folder goes too once nothing is left in it.
    let mut dirs: Vec<PathBuf> = rec.bundle.iter().filter_map(|f| file_path(&folder, f).parent().map(Path::to_path_buf)).collect();
    dirs.sort_by_key(|d| std::cmp::Reverse(d.components().count()));
    dirs.dedup();
    for d in dirs.iter().chain([&folder]) {
        if d.starts_with(&folder) {
            let _ = std::fs::remove_dir(d);
        }
    }
}
