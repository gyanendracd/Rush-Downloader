//! Web seeds (BEP 19): web servers that hold a torrent's files, listed in
//! the `.torrent` file under "url-list". Clients such as uTorrent fetch
//! pieces from them alongside the swarm; the torrent library the app uses
//! does not, so this does it beside it.
//!
//! The torrent is cut into lanes, one for each worker: the first for the
//! swarm, which fetches from the start of the torrent, and one for each web
//! seed fetcher, which works through its own lane from its start. A fetcher
//! whose lane is done takes over the back half of the lane with the most
//! left, the way a download split over several connections does. Each lane
//! is one bar in the interface, so every bar shown is someone working.
//! Every piece fetched here is checked against the torrent's own hash
//! before it is written.

use std::fs::{File, OpenOptions};
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use reqwest::Client;
use tokio_util::sync::CancellationToken;

use crate::limiter::Limiter;
use crate::task::{write_all_at, ConnGuard, Live};

/// What a web seed's bytes are counted against: the torrent engine's limit,
/// shared with its peers, or (once the engine has let go) the download's.
#[derive(Clone)]
pub(crate) enum Gate {
    Session(Arc<librqbit::Session>),
    Limiter(Arc<Limiter>),
}

impl Gate {
    async fn take(&self, n: usize) {
        match self {
            Gate::Session(session) => crate::torrent::take(session, n).await,
            Gate::Limiter(limiter) => limiter.acquire(n).await,
        }
    }
}

/* ---------- reading the .torrent file ---------- */

enum Value<'a> {
    Int(i64),
    Bytes(&'a [u8]),
    List(Vec<Value<'a>>),
    Dict(Vec<(&'a [u8], Value<'a>)>),
}

/// Reads one bencoded value. Malformed or absurdly nested input gives `None`.
fn parse<'a>(data: &'a [u8], at: &mut usize, depth: usize) -> Option<Value<'a>> {
    if depth > 64 {
        return None;
    }
    match *data.get(*at)? {
        b'i' => {
            let end = *at + data[*at..].iter().position(|&c| c == b'e')?;
            let n = std::str::from_utf8(&data[*at + 1..end]).ok()?.parse().ok()?;
            *at = end + 1;
            Some(Value::Int(n))
        }
        b'l' => {
            *at += 1;
            let mut items = Vec::new();
            while *data.get(*at)? != b'e' {
                items.push(parse(data, at, depth + 1)?);
            }
            *at += 1;
            Some(Value::List(items))
        }
        b'd' => {
            *at += 1;
            let mut items = Vec::new();
            while *data.get(*at)? != b'e' {
                let Value::Bytes(key) = parse(data, at, depth + 1)? else { return None };
                let value = parse(data, at, depth + 1)?;
                items.push((key, value));
            }
            *at += 1;
            Some(Value::Dict(items))
        }
        b'0'..=b'9' => {
            let colon = *at + data[*at..].iter().position(|&c| c == b':')?;
            let len: usize = std::str::from_utf8(&data[*at..colon]).ok()?.parse().ok()?;
            let start = colon + 1;
            let bytes = data.get(start..start.checked_add(len)?)?;
            *at = start + len;
            Some(Value::Bytes(bytes))
        }
        _ => None,
    }
}

impl<'a> Value<'a> {
    fn get(&self, key: &str) -> Option<&Value<'a>> {
        match self {
            Value::Dict(items) => items.iter().find(|(k, _)| *k == key.as_bytes()).map(|(_, v)| v),
            _ => None,
        }
    }
    fn bytes(&self) -> Option<&'a [u8]> {
        match self {
            Value::Bytes(b) => Some(b),
            _ => None,
        }
    }
    fn text(&self) -> Option<&'a str> {
        std::str::from_utf8(self.bytes()?).ok()
    }
    fn int(&self) -> Option<i64> {
        match self {
            Value::Int(n) => Some(*n),
            _ => None,
        }
    }
    fn list(&self) -> Option<&[Value<'a>]> {
        match self {
            Value::List(items) => Some(items),
            _ => None,
        }
    }
}

/// Characters left as they are in a file's address on a web seed.
const PATH_PART: &AsciiSet = &NON_ALPHANUMERIC.remove(b'-').remove(b'_').remove(b'.').remove(b'~');

/// A file or folder name that stays inside the torrent's own folder.
fn safe_part(part: &str) -> bool {
    !part.is_empty() && part != "." && part != ".." && !part.contains(['/', '\\', ':', '\0'])
}

/// One file of the torrent.
#[derive(Debug, PartialEq)]
pub(crate) struct Span {
    /// Where the torrent library keeps it, inside the torrent's folder.
    disk: PathBuf,
    /// Its address below a web seed's, already encoded.
    url_path: String,
    /// Where it starts in the torrent's bytes, all files laid end to end.
    offset: u64,
    len: u64,
    /// Padding (BEP 47): zeros that no web server holds.
    padding: bool,
}

/// What it takes to fetch a torrent's pieces from its web seeds.
#[derive(Debug)]
pub(crate) struct WebSeeds {
    mirrors: Vec<String>,
    multi_file: bool,
    piece_len: u64,
    total: u64,
    hashes: Vec<[u8; 20]>,
    files: Vec<Span>,
}

impl WebSeeds {
    /// The web seeds a `.torrent` file lists, or `None` when it lists none
    /// (or anything about it is not as it should be).
    pub(crate) fn read(torrent: &[u8]) -> Option<WebSeeds> {
        let top = parse(torrent, &mut 0, 0)?;
        let listed: Vec<&str> = match top.get("url-list")? {
            Value::List(items) => items.iter().filter_map(Value::text).collect(),
            single => vec![single.text()?],
        };
        let mut mirrors: Vec<String> = Vec::new();
        for text in listed {
            let text = text.trim();
            let fine = text.len() <= 2000
                && reqwest::Url::parse(text).is_ok_and(|u| matches!(u.scheme(), "http" | "https") && u.host_str().is_some());
            if fine && !mirrors.iter().any(|m| m == text) && mirrors.len() < 32 {
                mirrors.push(text.to_string());
            }
        }
        if mirrors.is_empty() {
            return None;
        }

        let info = top.get("info")?;
        let piece_len = u64::try_from(info.get("piece length")?.int()?).ok().filter(|l| (1..=64 << 20).contains(l))?;
        let pieces = info.get("pieces")?.bytes()?;
        if pieces.len() % 20 != 0 {
            return None;
        }
        let name = info.get("name")?.text().filter(|n| safe_part(n))?;
        let encode = |part: &str| utf8_percent_encode(part, PATH_PART).to_string();

        let mut files = Vec::new();
        let mut offset = 0u64;
        let multi_file = info.get("files").is_some();
        if let Some(listed) = info.get("files") {
            for file in listed.list()? {
                let len = u64::try_from(file.get("length")?.int()?).ok()?;
                let path: Vec<&str> = file.get("path")?.list()?.iter().map(Value::text).collect::<Option<_>>()?;
                if path.is_empty() || !path.iter().all(|p| safe_part(p)) {
                    return None;
                }
                let padding = file.get("attr").and_then(Value::bytes).is_some_and(|a| a.contains(&b'p'));
                files.push(Span {
                    disk: path.iter().collect(),
                    url_path: std::iter::once(name).chain(path.iter().copied()).map(encode).collect::<Vec<_>>().join("/"),
                    offset,
                    len,
                    padding,
                });
                offset = offset.checked_add(len)?;
            }
        } else {
            let len = u64::try_from(info.get("length")?.int()?).ok()?;
            files.push(Span { disk: PathBuf::from(name), url_path: encode(name), offset: 0, len, padding: false });
            offset = len;
        }
        let total = offset;
        if total == 0 || total.div_ceil(piece_len) != (pieces.len() / 20) as u64 {
            return None;
        }
        let hashes = pieces.chunks_exact(20).map(|h| h.try_into().expect("20 bytes")).collect();
        Some(WebSeeds { mirrors, multi_file, piece_len, total, hashes, files })
    }

    pub(crate) fn piece_count(&self) -> usize {
        self.hashes.len()
    }

    fn piece_range(&self, piece: usize) -> (u64, u64) {
        let start = piece as u64 * self.piece_len;
        (start, (start + self.piece_len).min(self.total))
    }

    /// The parts of each file a stretch of the torrent covers: the file's
    /// place in the list, and where the stretch starts and ends inside it.
    fn parts(&self, start: u64, end: u64) -> impl Iterator<Item = (usize, &Span, u64, u64)> {
        self.files.iter().enumerate().filter_map(move |(index, file)| {
            let from = start.max(file.offset);
            let to = end.min(file.offset + file.len);
            (from < to).then(|| (index, file, from - file.offset, to - file.offset))
        })
    }

    /// A file's address on one web seed (BEP 19).
    fn url_for(&self, mirror: usize, file: &Span) -> String {
        let base = &self.mirrors[mirror];
        if self.multi_file {
            let slash = if base.ends_with('/') { "" } else { "/" };
            format!("{base}{slash}{}", file.url_path)
        } else if base.ends_with('/') {
            format!("{base}{}", file.url_path)
        } else {
            base.clone()
        }
    }
}

/* ---------- sharing the work with the swarm ---------- */

#[derive(Default)]
struct Mirror {
    failures: u32,
    dead: bool,
}

/// A stretch of the torrent's pieces and who is working through it.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Lane {
    start: usize,
    end: usize,
    /// The fetcher working on it; `None` for the swarm's lane, or a lane its
    /// fetcher has finished.
    worker: Option<usize>,
}

#[derive(Default)]
struct Progress {
    /// Pieces the torrent library has (checked by it).
    swarm: Vec<bool>,
    /// Pieces fetched here, checked and written.
    ours: Vec<bool>,
    /// Pieces being fetched here right now.
    busy: Vec<bool>,
    mirrors: Vec<Mirror>,
    turn: usize,
    lanes: Vec<Lane>,
}

/// One torrent's web-seed work, shared by its fetchers and the loop that
/// watches the torrent.
pub(crate) struct Plan {
    seeds: WebSeeds,
    folder: PathBuf,
    /// Files chosen for download, by their place in the torrent.
    selected: Vec<bool>,
    /// Pieces holding any part of a chosen file.
    needed: Vec<bool>,
    progress: Mutex<Progress>,
    /// Bytes received from the web seeds, checked or not, for the speed shown.
    received: AtomicU64,
}

/// A web seed is given up after this many failures in a row.
const MIRROR_TRIES: u32 = 3;

impl Plan {
    /// `only_files`: the files chosen, by their place in the torrent; `None` for all.
    pub(crate) fn new(seeds: WebSeeds, folder: PathBuf, only_files: Option<&[usize]>) -> Plan {
        let selected: Vec<bool> = (0..seeds.files.len())
            .map(|i| only_files.is_none_or(|chosen| chosen.contains(&i)))
            .collect();
        let needed = (0..seeds.piece_count())
            .map(|piece| {
                let (start, end) = seeds.piece_range(piece);
                seeds.parts(start, end).any(|(index, file, _, _)| selected[index] && !file.padding)
            })
            .collect();
        let pieces = seeds.piece_count();
        let progress = Progress {
            swarm: vec![false; pieces],
            ours: vec![false; pieces],
            busy: vec![false; pieces],
            mirrors: seeds.mirrors.iter().map(|_| Mirror::default()).collect(),
            turn: 0,
            lanes: Vec::new(),
        };
        Plan { seeds, folder, selected, needed, progress: Mutex::new(progress), received: AtomicU64::new(0) }
    }

    /// Bytes received from the web seeds so far.
    pub(crate) fn received(&self) -> u64 {
        self.received.load(Relaxed)
    }

    pub(crate) fn piece_count(&self) -> usize {
        self.seeds.piece_count()
    }

    pub(crate) fn mirror_count(&self) -> usize {
        self.seeds.mirrors.len()
    }

    /// What the torrent library has, as it says.
    pub(crate) fn set_swarm(&self, have: impl Fn(usize) -> bool) {
        let mut p = self.progress.lock().unwrap();
        for (piece, slot) in p.swarm.iter_mut().enumerate() {
            *slot = have(piece);
        }
    }

    /// Bytes on disk from here that the torrent library does not count.
    pub(crate) fn extra_bytes(&self) -> u64 {
        let p = self.progress.lock().unwrap();
        (0..self.piece_count())
            .filter(|&i| p.ours[i] && !p.swarm[i])
            .map(|i| {
                let (start, end) = self.seeds.piece_range(i);
                end - start
            })
            .sum()
    }

    /// Every chosen piece is on disk.
    pub(crate) fn complete(&self) -> bool {
        let p = self.progress.lock().unwrap();
        (0..self.piece_count()).all(|i| !self.needed[i] || p.swarm[i] || p.ours[i])
    }

    /// Some web seed still answers.
    fn alive(&self) -> bool {
        self.progress.lock().unwrap().mirrors.iter().any(|m| !m.dead)
    }

    /// Cuts the torrent into `workers` + 1 equal lanes: the first for the
    /// swarm, one for each fetcher (numbered from 0).
    pub(crate) fn assign(&self, workers: usize) {
        let count = self.piece_count();
        let parts = (workers + 1).min(count).max(1);
        let lanes = (0..parts)
            .map(|k| Lane {
                start: k * count / parts,
                end: (k + 1) * count / parts,
                worker: k.checked_sub(1),
            })
            .filter(|l| l.start < l.end)
            .collect();
        self.progress.lock().unwrap().lanes = lanes;
    }

    /// Each lane, in order, as (first piece, first piece + pieces done, end):
    /// one bar each in the interface.
    pub(crate) fn lanes(&self) -> Vec<(usize, usize, usize)> {
        let p = self.progress.lock().unwrap();
        let mut lanes = p.lanes.clone();
        lanes.sort_by_key(|l| l.start);
        lanes
            .iter()
            .map(|l| {
                let done = (l.start..l.end).filter(|&i| p.swarm[i] || p.ours[i] || !self.needed[i]).count();
                (l.start, l.start + done, l.end)
            })
            .collect()
    }

    /// The next piece for fetcher `worker`, and the web seed to fetch it from:
    /// the first one left in its own lane, or else one from the back half of
    /// the lane with the most left, which becomes its lane.
    fn claim(&self, worker: usize) -> Option<(usize, usize)> {
        let mut p = self.progress.lock().unwrap();
        let count = p.mirrors.len();
        let mirror = (0..count).map(|k| (p.turn + k) % count).find(|&m| !p.mirrors[m].dead)?;
        let open = |p: &Progress, i: usize| self.needed[i] && !p.swarm[i] && !p.ours[i] && !p.busy[i];

        let own = p.lanes.iter().position(|l| l.worker == Some(worker));
        let mut piece = own.and_then(|k| (p.lanes[k].start..p.lanes[k].end).find(|&i| open(&p, i)));
        if piece.is_none() {
            // Its lane is done (or it had none): it is let go, and the lane
            // with the most left is split. A fetcher's lane is split only
            // while it has two pieces left, so its own fetcher keeps one.
            if let Some(k) = own {
                p.lanes[k].worker = None;
            }
            let best = p
                .lanes
                .iter()
                .enumerate()
                .map(|(k, l)| (k, (l.start..l.end).filter(|&i| open(&p, i)).collect::<Vec<_>>()))
                .filter(|(k, left)| !left.is_empty() && (p.lanes[*k].worker.is_none() || left.len() >= 2))
                .max_by_key(|(_, left)| left.len());
            if let Some((k, left)) = best {
                let from = left[left.len() / 2];
                let end = p.lanes[k].end;
                p.lanes[k].end = from;
                p.lanes.push(Lane { start: from, end, worker: Some(worker) });
                p.lanes.retain(|l| l.start < l.end);
                piece = Some(from);
            }
        }
        let piece = piece?;
        p.turn = mirror + 1;
        p.busy[piece] = true;
        Some((piece, mirror))
    }

    fn release(&self, piece: usize, mirror: usize, fetched: bool) {
        let mut p = self.progress.lock().unwrap();
        p.busy[piece] = false;
        if fetched {
            p.ours[piece] = true;
            p.mirrors[mirror].failures = 0;
        } else {
            let m = &mut p.mirrors[mirror];
            m.failures += 1;
            m.dead = m.failures >= MIRROR_TRIES;
        }
    }

    /// Pieces fetched here that the torrent library never confirmed.
    fn ours_only(&self) -> Vec<usize> {
        let p = self.progress.lock().unwrap();
        (0..self.piece_count()).filter(|&i| p.ours[i] && !p.swarm[i]).collect()
    }

    /// Fetches one piece from one web seed and checks it against its hash.
    async fn fetch(&self, client: &Client, gate: &Gate, piece: usize, mirror: usize) -> Result<Vec<u8>, String> {
        let (start, end) = self.seeds.piece_range(piece);
        let mut data = Vec::with_capacity((end - start) as usize);
        for (_, file, from, to) in self.seeds.parts(start, end) {
            let want = (to - from) as usize;
            if file.padding {
                data.resize(data.len() + want, 0);
                continue;
            }
            let mut answer = client
                .get(self.seeds.url_for(mirror, file))
                .header(reqwest::header::RANGE, format!("bytes={}-{}", from, to - 1))
                .send()
                .await
                .map_err(|e| e.to_string())?;
            match answer.status().as_u16() {
                206 => {
                    // The part sent must be the part asked for.
                    let range = answer
                        .headers()
                        .get(reqwest::header::CONTENT_RANGE)
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or_default();
                    if !range.starts_with(&format!("bytes {from}-")) {
                        return Err(format!("the web seed sent another part ({range})"));
                    }
                }
                // A server that ignores ranges is only of use for a whole small file.
                200 if from == 0 && to == file.len => {}
                status => return Err(format!("the web seed answered {status}")),
            }
            let mut got = 0;
            while let Some(chunk) = answer.chunk().await.map_err(|e| e.to_string())? {
                got += chunk.len();
                if got > want {
                    return Err("the web seed sent more than was asked".into());
                }
                self.received.fetch_add(chunk.len() as u64, Relaxed);
                gate.take(chunk.len()).await;
                data.extend_from_slice(&chunk);
            }
            if got != want {
                return Err("the web seed stopped early".into());
            }
        }
        let sum = ring::digest::digest(&ring::digest::SHA1_FOR_LEGACY_USE_ONLY, &data);
        if sum.as_ref() != self.seeds.hashes[piece] {
            return Err("the piece from the web seed did not match the torrent".into());
        }
        Ok(data)
    }

    /// Writes a checked piece into the chosen files it belongs to.
    fn write(&self, piece: usize, data: &[u8]) -> io::Result<()> {
        let (start, end) = self.seeds.piece_range(piece);
        let mut at = 0usize;
        for (index, file, from, to) in self.seeds.parts(start, end) {
            let len = (to - from) as usize;
            if self.selected[index] && !file.padding {
                let path = self.folder.join(&file.disk);
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let f = OpenOptions::new().write(true).create(true).truncate(false).open(&path)?;
                write_all_at(&f, &data[at..at + len], from)?;
            }
            at += len;
        }
        Ok(())
    }

    /// Whether a piece written here is still as it should be on disk.
    /// Only a piece wholly inside chosen files can be read back.
    fn intact_on_disk(&self, piece: usize) -> bool {
        let (start, end) = self.seeds.piece_range(piece);
        let mut data = Vec::with_capacity((end - start) as usize);
        for (index, file, from, to) in self.seeds.parts(start, end) {
            let len = (to - from) as usize;
            if file.padding {
                data.resize(data.len() + len, 0);
                continue;
            }
            if !self.selected[index] {
                return false;
            }
            let mut part = vec![0u8; len];
            let read = File::open(self.folder.join(&file.disk)).and_then(|f| read_exact_at(&f, &mut part, from));
            if read.is_err() {
                return false;
            }
            data.extend_from_slice(&part);
        }
        ring::digest::digest(&ring::digest::SHA1_FOR_LEGACY_USE_ONLY, &data).as_ref() == self.seeds.hashes[piece]
    }
}

#[cfg(windows)]
pub(crate) fn read_exact_at(f: &File, mut buf: &mut [u8], mut off: u64) -> io::Result<()> {
    use std::os::windows::fs::FileExt;
    while !buf.is_empty() {
        let n = f.seek_read(buf, off)?;
        if n == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        buf = &mut buf[n..];
        off += n as u64;
    }
    Ok(())
}

#[cfg(unix)]
pub(crate) fn read_exact_at(f: &File, buf: &mut [u8], off: u64) -> io::Result<()> {
    use std::os::unix::fs::FileExt;
    f.read_exact_at(buf, off)
}

/// One fetcher (`worker`, from 0): works through its lane, then helps with
/// others, until every chosen piece is on disk, the web seeds all stop
/// answering, or it is stopped.
pub(crate) async fn fetcher(
    plan: Arc<Plan>,
    worker: usize,
    client: Client,
    gate: Gate,
    live: Arc<Live>,
    stop: CancellationToken,
) {
    loop {
        if stop.is_cancelled() || plan.complete() || !plan.alive() {
            return;
        }
        let Some((piece, mirror)) = plan.claim(worker) else {
            // Everything left is being fetched by someone; look again shortly.
            tokio::select! {
                _ = stop.cancelled() => return,
                _ = tokio::time::sleep(Duration::from_secs(1)) => continue,
            }
        };
        let fetched = {
            let _conn = ConnGuard::new(&live.conns);
            tokio::select! {
                _ = stop.cancelled() => None,
                r = plan.fetch(&client, &gate, piece, mirror) => Some(r),
            }
        };
        let Some(fetched) = fetched else {
            plan.release(piece, mirror, false);
            return;
        };
        let written = match fetched {
            Ok(data) => {
                let writer = plan.clone();
                tokio::task::spawn_blocking(move || writer.write(piece, &data))
                    .await
                    .map_err(|e| e.to_string())
                    .and_then(|r| r.map_err(|e| e.to_string()))
            }
            Err(e) => Err(e),
        };
        plan.release(piece, mirror, written.is_ok());
        if written.is_err() {
            // A web seed that fails is tried again after the others.
            tokio::select! {
                _ = stop.cancelled() => return,
                _ = tokio::time::sleep(Duration::from_millis(500)) => {}
            }
        }
    }
}

/// After the torrent library has let go of the files: checks again every
/// piece only fetched here (a peer's bad data may have been written over
/// one before the library threw it away) and fetches again any that are
/// not right. A piece shared with a file not chosen is always fetched again.
pub(crate) async fn settle(plan: Arc<Plan>, client: Client, gate: Gate, stop: CancellationToken) -> Result<(), String> {
    for piece in plan.ours_only() {
        let checker = plan.clone();
        let intact = tokio::task::spawn_blocking(move || checker.intact_on_disk(piece)).await.unwrap_or(false);
        if intact {
            continue;
        }
        let mut last = String::from("no web seed answered");
        let mut done = false;
        for mirror in (0..plan.mirror_count()).cycle().take(plan.mirror_count() * 2) {
            let fetched = tokio::select! {
                _ = stop.cancelled() => return Err("cancelled".into()),
                r = plan.fetch(&client, &gate, piece, mirror) => r,
            };
            match fetched {
                Ok(data) => {
                    let writer = plan.clone();
                    tokio::task::spawn_blocking(move || writer.write(piece, &data))
                        .await
                        .map_err(|e| e.to_string())?
                        .map_err(|e| format!("could not write the file: {e}"))?;
                    done = true;
                    break;
                }
                Err(e) => last = e,
            }
        }
        if !done {
            return Err(format!("a piece could not be fetched again: {last}"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(text: &str) -> String {
        format!("{}:{}", text.len(), text)
    }

    /// A torrent of the given files ("name", length), cut into pieces of
    /// `piece_len`, with the content `content` (all files end to end).
    fn torrent(single: bool, files: &[(&str, usize)], piece_len: usize, content: &[u8], seeds: &[&str]) -> Vec<u8> {
        let mut out = String::from("d8:url-listl");
        for s in seeds {
            out.push_str(&bytes(s));
        }
        out.push_str("e4:info");
        let hashes: Vec<u8> = content
            .chunks(piece_len)
            .flat_map(|c| ring::digest::digest(&ring::digest::SHA1_FOR_LEGACY_USE_ONLY, c).as_ref().to_vec())
            .collect();
        let mut info = String::from("d");
        if single {
            info.push_str(&format!("6:lengthi{}e", files[0].1));
            info.push_str(&format!("4:name{}", bytes(files[0].0)));
        } else {
            info.push_str("5:filesl");
            for (name, len) in files {
                let attr = if name.starts_with(".pad") { "4:attr1:p" } else { "" };
                info.push_str(&format!("d{attr}6:lengthi{len}e4:pathl{}ee", bytes(name)));
            }
            info.push_str("e4:name4:pack");
        }
        info.push_str(&format!("12:piece lengthi{piece_len}e6:pieces{}:", hashes.len()));
        let mut raw = out.into_bytes();
        raw.extend_from_slice(info.as_bytes());
        raw.extend_from_slice(&hashes);
        raw.extend_from_slice(b"ee");
        raw
    }

    #[test]
    fn web_seeds_and_layout_are_read_from_the_torrent() {
        let content = vec![7u8; 10];
        let raw = torrent(true, &[("big file.bin", 10)], 4, &content, &["https://a.test/files/big%20file.bin", "https://b.test/dir/", "ftp://c.test/x"]);
        let seeds = WebSeeds::read(&raw).unwrap();
        assert_eq!(seeds.mirrors.len(), 2, "only web addresses");
        assert_eq!(seeds.piece_count(), 3);
        assert_eq!(seeds.piece_range(2), (8, 10), "the last piece is shorter");
        let file = &seeds.files[0];
        assert_eq!(seeds.url_for(0, file), "https://a.test/files/big%20file.bin", "a single file's address is the seed's own");
        assert_eq!(seeds.url_for(1, file), "https://b.test/dir/big%20file.bin", "a folder address gets the name");

        let content: Vec<u8> = (0..12u8).collect();
        let raw = torrent(false, &[("a.txt", 5), (".pad1", 3), ("b c.txt", 4)], 4, &content, &["https://m.test/t"]);
        let seeds = WebSeeds::read(&raw).unwrap();
        assert_eq!(seeds.url_for(0, &seeds.files[2]), "https://m.test/t/pack/b%20c.txt");
        assert_eq!(seeds.files[2].disk, PathBuf::from("b c.txt"), "on disk without the torrent's name, as the library keeps it");
        assert!(seeds.files[1].padding);
        let parts: Vec<_> = seeds.parts(4, 8).map(|(i, _, from, to)| (i, from, to)).collect();
        assert_eq!(parts, vec![(0, 4, 5), (1, 0, 3)], "a piece across files");
    }

    #[test]
    fn torrents_without_web_seeds_or_with_unsafe_names_get_none() {
        let content = vec![1u8; 8];
        assert!(WebSeeds::read(&torrent(true, &[("x", 8)], 4, &content, &[])).is_none());
        assert!(WebSeeds::read(&torrent(false, &[("..", 8)], 4, &content, &["https://m.test/"])).is_none());
        assert!(WebSeeds::read(&torrent(true, &[("..", 8)], 4, &content, &["https://m.test/"])).is_none());
        assert!(WebSeeds::read(b"d8:url-list5:https4:infodee").is_none());
        assert!(WebSeeds::read(b"not a torrent").is_none());
        assert!(WebSeeds::read(b"llllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllll").is_none());
    }

    #[test]
    fn each_fetcher_works_through_its_own_lane_and_skips_what_the_swarm_has() {
        let content = vec![0u8; 16];
        let raw = torrent(false, &[("a", 8), ("b", 8)], 4, &content, &["https://m.test/", "https://n.test/"]);
        let plan = Plan::new(WebSeeds::read(&raw).unwrap(), PathBuf::from("."), Some(&[0]));
        assert_eq!(plan.needed, vec![true, true, false, false], "only the chosen file's pieces");
        plan.assign(1);
        assert_eq!(plan.lanes(), vec![(0, 0, 2), (2, 4, 4)], "the swarm's lane, then the fetcher's (not needed: done)");
        plan.set_swarm(|i| i == 0);
        // its own lane has nothing left, so it takes over from the swarm's
        let (piece, first) = plan.claim(0).unwrap();
        assert_eq!(piece, 1, "past what the swarm has");
        assert!(plan.claim(0).is_none(), "nothing else is needed");
        plan.release(piece, first, true);
        assert!(plan.complete());
        assert_eq!(plan.extra_bytes(), 4);
        assert_eq!(plan.ours_only(), vec![1]);

        // A web seed that keeps failing is given up.
        let plan = Plan::new(WebSeeds::read(&raw).unwrap(), PathBuf::from("."), None);
        plan.assign(2);
        for _ in 0..MIRROR_TRIES * 2 {
            let (piece, mirror) = plan.claim(0).unwrap();
            plan.release(piece, mirror, false);
        }
        assert!(!plan.alive());
        assert!(plan.claim(0).is_none());
    }

    #[test]
    fn lanes_fill_side_by_side_and_a_finished_fetcher_helps_the_busiest() {
        let content = vec![0u8; 48];
        let raw = torrent(true, &[("big", 48)], 4, &content, &["https://m.test/big"]);
        let plan = Plan::new(WebSeeds::read(&raw).unwrap(), PathBuf::from("."), None);
        plan.assign(2);
        // 12 pieces: the swarm's lane 0..4, fetcher 0 has 4..8, fetcher 1 has 8..12
        assert_eq!(plan.lanes(), vec![(0, 0, 4), (4, 4, 8), (8, 8, 12)]);
        let (a, m) = plan.claim(0).unwrap();
        let (b, n) = plan.claim(1).unwrap();
        assert_eq!((a, b), (4, 8), "each starts at the front of its own lane");
        plan.release(a, m, true);
        plan.release(b, n, true);
        assert_eq!(plan.lanes(), vec![(0, 0, 4), (4, 5, 8), (8, 9, 12)], "both bars move");

        // fetcher 0 finishes its lane
        for _ in 0..3 {
            let (piece, mirror) = plan.claim(0).unwrap();
            assert!((5..8).contains(&piece));
            plan.release(piece, mirror, true);
        }
        // then takes the back half of the lane with the most left: the swarm's (4 left)
        let (piece, mirror) = plan.claim(0).unwrap();
        assert_eq!(piece, 2);
        plan.release(piece, mirror, true);
        let lanes = plan.lanes();
        assert!(lanes.contains(&(0, 0, 2)), "the swarm keeps the front half: {lanes:?}");
        assert!(lanes.contains(&(2, 3, 4)), "the fetcher has the back half: {lanes:?}");
        assert_eq!(lanes.iter().map(|l| l.2 - l.0).sum::<usize>(), 12, "the lanes still cover the torrent");
    }

    /// Needs internet. Set RUSH_TEST_TORRENT to a .torrent file with web seeds:
    /// `cargo test -p rushdownload-engine --lib -- --ignored each_web_seed --nocapture`
    /// Fetches the last piece from each web seed and prints how it went.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn each_web_seed_answers() {
        let path = std::env::var("RUSH_TEST_TORRENT").expect("set RUSH_TEST_TORRENT");
        let plan = Plan::new(WebSeeds::read(&std::fs::read(path).unwrap()).expect("web seeds"), PathBuf::from("."), None);
        let client = reqwest::Client::builder()
            .http1_only()
            .connect_timeout(Duration::from_secs(15))
            .read_timeout(Duration::from_secs(20))
            .build()
            .unwrap();
        let gate = Gate::Limiter(Arc::new(Limiter::new(0)));
        let piece = plan.piece_count() - 2;
        for mirror in 0..plan.mirror_count() {
            let started = std::time::Instant::now();
            let got = plan.fetch(&client, &gate, piece, mirror).await;
            println!(
                "{:6.1}s  {}  {}",
                started.elapsed().as_secs_f64(),
                got.as_ref().map_or_else(|e| e.clone(), |d| format!("ok, {} bytes", d.len())),
                plan.seeds.mirrors[mirror]
            );
        }
    }

    #[test]
    fn checked_pieces_are_written_into_the_chosen_files() {
        let dir = std::env::temp_dir().join(format!("rush-webseed-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let content: Vec<u8> = (0..12u8).collect();
        let raw = torrent(false, &[("a", 6), ("b", 6)], 4, &content, &["https://m.test/"]);
        let plan = Plan::new(WebSeeds::read(&raw).unwrap(), dir.clone(), Some(&[1]));
        plan.write(1, &content[4..8]).unwrap();
        plan.write(2, &content[8..12]).unwrap();
        assert!(!dir.join("a").exists(), "a file not chosen is not made");
        let b = std::fs::read(dir.join("b")).unwrap();
        assert_eq!(b, content[6..12]);
        assert!(plan.intact_on_disk(2));
        assert!(!plan.intact_on_disk(1), "a piece shared with a file not chosen cannot be read back");
        std::fs::write(dir.join("b"), [0u8; 6]).unwrap();
        assert!(!plan.intact_on_disk(2), "a piece written over is noticed");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
