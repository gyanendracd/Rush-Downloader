//! Web-page links (YouTube, Instagram, Vimeo and the many other sites yt-dlp
//! knows) are downloaded by running yt-dlp and translating its output into
//! the same progress model direct downloads use.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

use crate::task::{ConnGuard, Fail, Live};
use crate::{Inner, Record, Status};

/// Finds a helper program on PATH or where winget installs it.
pub(crate) fn find_tool(name: &str) -> Option<PathBuf> {
    let exe = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            let candidate = dir.join(&exe);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    // A freshly installed tool is not on the PATH of already-running shells.
    let winget = PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("Microsoft/WinGet");
    let link = winget.join("Links").join(&exe);
    if link.is_file() {
        return Some(link);
    }
    for pkg in std::fs::read_dir(winget.join("Packages")).ok()?.flatten() {
        if !pkg.file_name().to_string_lossy().to_lowercase().contains(name) {
            continue;
        }
        let direct = pkg.path().join(&exe);
        if direct.is_file() {
            return Some(direct);
        }
        for sub in std::fs::read_dir(pkg.path()).into_iter().flatten().flatten() {
            let nested = sub.path().join("bin").join(&exe);
            if nested.is_file() {
                return Some(nested);
            }
        }
    }
    None
}

pub(crate) fn command(program: &Path) -> Command {
    let mut cmd = Command::new(program);
    cmd.stdin(Stdio::null()).kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    cmd
}

/// Stops yt-dlp together with the processes it started (its own worker
/// process and ffmpeg); killing only the parent leaves them running.
pub(crate) async fn kill_tree(child: &mut Child) {
    #[cfg(windows)]
    if let Some(pid) = child.id() {
        let _ = command(Path::new("taskkill"))
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;
    }
    let _ = child.kill().await;
}

fn number(field: &str) -> Option<u64> {
    field.trim().parse::<f64>().ok().map(|v| v as u64)
}

fn friendly(error: &str) -> String {
    let error = error.trim_start_matches("ERROR:").trim();
    // Sites yt-dlp has no support for end up with its catch-all "generic"
    // reader, whose failures mean nothing to a user. The Browser page can
    // still catch the video while it plays.
    if error.contains("Unsupported URL") || error.contains("[generic]") {
        "This link is a web page with no video RushDownload can download from the link alone. \
         Open the page on the Browser page and play the video to catch it"
            .into()
    } else if error.contains("Sign in to confirm") {
        "This site is asking for a sign-in to confirm you are not a bot. Wait a while and retry, \
         or sign in to the site on the Sign in page"
            .into()
    } else if error.contains("cookie") && (error.contains("Could not copy") || error.contains("decrypt") || error.contains("could not find")) {
        "Could not read the sign-in from the chosen browser. Close that browser completely and \
         retry, or choose Firefox under Settings, Downloads"
            .into()
    } else if !cut_short(error) && could_not_read_page(error) {
        // yt-dlp knows the site but its reader for it no longer matches the
        // site (sites change). What the page plays can still be caught.
        "The site downloader could not read this page; the site has probably changed how it \
         works. Open the page on the Browser page and play the video to catch it"
            .into()
    } else {
        error.to_string()
    }
}

/// yt-dlp reached the site but could not make sense of what it got back.
fn could_not_read_page(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    [
        "unable to download json metadata",
        "unable to extract",
        "http error 400",
        "no video formats found",
        "failed to parse json",
        "unable to download api page",
    ]
    .iter()
    .any(|sign| lower.contains(sign))
}

/// The private folder where one site download keeps its unfinished pieces.
/// Every download has its own, so two copies of the same video never touch
/// each other's files.
pub(crate) fn work_dir(dir: &Path, id: u64) -> PathBuf {
    dir.join(format!(".rush-{id}"))
}

/// Deletes one download's private folder and nothing else.
pub(crate) fn remove_work_dir(dir: &Path, id: u64) {
    let target = work_dir(dir, id);
    if target.parent() == Some(dir) && target.is_dir() {
        let _ = std::fs::remove_dir_all(&target);
    }
}

/// What a video page offers, shown before the download starts.
#[derive(Clone, Debug, Serialize)]
pub struct MediaInfo {
    pub title: String,
    pub thumbnail: Option<String>,
    /// Length in seconds.
    pub duration: Option<u64>,
    /// Picture heights on offer, largest first (2160, 1080, 720...).
    pub heights: Vec<u32>,
    /// The site's name as yt-dlp knows it.
    pub site: Option<String>,
}

fn read_info(info: &serde_json::Value) -> Option<MediaInfo> {
    let title = info.get("title")?.as_str()?.trim().to_string();
    let mut heights: Vec<u32> = info
        .get("formats")
        .and_then(|f| f.as_array())
        .map(|formats| {
            formats
                .iter()
                .filter(|f| f.get("vcodec").and_then(|v| v.as_str()) != Some("none"))
                .filter_map(|f| f.get("height")?.as_u64())
                .map(|h| h as u32)
                .collect()
        })
        .unwrap_or_default();
    if heights.is_empty() {
        heights.extend(info.get("height").and_then(|h| h.as_u64()).map(|h| h as u32));
    }
    heights.retain(|h| *h >= 100);
    heights.sort_unstable_by(|a, b| b.cmp(a));
    heights.dedup();
    let text = |key: &str| {
        info.get(key)
            .and_then(|v| v.as_str())
            .filter(|v| !v.is_empty())
            .map(str::to_string)
    };
    Some(MediaInfo {
        title,
        thumbnail: text("thumbnail").filter(|t| t.starts_with("http")),
        duration: info.get("duration").and_then(|d| d.as_f64()).map(|d| d as u64),
        heights,
        site: text("extractor_key").filter(|s| s != "Generic"),
    })
}

/// Looks up a video page's title, picture and resolutions. Nothing is downloaded.
pub(crate) async fn info(inner: &Arc<Inner>, url: &str) -> Result<MediaInfo, String> {
    let mut tries = 0;
    loop {
        tries += 1;
        match info_once(inner, url).await {
            Err(e) if tries < TRIES && cut_short(&e) => {
                debug(|| format!("the connection was cut during the lookup (try {tries}), trying again"));
                tokio::time::sleep(std::time::Duration::from_millis(700)).await;
            }
            other => return other,
        }
    }
}

/// How many times a site is asked when the connection keeps being cut.
const TRIES: u32 = 5;

/// A failure that came from the connection being cut or stalling on the way,
/// not from the site's answer. Some networks drop a share of the connections
/// to certain sites; the next try often gets through.
fn cut_short(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    [
        "eof occurred",
        "unexpected_eof",
        "connection reset",
        "connection aborted",
        "forcibly closed",
        "10054",
        "remotedisconnected",
        "remote end closed",
        "connection broken",
        "timed out",
        "incompleteread",
    ]
    .iter()
    .any(|sign| lower.contains(sign))
}

async fn info_once(inner: &Arc<Inner>, url: &str) -> Result<MediaInfo, String> {
    let ytdlp = find_tool("yt-dlp").ok_or("yt-dlp was not found on this computer")?;
    let mut cmd = command(&ytdlp);
    cmd.args(["--encoding", "utf-8", "--no-playlist", "--no-colors", "-J"]);
    if let Some(node) = find_tool("node") {
        cmd.arg("--js-runtimes").arg(format!("node:{}", node.display()));
    }
    inner.add_sign_in(&mut cmd, None);
    cmd.arg("--").arg(url).stdout(Stdio::piped()).stderr(Stdio::piped());
    let child = cmd.spawn().map_err(|e| format!("could not start yt-dlp: {e}"))?;
    // Dropping the unfinished call stops yt-dlp (kill_on_drop).
    let out = tokio::time::timeout(std::time::Duration::from_secs(45), child.wait_with_output())
        .await
        .map_err(|_| "the site took too long to answer".to_string())?
        .map_err(|e| format!("yt-dlp failed: {e}"))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(stderr
            .lines()
            .rev()
            .find(|l| l.starts_with("ERROR:"))
            .map(friendly)
            .unwrap_or_else(|| "no video was found on this page".into()));
    }
    serde_json::from_slice::<serde_json::Value>(&out.stdout)
        .ok()
        .as_ref()
        .and_then(read_info)
        .ok_or_else(|| "no video was found on this page".into())
}

/// The name a named site download starts from: the given name without a
/// media ending, with the quality asked for ("Clip 720p").
fn named_base(rec: &Record) -> String {
    let media_ext = ["mp4", "mkv", "webm", "mov", "m4v", "mp3", "m4a", "flv", "avi", "ogg", "opus", "wav"];
    let name = Path::new(&rec.filename);
    let stem = match name.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()) {
        Some(ext) if media_ext.contains(&ext.as_str()) => {
            name.file_stem().unwrap_or_default().to_string_lossy().to_string()
        }
        _ => rec.filename.clone(),
    };
    match rec.media_options.max_height {
        Some(h) if !rec.media_options.audio_only => format!("{stem} {h}p"),
        _ => stem,
    }
}

/// yt-dlp with everything both of its uses share: which video, which
/// quality, what the file is called and how to reach the site.
fn base_command(ytdlp: &Path, inner: &Arc<Inner>, rec: &Record, suffix: &str, work: &Path) -> Command {
    let mut temp = std::ffi::OsString::from("temp:");
    temp.push(work);
    let mut cmd = command(ytdlp);
    cmd.args([
        "--encoding",
        "utf-8",
        "--no-playlist",
        "--no-colors",
        "--windows-filenames",
        // Connections that are cut on the way are tried again.
        "--retries",
        "10",
        "--fragment-retries",
        "10",
        "--extractor-retries",
        "5",
        "--retry-sleep",
        "1",
    ])
    .arg("-P")
    .arg(&rec.dir)
    .arg("-P")
    .arg(temp);
    let opts = rec.media_options;
    // A name given with the download is used as it is; otherwise the file is
    // named after the video's own title.
    // The name was settled when the download first ran (see `run`).
    let named = inner
        .record(rec.id)
        .and_then(|r| r.named_stem)
        // "%" starts a field in yt-dlp's name pattern.
        .map(|stem| format!("{}.%(ext)s", stem.replace('%', "%%")));
    if opts.audio_only {
        cmd.args([
            "-f",
            "ba/b",
            "--extract-audio",
            "--audio-format",
            "mp3",
            "--audio-quality",
            "0",
            "-o",
            &named.clone().unwrap_or_else(|| format!("%(title).120B [%(id)s]{suffix}.%(ext)s")),
        ]);
    } else {
        let format = match opts.max_height {
            // Best at or below the wanted height; if the video only exists
            // in larger sizes, the smallest of those.
            Some(h) => format!("bv*[height<={h}]+ba/b[height<={h}]/wv*+ba/w"),
            None => "bv*+ba/b".to_string(),
        };
        // The resolution is part of the name so the same video can be kept
        // in several sizes.
        cmd.args([
            "-f",
            &format,
            "--merge-output-format",
            "mp4/mkv",
            "-o",
            &named.unwrap_or_else(|| format!("%(title).120B %(height&{{}}p |)s[%(id)s]{suffix}.%(ext)s")),
        ]);
    }
    if let Some(ffmpeg) = find_tool("ffmpeg") {
        cmd.arg("--ffmpeg-location").arg(ffmpeg);
    }
    // Some sites (YouTube) need a JavaScript runtime to list every format.
    if let Some(node) = find_tool("node") {
        cmd.arg("--js-runtimes").arg(format!("node:{}", node.display()));
    }
    // The user's sign-in, if they made one.
    inner.add_sign_in(&mut cmd, Some(work));
    for (k, v) in &rec.headers {
        cmd.arg("--add-headers").arg(format!("{k}:{v}"));
    }
    cmd
}

/// One stream of a video as the site serves it: a plain link to a file.
struct Stream {
    url: String,
    headers: Vec<(String, String)>,
    ext: String,
    /// Largest range to ask for in one request (0: no limit).
    request_cap: u64,
    /// Exact size in bytes, when the site states it.
    size: Option<u64>,
}

/// What yt-dlp found for a link: the streams to fetch and the file's name.
struct Plan {
    streams: Vec<Stream>,
    /// File name without its extension.
    stem: String,
}

/// The streams of a video, if every one of them is a plain file link that
/// the app's own multi-connection downloader can fetch. Videos served in
/// many small pieces (HLS, DASH) return `None` and stay with yt-dlp.
fn direct_streams(info: &serde_json::Value) -> Option<Vec<Stream>> {
    let list: Vec<&serde_json::Value> = match info.get("requested_formats").and_then(|v| v.as_array()) {
        Some(formats) if !formats.is_empty() => formats.iter().collect(),
        _ => vec![info],
    };
    if list.len() > 2 {
        return None;
    }
    list.into_iter()
        .map(|f| {
            let protocol = f.get("protocol")?.as_str()?;
            if protocol != "https" && protocol != "http" {
                return None;
            }
            let url = f.get("url")?.as_str()?;
            if !url.starts_with("http") {
                return None;
            }
            let headers = f
                .get("http_headers")
                .and_then(|h| h.as_object())
                .map(|h| {
                    h.iter()
                        .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string())))
                        .collect()
                })
                .unwrap_or_default();
            Some(Stream {
                url: url.to_string(),
                headers,
                ext: f.get("ext").and_then(|e| e.as_str()).unwrap_or("mp4").to_string(),
                request_cap: f
                    .get("downloader_options")
                    .and_then(|o| o.get("http_chunk_size"))
                    .and_then(|c| c.as_u64())
                    .unwrap_or(0),
                size: f.get("filesize").and_then(|s| s.as_u64()).filter(|s| *s > 0),
            })
        })
        .collect()
}

/// Asks yt-dlp what the link holds, without downloading anything.
/// `Ok(None)`: the video cannot be fetched as plain files.
async fn plan(
    inner: &Arc<Inner>,
    live: &Arc<Live>,
    rec: &Record,
    ytdlp: &Path,
    suffix: &str,
    work: &Path,
) -> Result<Option<Plan>, Fail> {
    let mut tries = 0;
    loop {
        tries += 1;
        match plan_once(inner, live, rec, ytdlp, suffix, work).await {
            Err(Fail::Fatal(e)) if tries < TRIES && cut_short(&e) => {
                debug(|| format!("the connection was cut while asking the site (try {tries}), trying again"));
                tokio::select! {
                    _ = live.cancel.cancelled() => return Err(Fail::Cancelled),
                    _ = tokio::time::sleep(std::time::Duration::from_millis(700)) => {}
                }
            }
            other => return other,
        }
    }
}

async fn plan_once(
    inner: &Arc<Inner>,
    live: &Arc<Live>,
    rec: &Record,
    ytdlp: &Path,
    suffix: &str,
    work: &Path,
) -> Result<Option<Plan>, Fail> {
    let mut cmd = base_command(ytdlp, inner, rec, suffix, work);
    cmd.args(["--print", "OMNINAME|%(filename)s", "--print", "OMNIJSON|%()j"])
        .arg("--")
        .arg(&rec.url)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = cmd
        .spawn()
        .map_err(|e| Fail::Fatal(format!("could not start yt-dlp: {e}")))?;
    // Dropping the unfinished call stops yt-dlp (kill_on_drop).
    let out = tokio::select! {
        _ = live.cancel.cancelled() => return Err(Fail::Cancelled),
        o = child.wait_with_output() => o.map_err(|e| Fail::Fatal(format!("yt-dlp failed: {e}")))?,
    };
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let msg = stderr
            .lines()
            .rev()
            .find(|l| l.starts_with("ERROR:"))
            .map(friendly)
            .unwrap_or_else(|| format!("yt-dlp stopped unexpectedly ({})", out.status));
        return Err(Fail::Fatal(msg));
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut stem = None;
    let mut streams = None;
    for line in stdout.lines() {
        if let Some(path) = line.strip_prefix("OMNINAME|") {
            stem = Path::new(path.trim())
                .file_stem()
                .map(|s| s.to_string_lossy().to_string());
        } else if let Some(json) = line.strip_prefix("OMNIJSON|") {
            let info = serde_json::from_str::<serde_json::Value>(json).ok();
            debug(|| {
                let formats = info
                    .as_ref()
                    .and_then(|i| i.get("requested_formats"))
                    .and_then(|f| f.as_array())
                    .map(|list| {
                        list.iter()
                            .map(|f| {
                                format!(
                                    "{} {} {}",
                                    f.get("format_id").and_then(|v| v.as_str()).unwrap_or("?"),
                                    f.get("ext").and_then(|v| v.as_str()).unwrap_or("?"),
                                    f.get("protocol").and_then(|v| v.as_str()).unwrap_or("?")
                                )
                            })
                            .collect::<Vec<_>>()
                            .join(" + ")
                    });
                format!(
                    "yt-dlp answered ({} bytes): formats {:?}, single protocol {:?}",
                    json.len(),
                    formats,
                    info.as_ref().and_then(|i| i.get("protocol")).and_then(|p| p.as_str())
                )
            });
            streams = info.as_ref().and_then(direct_streams);
            if std::env::var_os("RUSHDOWNLOAD_DEBUG").is_some() {
                for (i, s) in streams.iter().flatten().enumerate() {
                    let _ = std::fs::write(std::env::temp_dir().join(format!("rush-debug-link-{i}.txt")), &s.url);
                }
            }
        }
    }
    Ok(match (streams, stem) {
        (Some(streams), Some(stem)) if !stem.is_empty() => Some(Plan { streams, stem }),
        _ => None,
    })
}

/// Runs ffmpeg to completion; stops it when the download is cancelled.
async fn ffmpeg_run(live: &Arc<Live>, ffmpeg: &Path, args: &[&std::ffi::OsStr]) -> Result<(), Fail> {
    let mut cmd = command(ffmpeg);
    cmd.args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let child = cmd
        .spawn()
        .map_err(|e| Fail::Fatal(format!("could not start ffmpeg: {e}")))?;
    let out = tokio::select! {
        _ = live.cancel.cancelled() => return Err(Fail::Cancelled),
        o = child.wait_with_output() => o.map_err(|e| Fail::Fatal(format!("ffmpeg failed: {e}")))?,
    };
    if out.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    let last = stderr.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("unknown error");
    Err(Fail::Fatal(format!("could not put the video together: {last}")))
}

/// Joins the fetched streams into the finished file at `out`.
pub(crate) async fn assemble(
    live: &Arc<Live>,
    ffmpeg: Option<&Path>,
    inputs: &[PathBuf],
    audio_only: bool,
    out: &Path,
) -> Result<(), Fail> {
    use std::ffi::OsStr;
    if !audio_only && inputs.len() == 1 {
        // One stream that already holds everything: it is the file.
        return std::fs::rename(&inputs[0], out)
            .map_err(|e| Fail::Fatal(format!("could not move finished file: {e}")));
    }
    let ffmpeg = ffmpeg.ok_or_else(|| Fail::Fatal("ffmpeg was not found".into()))?;
    let mut args: Vec<&OsStr> = Vec::new();
    for input in inputs {
        args.push(OsStr::new("-i"));
        args.push(input.as_os_str());
    }
    if audio_only {
        args.extend(["-vn", "-c:a", "libmp3lame", "-q:a", "0"].map(OsStr::new));
    } else {
        // Video and audio are copied as they are, not converted.
        args.extend(["-c", "copy"].map(OsStr::new));
        if out.extension().is_some_and(|e| e == "mp4") {
            args.extend(["-movflags", "+faststart"].map(OsStr::new));
        }
    }
    args.push(out.as_os_str());
    ffmpeg_run(live, ffmpeg, &args).await
}

/// Notes on which way a site download took, printed when
/// RUSHDOWNLOAD_DEBUG is set.
pub(crate) fn debug(note: impl FnOnce() -> String) {
    if std::env::var_os("RUSHDOWNLOAD_DEBUG").is_some() {
        eprintln!("[site] {}", note());
    }
}

/// Waits until a freshly issued stream link is accepted. YouTube turns a new
/// link away with 403 for the first two or three seconds and serves it
/// normally after that.
async fn wait_until_served(inner: &Arc<Inner>, live: &Arc<Live>, stream: &Stream) -> Result<(), Fail> {
    for attempt in 0..30 {
        let req = crate::probe::request(&inner.client, &stream.url, &stream.headers)
            .header(reqwest::header::RANGE, "bytes=0-0");
        let status = tokio::select! {
            _ = live.cancel.cancelled() => return Err(Fail::Cancelled),
            r = req.send() => r.map(|r| r.status()),
        };
        match status {
            Ok(s) if s == reqwest::StatusCode::FORBIDDEN => {}
            Ok(s) if s.is_success() => {
                debug(|| format!("stream .{} is served after {} ms", stream.ext, attempt * 500));
                return Ok(());
            }
            Ok(s) => return Err(Fail::Fatal(format!("the site answered {s}"))),
            Err(e) => return Err(crate::probe::net_err(e)),
        }
        tokio::select! {
            _ = live.cancel.cancelled() => return Err(Fail::Cancelled),
            _ = tokio::time::sleep(std::time::Duration::from_millis(500)) => {}
        }
    }
    Err(Fail::Fatal("the site kept refusing the video link".into()))
}

fn stream_part(work: &Path, i: usize) -> PathBuf {
    work.join(format!("stream-{i}.part"))
}

fn stream_done(work: &Path, i: usize, ext: &str) -> PathBuf {
    work.join(format!("stream-{i}.{ext}"))
}

/// Fetches a video's streams with the app's own multi-connection downloader
/// (every connection takes a different part of the file at the same time)
/// and joins them. Much faster than one connection, where the site allows it.
async fn fetch_direct(
    inner: &Arc<Inner>,
    id: u64,
    live: &Arc<Live>,
    rec: &Record,
    plan: &Plan,
    work: &Path,
    saved_segments: &[[u64; 3]],
) -> Result<(), Fail> {
    let audio_only = rec.media_options.audio_only;
    let ffmpeg = find_tool("ffmpeg");
    if (audio_only || plan.streams.len() > 1) && ffmpeg.is_none() {
        return Err(Fail::Fatal("ffmpeg was not found".into()));
    }
    let ext = if audio_only {
        "mp3".to_string()
    } else if plan.streams.len() == 1 {
        plan.streams[0].ext.clone()
    } else {
        let mp4_family = |e: &str| matches!(e, "mp4" | "m4v" | "m4a" | "aac" | "mov");
        if plan.streams.iter().all(|s| mp4_family(&s.ext)) { "mp4" } else { "mkv" }.to_string()
    };
    let name = format!("{}.{ext}", plan.stem);

    // Every stream must be a file of known size that can be read in ranges.
    let mut sizes = Vec::new();
    for stream in &plan.streams {
        wait_until_served(inner, live, stream).await?;
        // The size the site states is used as it is.
        if let Some(size) = stream.size {
            debug(|| format!("stream .{}: {size} bytes, request cap {}", stream.ext, stream.request_cap));
            sizes.push(size);
            continue;
        }
        let probe = tokio::select! {
            _ = live.cancel.cancelled() => return Err(Fail::Cancelled),
            p = crate::probe::probe(&inner.client, &stream.url, &stream.headers) => p?,
        };
        debug(|| {
            format!(
                "stream .{}: size {:?}, ranges {}, page {}, request cap {}",
                stream.ext, probe.total, probe.resumable, probe.is_html, stream.request_cap
            )
        });
        match probe.total {
            Some(total) if probe.resumable && !probe.is_html && total > 0 => sizes.push(total),
            _ => return Err(Fail::Fatal("the site does not serve this video in ranges".into())),
        }
    }
    let total: u64 = sizes.iter().sum();
    live.downloaded.store(0, Relaxed);
    live.total_hint.store(total, Relaxed);
    inner.update(id, |r| {
        r.filename = name.clone();
        r.total = Some(total);
        r.status = Status::Downloading;
    });

    let mut finished = 0u64;
    let mut inputs = Vec::new();
    let mut may_resume = true;
    for (i, stream) in plan.streams.iter().enumerate() {
        let part = stream_part(work, i);
        let done = stream_done(work, i, &stream.ext);
        let size = sizes[i];
        if std::fs::metadata(&done).map(|m| m.len()).ok() != Some(size) {
            // Continue the stream that was under way when the download was
            // paused, if what is on disk still matches it.
            let fits = may_resume
                && !saved_segments.is_empty()
                && saved_segments.iter().all(|s| s[0] <= s[1] && s[1] <= s[2] && s[2] <= size)
                && saved_segments.iter().map(|s| s[2]).max() == Some(size)
                && std::fs::metadata(&part).map(|m| m.len()).ok() == Some(size);
            may_resume = false;
            if !fits {
                let _ = std::fs::remove_file(&part);
            }
            live.downloaded.store(finished, Relaxed);
            crate::task::fetch_stream(
                inner,
                live,
                &stream.url,
                &stream.headers,
                &part,
                size,
                rec.connections,
                stream.request_cap,
                fits.then(|| saved_segments.to_vec()),
            )
            .await?;
            std::fs::rename(&part, &done)
                .map_err(|e| Fail::Fatal(format!("could not move finished file: {e}")))?;
        }
        finished += size;
        live.downloaded.store(finished, Relaxed);
        inputs.push(done);
    }
    live.segs.lock().unwrap().clear();
    inner.update(id, |r| r.segments.clear());

    let joined = work.join(format!("joined.{ext}"));
    assemble(live, ffmpeg.as_deref(), &inputs, audio_only, &joined).await?;

    let mut final_name = None;
    for n in 0..10_000 {
        let candidate = crate::probe::candidate(&name, n);
        let dest = rec.dir.join(&candidate);
        if dest.exists() {
            continue;
        }
        std::fs::rename(&joined, &dest)
            .map_err(|e| Fail::Fatal(format!("could not move finished file: {e}")))?;
        final_name = Some(candidate);
        break;
    }
    let final_name = final_name.ok_or_else(|| Fail::Fatal("could not find a free file name".into()))?;
    let size = std::fs::metadata(rec.dir.join(&final_name)).map(|m| m.len()).unwrap_or(0);
    live.downloaded.store(size, Relaxed);
    live.total_hint.store(size, Relaxed);
    inner.update(id, |r| {
        r.filename = final_name.clone();
        r.total = Some(size);
    });
    remove_work_dir(&rec.dir, id);
    Ok(())
}

pub(crate) async fn run(
    inner: &Arc<Inner>,
    id: u64,
    live: &Arc<Live>,
    rec: &Record,
) -> Result<(), Fail> {
    let ytdlp = find_tool("yt-dlp").ok_or_else(|| {
        Fail::Fatal(
            "This link is a web page, not a file. Site downloads need yt-dlp, \
             which was not found on this computer"
                .into(),
        )
    })?;
    std::fs::create_dir_all(&rec.dir)
        .map_err(|e| Fail::Fatal(format!("could not create folder: {e}")))?;

    // Segments saved when a multi-connection site download was paused.
    let saved_segments = rec.segments.clone();
    inner.update(id, |r| {
        r.media = true;
        r.resumable = true;
        r.segments.clear();
    });

    // A download that was given a name (the Browser page names it after the
    // page) gets the quality in its name, and a file of its own: the same
    // page in another quality must not land on the first one's file.
    if rec.user_named {
        inner.claim_named_stem(id, &named_base(rec));
    }

    // A second download of the same video in the same quality keeps its own
    // file: "name (2)", "name (3)" and so on, decided once and remembered.
    let copy = inner.claim_media_copy(id);
    let suffix = if copy == 0 {
        String::new()
    } else {
        format!(" ({})", copy + 1)
    };
    let work = work_dir(&rec.dir, id);
    std::fs::create_dir_all(&work)
        .map_err(|e| Fail::Fatal(format!("could not create folder: {e}")))?;

    // First choice: fetch the video's streams with many connections at once.
    // RUSHDOWNLOAD_NO_FAST_SITE turns this off, for tests.
    if std::env::var_os("RUSHDOWNLOAD_NO_FAST_SITE").is_none() {
        let plan = plan(inner, live, rec, &ytdlp, &suffix, &work).await?;
        debug(|| match &plan {
            Some(p) => format!("{} plain stream(s) found, fetching with many connections", p.streams.len()),
            None => "no plain streams, leaving the download to yt-dlp".into(),
        });
        if let Some(plan) = plan {
            match fetch_direct(inner, id, live, rec, &plan, &work, &saved_segments).await {
                Ok(()) => return Ok(()),
                Err(Fail::Cancelled) => return Err(Fail::Cancelled),
                Err(e) => {
                    debug(|| format!("many connections failed ({e:?}), starting over with yt-dlp"));
                    // The site would not serve it that way: start over with
                    // yt-dlp's own downloader.
                    live.segs.lock().unwrap().clear();
                    live.downloaded.store(0, Relaxed);
                    inner.update(id, |r| r.segments.clear());
                    if let Ok(entries) = std::fs::read_dir(&work) {
                        for entry in entries.flatten() {
                            let name = entry.file_name().to_string_lossy().to_string();
                            if name.starts_with("stream-") || name.starts_with("joined.") {
                                let _ = std::fs::remove_file(entry.path());
                            }
                        }
                    }
                }
            }
        }
    }

    let mut cmd = base_command(&ytdlp, inner, rec, &suffix, &work);
    cmd.args([
        "--newline",
        "--progress",
        "--no-simulate",
        "--continue",
        "--progress-template",
        "download:OMNI|%(progress.downloaded_bytes)s|%(progress.total_bytes)s|%(progress.total_bytes_estimate)s",
        "--print",
        "before_dl:OMNINAME|%(filename)s",
        "--print",
        "after_move:OMNIFILE|%(filepath)s",
        "-N",
    ])
    .arg(rec.connections.clamp(1, 16).to_string());
    // The limit in force when the download starts applies to it throughout.
    let limit = inner.limiter.rate();
    if limit > 0 {
        cmd.arg("--limit-rate").arg(limit.to_string());
    }
    cmd.arg("--").arg(&rec.url);

    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| Fail::Fatal(format!("could not start yt-dlp: {e}")))?;
    let _conn = ConnGuard::new(&live.conns);

    let last_error = Arc::new(Mutex::new(None::<String>));
    if let Some(stderr) = child.stderr.take() {
        let last_error = last_error.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if line.starts_with("ERROR:") {
                    *last_error.lock().unwrap() = Some(line);
                }
            }
        });
    }

    // Video and audio usually arrive as separate streams, one after the
    // other; `base` carries the bytes of the streams already finished.
    let mut base = 0u64;
    let mut current = 0u64;
    let mut started = false;
    let mut final_path: Option<PathBuf> = None;
    let mut planned_name: Option<String> = None;

    let stdout = child.stdout.take().expect("stdout is piped");
    let mut lines = BufReader::new(stdout).lines();
    loop {
        let line = tokio::select! {
            _ = live.cancel.cancelled() => {
                kill_tree(&mut child).await;
                return Err(Fail::Cancelled);
            }
            l = lines.next_line() => l,
        };
        let Ok(Some(line)) = line else { break };

        if let Some(rest) = line.strip_prefix("OMNI|") {
            let mut fields = rest.split('|');
            let Some(done) = fields.next().and_then(number) else {
                continue;
            };
            let total = fields.next().and_then(number).or_else(|| fields.next().and_then(number));
            if done < current {
                base += current; // a new stream started
            }
            current = done;
            live.downloaded.store(base + done, Relaxed);
            if let Some(total) = total {
                live.total_hint.store(base + total.max(done), Relaxed);
            }
            if !started {
                started = true;
                inner.update(id, |r| r.status = Status::Downloading);
            }
        } else if let Some(path) = line.strip_prefix("OMNINAME|") {
            if let Some(name) = Path::new(path.trim()).file_name() {
                let name = name.to_string_lossy().to_string();
                planned_name = Some(name.clone());
                inner.update(id, |r| r.filename = name);
            }
        } else if let Some(path) = line.strip_prefix("OMNIFILE|") {
            final_path = Some(PathBuf::from(path.trim()));
        }
    }

    let status = tokio::select! {
        _ = live.cancel.cancelled() => {
            kill_tree(&mut child).await;
            return Err(Fail::Cancelled);
        }
        s = child.wait() => s.map_err(|e| Fail::Fatal(format!("yt-dlp failed: {e}")))?,
    };
    if !status.success() {
        let msg = last_error
            .lock()
            .unwrap()
            .take()
            .map(|e| friendly(&e))
            .unwrap_or_else(|| format!("yt-dlp stopped unexpectedly ({status})"));
        return Err(Fail::Fatal(msg));
    }

    let path = final_path
        .or_else(|| planned_name.map(|n| rec.dir.join(n)))
        .filter(|p| p.is_file())
        .ok_or_else(|| Fail::Fatal("the site returned nothing to download".into()))?;
    let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    live.downloaded.store(size, Relaxed);
    live.total_hint.store(size, Relaxed);
    inner.update(id, |r| {
        if let Some(name) = path.file_name() {
            r.filename = name.to_string_lossy().to_string();
        }
        if let Some(dir) = path.parent() {
            r.dir = dir.to_path_buf();
        }
        r.total = Some(size);
    });
    remove_work_dir(&rec.dir, id);
    Ok(())
}

/// Removes the temporary pieces yt-dlp leaves for an unfinished download.
pub(crate) fn cleanup(dir: &Path, filename: &str) {
    let stem = match filename.rfind('.') {
        Some(i) if i > 0 => &filename[..i],
        _ => filename,
    };
    // Only names yt-dlp derived from "<title> [<id>]" are touched.
    if !stem.ends_with(']') {
        return;
    }
    let prefix = format!("{stem}.");
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let Some(rest) = name.strip_prefix(&prefix) else {
            continue;
        };
        let partial = rest.ends_with(".part")
            || rest.ends_with(".ytdl")
            || rest.contains(".part-Frag")
            || (rest.starts_with('f') && rest[1..].starts_with(|c: char| c.is_ascii_digit()));
        if partial {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plain_file_streams_are_fetched_directly() {
        let two: serde_json::Value = serde_json::from_str(
            r#"{"requested_formats":[
                {"url":"https://v.test/video","protocol":"https","ext":"mp4",
                 "filesize":81320500,
                 "http_headers":{"User-Agent":"x"},"downloader_options":{"http_chunk_size":10485760}},
                {"url":"https://v.test/audio","protocol":"https","ext":"m4a"}]}"#,
        )
        .unwrap();
        let streams = direct_streams(&two).unwrap();
        assert_eq!(streams.len(), 2);
        assert_eq!(streams[0].request_cap, 10485760);
        assert_eq!(streams[0].size, Some(81320500));
        assert_eq!(streams[1].size, None);
        assert_eq!(streams[0].headers, vec![("User-Agent".to_string(), "x".to_string())]);
        assert_eq!(streams[1].request_cap, 0);

        let one: serde_json::Value =
            serde_json::from_str(r#"{"url":"http://v.test/clip.mp4","protocol":"http","ext":"mp4"}"#).unwrap();
        assert_eq!(direct_streams(&one).unwrap().len(), 1);

        // Videos served in many small pieces stay with yt-dlp.
        let pieces: serde_json::Value = serde_json::from_str(
            r#"{"requested_formats":[
                {"url":"https://v.test/a.m3u8","protocol":"m3u8_native","ext":"mp4"},
                {"url":"https://v.test/audio","protocol":"https","ext":"m4a"}]}"#,
        )
        .unwrap();
        assert!(direct_streams(&pieces).is_none());
    }

    #[test]
    fn a_cut_connection_is_told_apart_from_a_real_answer() {
        for cut in [
            "[Site] abc: Unable to download webpage: [SSL: UNEXPECTED_EOF_WHILE_READING] EOF occurred in violation of protocol",
            "Unable to download webpage: [WinError 10054] An existing connection was forcibly closed by the remote host",
            "Connection to site.test timed out. (connect timeout=20.0)",
            "('Connection aborted.', RemoteDisconnected('Remote end closed connection without response'))",
        ] {
            assert!(cut_short(cut), "{cut}");
        }
        for real in [
            "This link is a web page with no video RushDownload can download from the link alone",
            "HTTP Error 404: Not Found",
            "This video is private",
            "Sign in to confirm you are not a bot",
        ] {
            assert!(!cut_short(real), "{real}");
        }
    }

    #[test]
    fn reads_title_picture_and_resolutions() {
        let info: serde_json::Value = serde_json::from_str(
            r#"{"title":" A clip ","thumbnail":"https://i.test/t.jpg","duration":95.4,"extractor_key":"Youtube",
                "formats":[
                  {"vcodec":"none","acodec":"opus"},
                  {"vcodec":"avc1","height":720},{"vcodec":"vp9","height":1080},
                  {"vcodec":"avc1","height":1080},{"vcodec":"avc1","height":360},
                  {"vcodec":"none","height":48}]}"#,
        )
        .unwrap();
        let read = read_info(&info).unwrap();
        assert_eq!(read.title, "A clip");
        assert_eq!(read.thumbnail.as_deref(), Some("https://i.test/t.jpg"));
        assert_eq!(read.duration, Some(95));
        assert_eq!(read.heights, vec![1080, 720, 360]);
        assert_eq!(read.site.as_deref(), Some("Youtube"));
        assert!(read_info(&serde_json::json!({"formats": []})).is_none());
    }

    /// Needs ffmpeg: `cargo test --release -p rushdownload-engine -- --ignored joins`
    #[tokio::test]
    #[ignore]
    async fn joins_a_video_stream_and_an_audio_stream() {
        let ffmpeg = find_tool("ffmpeg").expect("ffmpeg is installed");
        let dir = std::env::temp_dir().join(format!("rush-join-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let live = Arc::new(Live::new());
        let os = std::ffi::OsStr::new;

        // A two-second test picture and a two-second tone, as separate files.
        let video = dir.join("stream-0.mp4");
        let audio = dir.join("stream-1.m4a");
        ffmpeg_run(
            &live,
            &ffmpeg,
            &["-f", "lavfi", "-i", "testsrc=duration=2:size=160x120:rate=10", "-pix_fmt", "yuv420p"]
                .map(os)
                .into_iter()
                .chain([video.as_os_str()])
                .collect::<Vec<_>>(),
        )
        .await
        .unwrap();
        ffmpeg_run(
            &live,
            &ffmpeg,
            &["-f", "lavfi", "-i", "sine=frequency=440:duration=2", "-c:a", "aac"]
                .map(os)
                .into_iter()
                .chain([audio.as_os_str()])
                .collect::<Vec<_>>(),
        )
        .await
        .unwrap();

        let joined = dir.join("joined.mp4");
        assemble(&live, Some(&ffmpeg), &[video.clone(), audio.clone()], false, &joined)
            .await
            .unwrap();
        let size = std::fs::metadata(&joined).unwrap().len();
        let parts = std::fs::metadata(&video).unwrap().len() + std::fs::metadata(&audio).unwrap().len();
        println!("video + audio = {parts} bytes, joined = {size} bytes");
        assert!(size > parts / 2, "the joined file holds both streams");

        // The joined file really has one picture stream and one sound stream.
        let probe = std::process::Command::new(&ffmpeg)
            .args(["-hide_banner", "-i"])
            .arg(&joined)
            .output()
            .unwrap();
        let report = String::from_utf8_lossy(&probe.stderr).to_string();
        assert!(report.contains("Video:") && report.contains("Audio:"), "{report}");

        // Audio only: the sound stream becomes an mp3.
        let mp3 = dir.join("joined.mp3");
        assemble(&live, Some(&ffmpeg), &[audio.clone()], true, &mp3).await.unwrap();
        assert!(std::fs::metadata(&mp3).unwrap().len() > 1000);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
