//! Checks the site list against the real sites: `cargo run -p rushdownload-engine --example sitecheck -- <list.tsv> [only-ids]`
//!
//! Each line of the list: `id<TAB>site<TAB>mode<TAB>url`, where mode says
//! what is checked:
//! - `files`: the site resolver lists the link's files;
//! - `files+dl`: that, and the smallest listed file (up to 40 MB) is downloaded;
//! - `media`: yt-dlp reads the video or audio page (title, qualities);
//! - `media+dl`: that, and the lowest quality is downloaded;
//! - `list`: the link is a playlist or collection the site backend can count;
//! - `get`: the link is downloaded as a plain file, end to end;
//! - `direct`: the link is a file whose size the server tells;
//! - `scan`: the page holds media links the deep scan finds;
//! - `torrent`: the torrent's name and files are read (nothing downloaded);
//! - `blocked`: the app says the site cannot be downloaded from, and why.
//!
//! Prints one result line per site; nothing is kept afterwards.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use rushdownload_engine::{Engine, MediaOptions, NewDownload, Settings, Status, Support};

const MAX_DL: u64 = 40 << 20;

#[tokio::main]
async fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let list = args.next().ok_or("usage: sitecheck <list.tsv> [id,id,...]")?;
    let only: Option<Vec<String>> = args.next().map(|s| s.split(',').map(str::to_string).collect());
    let text = std::fs::read_to_string(&list).map_err(|e| format!("{list}: {e}"))?;
    let root = std::env::temp_dir().join(format!("rush-sitecheck-{}", std::process::id()));
    let dir = root.join("files");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mut settings = Settings::new(dir.clone());
    settings.max_concurrent = 4;
    let engine = Engine::new(root.join("state"), settings)?;

    for line in text.lines().filter(|l| !l.trim().is_empty() && !l.starts_with('#')) {
        let cols: Vec<&str> = line.split('\t').collect();
        let [id, site, mode, url, ..] = cols.as_slice() else {
            println!("?? bad line: {line}");
            continue;
        };
        if only.as_ref().is_some_and(|o| !o.iter().any(|x| x == id)) {
            continue;
        }
        let started = Instant::now();
        let result = tokio::time::timeout(Duration::from_secs(240), check(&engine, mode, url)).await;
        let (ok, detail) = match result {
            Ok(Ok(d)) => (true, d),
            Ok(Err(e)) => (false, e),
            Err(_) => (false, "timed out after 240 s".into()),
        };
        println!(
            "{}\t{}\t{}\t{}\t{:.1}s\t{}",
            id,
            site,
            mode,
            if ok { "PASS" } else { "FAIL" },
            started.elapsed().as_secs_f64(),
            detail.replace(['\n', '\t'], " ")
        );
    }
    let _ = std::fs::remove_dir_all(&root);
    Ok(())
}

async fn check(engine: &Engine, mode: &str, url: &str) -> Result<String, String> {
    match mode {
        "blocked" => match Engine::site_kind(url) {
            Some(k) if k.support == Support::Blocked => Ok(format!("{} blocked: {:?}", k.site, k.why)),
            other => Err(format!("not reported as blocked: {other:?}")),
        },
        "files" | "files+dl" | "files+dl2" => {
            let kind = Engine::site_kind(url).ok_or("no site integration matches this link")?;
            let found = engine.site_files(url).await?;
            let total: u64 = found.files.iter().filter_map(|f| f.size).sum();
            let sized = found.files.iter().filter(|f| f.size.is_some()).count();
            let first: Vec<String> = found.files.iter().take(3).map(|f| f.path.join("/")).collect();
            let mut out = format!(
                "{}: \"{}\" {} file(s), {} sized, {:.1} MB; e.g. {}",
                kind.site,
                found.title,
                found.files.len(),
                sized,
                total as f64 / 1048576.0,
                first.join(", ")
            );
            if mode == "files+dl2" {
                let mut small: Vec<_> = found.files.iter().filter(|f| f.size.is_some_and(|s| s > 0 && s <= MAX_DL)).cloned().collect();
                small.sort_by_key(|f| f.size);
                small.truncate(3);
                let names: Vec<String> = small.iter().map(|f| f.path.join("/")).collect();
                let id = engine.add_files(url, &found.title, small, None)?;
                let (bytes, secs, path) = wait(engine, id, 200).await?;
                let items = engine.items(id);
                let done = items.iter().filter(|i| i.state == rushdownload_engine::ItemState::Done).count();
                out.push_str(&format!("; DOWNLOADED {} files together ({done} listed done) {} bytes in {secs:.1}s: {}", names.len(), bytes.max(file_size(&path)), names.join(", ")));
                let _ = std::fs::remove_dir_all(&path);
            }
            if mode == "files+dl" {
                let pick = found
                    .files
                    .iter()
                    .filter(|f| f.size.is_none_or(|s| s > 0 && s <= MAX_DL))
                    .min_by_key(|f| f.size.unwrap_or(MAX_DL))
                    .cloned()
                    .ok_or("no file small enough to try")?;
                let label = pick.path.join("/");
                let id = engine.add_files(url, &found.title, vec![pick], None)?;
                let (bytes, secs, path) = wait(engine, id, 200).await?;
                let size = file_size(&path);
                if size == 0 {
                    return Err(format!("{out}; downloaded {label} but the file is empty or missing"));
                }
                out.push_str(&format!("; DOWNLOADED {label} {} bytes in {secs:.1}s", bytes.max(size)));
                let _ = std::fs::remove_dir_all(&path).or_else(|_| std::fs::remove_file(&path));
            }
            Ok(out)
        }
        "media" | "media+dl" => {
            let info = engine.media_info(url).await?;
            let mut out = format!(
                "{} \"{}\" {}s heights {:?}",
                info.site.unwrap_or_default(),
                info.title.chars().take(60).collect::<String>(),
                info.duration.unwrap_or(0),
                info.heights.iter().take(5).collect::<Vec<_>>()
            );
            if mode == "media+dl" {
                let low = info.heights.iter().min().copied();
                let id = engine.add(NewDownload {
                    url: url.into(),
                    via_site: true,
                    media: MediaOptions { max_height: low, audio_only: low.is_none() },
                    ..Default::default()
                })?;
                let (bytes, secs, path) = wait(engine, id, 220).await?;
                out.push_str(&format!("; DOWNLOADED {} bytes in {secs:.1}s", bytes.max(file_size(&path))));
                let _ = std::fs::remove_file(&path);
            }
            Ok(out)
        }
        "list" => {
            let info = engine.playlist_info(url).await?;
            Ok(format!("\"{}\" {} videos {} photos{}", info.title, info.count, info.photos, if info.more { "+" } else { "" }))
        }
        "collection" => match engine.collection_of(url).await {
            Some(p) => Ok(format!("collection {} on {}", p.name, p.site)),
            None => Err("not found to be a collection".into()),
        },
        "direct" => match engine.link_size(url, &[]).await {
            Some(n) => Ok(format!("file of {n} bytes")),
            None => Err("the server did not give a file size (page or refused)".into()),
        },
        "torrent" => {
            let t = engine.torrent_preview(url).await?;
            Ok(format!("torrent \"{}\" {} file(s), {} bytes", t.name, t.files.len(), t.total))
        }
        "get" => {
            let id = engine.add(NewDownload { url: url.into(), ..Default::default() })?;
            let (bytes, secs, path) = wait(engine, id, 200).await?;
            let size = file_size(&path);
            let _ = std::fs::remove_file(&path);
            Ok(format!("DOWNLOADED {} bytes in {secs:.1}s", bytes.max(size)))
        }
        "scan" => {
            let found = engine.scan_page(url).await?;
            if found.is_empty() {
                return Err("no media found on the page".into());
            }
            Ok(format!("{} media link(s); e.g. {}", found.len(), found[0].name))
        }
        other => Err(format!("unknown mode {other}")),
    }
}

async fn wait(engine: &Engine, id: u64, limit: u64) -> Result<(u64, f64, PathBuf), String> {
    let started = Instant::now();
    let mut changes = engine.subscribe();
    loop {
        let _ = tokio::time::timeout(Duration::from_secs(1), changes.changed()).await;
        let t = engine.list().into_iter().find(|t| t.id == id).ok_or("download disappeared")?;
        match t.status {
            Status::Completed => return Ok((t.downloaded, started.elapsed().as_secs_f64(), t.path)),
            Status::Failed => {
                engine.remove(id, true);
                return Err(format!("download failed: {}", t.error.unwrap_or_default()));
            }
            _ if started.elapsed().as_secs() > limit => {
                engine.remove(id, true);
                return Err(format!("download too slow: {} of {:?} bytes", t.downloaded, t.total));
            }
            _ => {}
        }
    }
}

fn file_size(path: &std::path::Path) -> u64 {
    if path.is_dir() {
        std::fs::read_dir(path)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| file_size(&e.path()))
            .sum()
    } else {
        std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
    }
}
