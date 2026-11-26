//! A social media profile link downloads everything the profile has posted,
//! into a folder of its own with `Images` and `Videos` inside. gallery-dl
//! does the fetching for most sites; yt-dlp for the video-only ones.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use tokio::io::{AsyncBufReadExt, BufReader};

use crate::media::{command, find_tool, kill_tree};
use crate::probe::{candidate, sanitize};
use crate::task::{ConnGuard, Fail, Live};
use crate::{CurrentFile, Inner, ItemInfo, ItemState, Record, Status};

/// A profile recognised in a link.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProfileRef {
    /// The site's name, e.g. "Instagram".
    pub site: String,
    /// The account's name as written in the link.
    pub name: String,
}

const INCOMING: &str = ".incoming";
const IMAGES: &str = "Images";
const VIDEOS: &str = "Videos";
const IMAGE_EXT: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "webp", "bmp", "avif", "heic", "heif", "tif", "tiff", "svg", "jfif",
];
const VIDEO_EXT: &[&str] = &[
    "mp4", "webm", "mov", "m4v", "mkv", "avi", "flv", "ts", "wmv", "3gp", "mpg", "mpeg", "m2ts",
];

/// Recognises links that point at a whole profile rather than one post.
pub(crate) fn detect(link: &str) -> Option<ProfileRef> {
    let url = reqwest::Url::parse(link.trim()).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let host = url.host_str()?.to_ascii_lowercase();
    let host = ["www.", "m.", "mobile.", "old.", "new."]
        .iter()
        .find_map(|p| host.strip_prefix(p))
        .unwrap_or(&host)
        .to_string();
    let segments: Vec<&str> = url.path_segments()?.filter(|s| !s.is_empty()).collect();
    let first = segments.first().copied().unwrap_or("");
    let lower = first.to_ascii_lowercase();
    let found = |site: &str, name: &str| {
        let name = name.trim_start_matches('@');
        (!name.is_empty()).then(|| ProfileRef {
            site: site.to_string(),
            name: name.to_string(),
        })
    };
    let one = segments.len() == 1;

    match host.as_str() {
        "instagram.com" => {
            let reserved = [
                "p", "reel", "reels", "tv", "explore", "stories", "accounts", "direct", "about", "legal",
                "developer", "share",
            ];
            (one && !reserved.contains(&lower.as_str())).then(|| found("Instagram", first))?
        }
        "x.com" | "twitter.com" => {
            let reserved = [
                "home", "i", "search", "explore", "settings", "notifications", "messages", "hashtag", "intent",
                "share", "login", "signup", "compose", "tos", "privacy",
            ];
            let tab = segments.len() == 2 && matches!(segments[1], "media" | "with_replies");
            ((one || tab) && !reserved.contains(&lower.as_str())).then(|| found("X", first))?
        }
        "tiktok.com" => (one && first.starts_with('@')).then(|| found("TikTok", first))?,
        "youtube.com" => {
            if lower == "playlist" {
                // A playlist is fetched whole too, into a folder of its own.
                let list = url.query_pairs().find(|(key, _)| key == "list").map(|(_, id)| id.to_string());
                list.and_then(|id| found(PLAYLIST, &id))
            } else if first.starts_with('@') {
                found("YouTube", first)
            } else if matches!(lower.as_str(), "channel" | "c" | "user") && segments.len() >= 2 {
                found("YouTube", segments[1])
            } else {
                None
            }
        }
        "facebook.com" => {
            let reserved = [
                "watch", "reel", "groups", "events", "marketplace", "gaming", "photo", "photo.php", "story.php",
                "share", "login", "profile.php", "pages", "permalink.php", "sharer", "help", "stories",
            ];
            (one && !reserved.contains(&lower.as_str())).then(|| found("Facebook", first))?
        }
        "reddit.com" => {
            (segments.len() >= 2 && matches!(lower.as_str(), "user" | "u")).then(|| found("Reddit", segments[1]))?
        }
        "bsky.app" => (segments.len() == 2 && lower == "profile").then(|| found("Bluesky", segments[1]))?,
        "flickr.com" => (segments.len() == 2 && lower == "photos").then(|| found("Flickr", segments[1]))?,
        "tumblr.com" => (one && !matches!(lower.as_str(), "explore" | "dashboard" | "search" | "tagged"))
            .then(|| found("Tumblr", first))?,
        "deviantart.com" => (one || (segments.len() == 2 && segments[1] == "gallery"))
            .then(|| found("DeviantArt", first))?,
        "vsco.co" => (one || (segments.len() == 2 && segments[1] == "gallery")).then(|| found("VSCO", first))?,
        _ => {
            if let Some(name) = host.strip_suffix(".tumblr.com") {
                return segments.is_empty().then(|| found("Tumblr", name))?;
            }
            if host.starts_with("pinterest.") {
                let reserved = ["pin", "search", "ideas", "today", "login", "explore"];
                return (segments.len() <= 2 && !first.is_empty() && !reserved.contains(&lower.as_str()))
                    .then(|| found("Pinterest", first))?;
            }
            None
        }
    }
}

/// The folder name a profile is saved under.
pub(crate) fn folder_name(profile: &ProfileRef) -> String {
    sanitize(&format!("{} ({})", profile.name, profile.site))
}

/// Sites whose profiles hold videos only, which yt-dlp fetches.
fn video_site(site: &str) -> bool {
    matches!(site, "YouTube" | "TikTok" | PLAYLIST)
}

/// What a playlist link is called in place of a site's name.
pub(crate) const PLAYLIST: &str = "YouTube playlist";

/// What a playlist or a profile holds, shown before it is downloaded.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PlaylistInfo {
    pub title: String,
    /// How many videos it lists.
    pub count: u64,
    /// How many photos it lists.
    pub photos: u64,
    /// The counting stopped early: it holds more than this.
    pub more: bool,
    /// How long all its videos run together, in seconds, where the site says.
    pub duration: Option<u64>,
    pub thumbnail: Option<String>,
    /// Whose playlist it is.
    pub channel: Option<String>,
    /// A few of its videos (first, middle, last), to see which resolutions it really has.
    pub samples: Vec<String>,
}

/// The videos listed, however deep: a channel lists its tabs, which list the videos.
fn videos_of<'a>(info: &'a serde_json::Value, out: &mut Vec<&'a serde_json::Value>) {
    for entry in info.get("entries").and_then(|e| e.as_array()).into_iter().flatten() {
        if entry.get("entries").is_some() {
            videos_of(entry, out);
        } else if !entry.is_null() {
            out.push(entry);
        }
    }
}

fn read_playlist(info: &serde_json::Value) -> Option<PlaylistInfo> {
    let title = info.get("title")?.as_str()?.trim().to_string();
    let mut videos = Vec::new();
    videos_of(info, &mut videos);
    let in_tabs = info
        .get("entries")
        .and_then(|e| e.as_array())
        .is_some_and(|list| list.iter().any(|e| e.get("entries").is_some()));
    // The largest picture the site offers: the playlist's own, else its first video's.
    let picture = |of: &serde_json::Value| {
        of.get("thumbnails")
            .and_then(|t| t.as_array())
            .and_then(|list| list.iter().rev().find_map(|t| t.get("url")?.as_str()))
            .or_else(|| of.get("thumbnail").and_then(|t| t.as_str()))
            .filter(|url| url.starts_with("http"))
            .map(str::to_string)
    };
    let text = |key: &str| info.get(key).and_then(|v| v.as_str()).filter(|v| !v.is_empty()).map(str::to_string);
    let seconds: f64 = videos.iter().filter_map(|v| v.get("duration")?.as_f64()).sum();
    let mut samples: Vec<String> = Vec::new();
    for at in [0, videos.len() / 2, videos.len().saturating_sub(1)] {
        let link = videos.get(at).and_then(|v| {
            ["url", "webpage_url"].iter().find_map(|key| v.get(*key)?.as_str()).filter(|l| l.starts_with("http"))
        });
        if let Some(link) = link.filter(|l| !samples.iter().any(|s| s == l)) {
            samples.push(link.to_string());
        }
    }
    Some(PlaylistInfo {
        // With tabs, the site's own count is the number of tabs.
        count: info
            .get("playlist_count")
            .and_then(|c| c.as_u64())
            .filter(|_| !in_tabs)
            .unwrap_or(videos.len() as u64),
        photos: 0,
        more: false,
        duration: (seconds >= 1.0).then_some(seconds as u64),
        thumbnail: picture(info).or_else(|| videos.first().and_then(|v| picture(v))),
        channel: text("channel").or_else(|| text("uploader")),
        samples,
        title,
    })
    .filter(|p| !p.title.is_empty())
}

/// gallery-dl is asked to print each file's kind on a line of its own, marked like this.
const LISTED: &str = "kind=";

/// Whether a line gallery-dl printed stands for a video (`true`) or a photo
/// (`false`), or is not one of the profile's files at all.
fn listed(line: &str) -> Option<bool> {
    let kind = line.trim().strip_prefix(LISTED)?.trim().to_ascii_lowercase();
    Some(VIDEO_EXT.contains(&kind.as_str()) || matches!(kind.as_str(), "m3u8" | "mpd"))
}

/// Counting a profile stops at this many files, or after `COUNT_FOR`.
const COUNT_UP_TO: u64 = 500;
const COUNT_FOR: Duration = Duration::from_secs(30);

/// Counts the photos and videos a profile lists. Nothing is downloaded.
async fn profile_counts(inner: &Arc<Inner>, url: &str, profile: &ProfileRef) -> Result<PlaylistInfo, String> {
    let tool = find_tool("gallery-dl").ok_or("gallery-dl was not found on this computer")?;
    let mut cmd = command(&tool);
    cmd.env("PYTHONUTF8", "1")
        .env("PYTHONIOENCODING", "utf-8")
        // Each file's kind is printed in place of fetching it; the address
        // alone does not always say whether it is a photo or a video.
        .arg("--print")
        .arg(format!("{LISTED}{{extension}}"))
        .arg("--range")
        .arg(format!("1-{COUNT_UP_TO}"));
    inner.add_sign_in(&mut cmd, None);
    cmd.arg(url).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("could not start gallery-dl: {e}"))?;

    let last_error = Arc::new(Mutex::new(None::<String>));
    if let Some(stderr) = child.stderr.take() {
        let last_error = last_error.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if line.contains("[error]") || line.starts_with("ERROR:") {
                    *last_error.lock().unwrap() = Some(line);
                }
            }
        });
    }
    let stdout = child.stdout.take().expect("stdout is piped");
    let mut lines = BufReader::new(stdout).lines();
    let until = tokio::time::Instant::now() + COUNT_FOR;
    let (mut videos, mut photos, mut cut_off) = (0u64, 0u64, false);
    loop {
        match tokio::time::timeout_at(until, lines.next_line()).await {
            Ok(Ok(Some(line))) => match listed(&line) {
                Some(true) => videos += 1,
                Some(false) => photos += 1,
                None => {}
            },
            Ok(_) => break,
            Err(_) => {
                cut_off = true;
                break;
            }
        }
    }
    if cut_off {
        kill_tree(&mut child).await;
    } else {
        let _ = child.wait().await;
    }
    if videos + photos == 0 {
        let said = last_error.lock().unwrap().take();
        return Err(match said {
            Some(error) => friendly(&error),
            None if cut_off => "the site took too long to answer".into(),
            None => "Nothing to download was found on this profile".into(),
        });
    }
    Ok(PlaylistInfo {
        title: profile.name.clone(),
        count: videos,
        photos,
        more: cut_off || videos + photos >= COUNT_UP_TO,
        duration: None,
        thumbnail: None,
        channel: None,
        samples: Vec::new(),
    })
}

/// Looks up what a playlist or a profile holds: its name, picture, how many
/// videos and photos, and how long the videos run. Nothing is downloaded.
pub(crate) async fn playlist_info(inner: &Arc<Inner>, url: &str) -> Result<PlaylistInfo, String> {
    if let Some(profile) = detect(url).filter(|p| !video_site(&p.site)) {
        return profile_counts(inner, url, &profile).await;
    }
    let tool = find_tool("yt-dlp").ok_or("yt-dlp was not found on this computer")?;
    let mut cmd = command(&tool);
    cmd.args(["--encoding", "utf-8", "--no-colors", "--flat-playlist", "-J"]);
    inner.add_sign_in(&mut cmd, None);
    cmd.arg("--").arg(url).stdout(Stdio::piped()).stderr(Stdio::piped());
    let child = cmd.spawn().map_err(|e| format!("could not start yt-dlp: {e}"))?;
    let out = tokio::time::timeout(Duration::from_secs(60), child.wait_with_output())
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
            .unwrap_or_else(|| "the playlist could not be read".into()));
    }
    serde_json::from_slice::<serde_json::Value>(&out.stdout)
        .ok()
        .as_ref()
        .and_then(read_playlist)
        .ok_or_else(|| "the playlist could not be read".into())
}

/// The playlist's own name, asked from the site. Nothing is downloaded.
async fn playlist_title(inner: &Arc<Inner>, live: &Arc<Live>, tool: &Path, url: &str) -> Option<String> {
    let mut cmd = command(tool);
    cmd.args([
        "--encoding",
        "utf-8",
        "--no-colors",
        "--flat-playlist",
        "--playlist-end",
        "1",
        "--print",
        "%(playlist_title)s",
    ]);
    inner.add_sign_in(&mut cmd, None);
    cmd.arg("--").arg(url).stdout(Stdio::piped()).stderr(Stdio::null());
    let child = cmd.spawn().ok()?;
    let out = tokio::select! {
        _ = live.cancel.cancelled() => return None,
        o = tokio::time::timeout(Duration::from_secs(40), child.wait_with_output()) => o.ok()?.ok()?,
    };
    let title = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && *line != "NA")?
        .to_string();
    Some(sanitize(&title)).filter(|name| !name.is_empty())
}

/// The videos a playlist or channel lists, and which of them are fetched.
#[derive(Clone, Default)]
pub(crate) struct Listed {
    /// Each video, in the site's order.
    pub items: Vec<ListedItem>,
    done: HashSet<String>,
    /// The download ran to its end: what is not done was not given.
    finished: bool,
}

/// One video as the site lists it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ListedItem {
    id: String,
    title: String,
    duration: Option<u64>,
    thumbnail: Option<String>,
    url: Option<String>,
    channel: Option<String>,
}

impl Listed {
    /// `folder` is where the finished files are; each carries its id in its name.
    pub(crate) fn show(&self, current: Option<&CurrentFile>, folder: Option<&Path>) -> Vec<ItemInfo> {
        let files: Vec<(String, u64)> = folder
            .map(|f| [f.to_path_buf(), f.join(VIDEOS)])
            .into_iter()
            .flatten()
            .flat_map(|dir| files_in(&dir))
            .filter_map(|(path, size)| Some((path.file_name()?.to_string_lossy().to_string(), size)))
            .collect();
        self.items
            .iter()
            .map(|item| {
                let now = current.filter(|c| c.id == item.id);
                let done = self.done.contains(&item.id);
                let marked = format!("[{}].", item.id);
                let file = files.iter().find(|(name, _)| done && name.contains(&marked));
                ItemInfo {
                    title: item.title.clone(),
                    state: if done {
                        ItemState::Done
                    } else if now.is_some() {
                        ItemState::Downloading
                    } else if self.finished {
                        ItemState::Missed
                    } else {
                        ItemState::Waiting
                    },
                    size: file.map(|(_, size)| *size).or(now.map(|c| c.downloaded)),
                    file: file.map(|(name, _)| name.clone()),
                    thumbnail: item.thumbnail.clone(),
                    duration: item.duration,
                    channel: item.channel.clone(),
                    url: item.url.clone(),
                }
            })
            .collect()
    }
}

/// What the site is asked to print for each video, tabs between.
const LISTING: &str = "%(id)s\t%(duration)s\t%(thumbnails.-1.url)s\t%(url)s\t%(channel,uploader)s\t%(title)s";

/// Reads the lines of a listing, one video each.
fn read_listing(text: &str) -> Vec<ListedItem> {
    let mut seen = HashSet::new();
    text.lines()
        .filter_map(|line| {
            let mut fields = line.splitn(6, '\t').map(|f| Some(f.trim()).filter(|f| !f.is_empty() && *f != "NA"));
            let id = fields.next()??;
            if !seen.insert(id.to_string()) {
                return None;
            }
            let duration = fields.next()?.and_then(|d| d.parse::<f64>().ok()).map(|d| d as u64);
            let mut link = || fields.next().flatten().filter(|l| l.starts_with("http")).map(str::to_string);
            let (thumbnail, url) = (link(), link());
            let channel = fields.next().flatten().map(str::to_string);
            Some(ListedItem {
                id: id.to_string(),
                title: fields.next().flatten().unwrap_or(id).to_string(),
                duration,
                thumbnail,
                url,
                channel,
            })
        })
        .collect()
}

/// The ids in yt-dlp's record of what it has fetched: `site id` per line.
fn fetched(folder: &Path) -> HashSet<String> {
    std::fs::read_to_string(folder.join(ARCHIVE))
        .unwrap_or_default()
        .lines()
        .filter_map(|line| line.split_whitespace().last().map(str::to_string))
        .collect()
}

const ARCHIVE: &str = ".rush-archive.txt";

/// Asks the site which videos the link lists, while the download gets going.
async fn list_items(inner: Arc<Inner>, live: Arc<Live>, id: u64, tool: PathBuf, url: String, limit: Option<String>) {
    let mut cmd = command(&tool);
    cmd.args(["--encoding", "utf-8", "--no-colors", "--flat-playlist", "--quiet", "--no-warnings", "--print", LISTING]);
    if let Some(n) = &limit {
        cmd.arg("--playlist-end").arg(n);
    }
    inner.add_sign_in(&mut cmd, None);
    cmd.arg("--").arg(&url).stdout(Stdio::piped()).stderr(Stdio::null());
    let Ok(child) = cmd.spawn() else { return };
    let out = tokio::select! {
        _ = live.cancel.cancelled() => return,
        o = tokio::time::timeout(Duration::from_secs(180), child.wait_with_output()) => o,
    };
    let Ok(Ok(out)) = out else { return };
    let items = read_listing(&String::from_utf8_lossy(&out.stdout));
    if items.is_empty() || live.removed.load(Relaxed) {
        return;
    }
    inner.lists.lock().unwrap().entry(id).or_default().items = items;
}

/// What a profile's folder holds, as finished items in name order.
pub(crate) fn on_disk(folder: &Path) -> Vec<ItemInfo> {
    let mut files: Vec<(String, u64)> = [folder.to_path_buf(), folder.join(IMAGES), folder.join(VIDEOS)]
        .iter()
        .flat_map(|dir| files_in(dir))
        .filter_map(|(path, size)| Some((path.file_name()?.to_string_lossy().to_string(), size)))
        .filter(|(name, _)| !name.starts_with(".rush-"))
        .collect();
    files.sort();
    files
        .into_iter()
        .map(|(title, size)| ItemInfo {
            file: Some(title.clone()),
            title,
            state: ItemState::Done,
            size: Some(size),
            thumbnail: None,
            duration: None,
            channel: None,
            url: None,
        })
        .collect()
}

/// yt-dlp is asked to report each file's progress on lines marked like this.
const PROGRESS: &str = "RUSH|";
const PROGRESS_TEMPLATE: &str = "download:RUSH|%(progress.downloaded_bytes)s|%(progress.total_bytes)s|\
     %(progress.total_bytes_estimate)s|%(info.playlist_index)s|%(info.n_entries)s|%(info.id)s|%(info.title)s";

/// Follows the file yt-dlp is fetching. A video usually arrives as two
/// streams, picture then sound; the bytes of the first are carried over.
#[derive(Default)]
struct Following {
    id: String,
    /// Bytes of this file's streams that are already complete.
    base: u64,
    /// Bytes of the stream arriving now.
    stream: u64,
}

impl Following {
    /// Reads one progress line; `numbered` keeps the place in the playlist.
    fn read(&mut self, line: &str, numbered: bool) -> Option<CurrentFile> {
        let mut fields = line.strip_prefix(PROGRESS)?.splitn(7, '|');
        let number = |field: Option<&str>| field?.trim().parse::<f64>().ok().map(|v| v as u64);
        let done = number(fields.next())?;
        let exact = number(fields.next());
        let estimate = number(fields.next());
        let total = exact.or(estimate);
        let index = number(fields.next()).filter(|_| numbered);
        let count = number(fields.next()).filter(|_| numbered);
        let id = fields.next()?.trim();
        let title = fields.next().map(str::trim).filter(|t| !t.is_empty() && *t != "NA");
        if id != self.id {
            *self = Following { id: id.to_string(), ..Default::default() };
        } else if done < self.stream {
            self.base += self.stream; // the next stream of the same file
        }
        self.stream = done;
        Some(CurrentFile {
            id: id.to_string(),
            name: title.unwrap_or(id).to_string(),
            downloaded: self.base + done,
            total: total.map(|t| self.base + t.max(done)),
            index,
            count: count.filter(|c| index.is_some_and(|i| i <= *c)),
        })
    }
}

/// The file gallery-dl is part-way through, read from the incoming folder:
/// it reports nothing itself until a file is complete.
fn arriving(incoming: &Path) -> Option<CurrentFile> {
    let (path, size) = files_in(incoming).into_iter().find(|(path, _)| extension(path) == "part")?;
    Some(CurrentFile {
        id: String::new(),
        name: path.file_stem()?.to_string_lossy().to_string(),
        downloaded: size,
        total: None,
        index: None,
        count: None,
    })
}

fn extension(path: &Path) -> String {
    path.extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

/// Moves one finished file from the incoming folder to `Images`, `Videos`,
/// or (anything else) the profile folder itself.
fn file_away(folder: &Path, file: &Path) {
    let Some(name) = file.file_name().map(|n| n.to_string_lossy().to_string()) else {
        return;
    };
    if !file.is_file() {
        return;
    }
    let ext = extension(file);
    let target = if VIDEO_EXT.contains(&ext.as_str()) {
        folder.join(VIDEOS)
    } else if IMAGE_EXT.contains(&ext.as_str()) {
        folder.join(IMAGES)
    } else {
        folder.to_path_buf()
    };
    for n in 0..10_000 {
        let dest = target.join(candidate(&name, n));
        if dest.exists() {
            continue;
        }
        let _ = std::fs::rename(file, dest);
        return;
    }
}

fn files_in(dir: &Path) -> Vec<(PathBuf, u64)> {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let meta = e.metadata().ok()?;
            meta.is_file().then(|| (e.path(), meta.len()))
        })
        .collect()
}

/// Bytes on disk and the number of finished files.
fn tally(folder: &Path) -> (u64, u64) {
    let mut bytes = 0;
    let mut count = 0;
    for part in [IMAGES, VIDEOS] {
        for (_, size) in files_in(&folder.join(part)) {
            bytes += size;
            count += 1;
        }
    }
    for (path, size) in files_in(folder) {
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        if !name.starts_with(".rush-") {
            bytes += size;
            count += 1;
        }
    }
    bytes += files_in(&folder.join(INCOMING)).iter().map(|(_, s)| s).sum::<u64>();
    (bytes, count)
}

/// The folder as a plain child of `dir`, or nothing if the name is unsafe.
fn child(dir: &Path, folder: &str) -> Option<PathBuf> {
    if folder.is_empty() || folder == "." || folder == ".." || folder.contains(['/', '\\']) {
        return None;
    }
    let target = dir.join(folder);
    (target.parent() == Some(dir) && target.is_dir()).then_some(target)
}

fn remove_working_files(folder: &Path) {
    let _ = std::fs::remove_dir_all(folder.join(INCOMING));
    for (path, _) in files_in(folder) {
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        if name.starts_with(".rush-") {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Cancelling an unfinished profile download: the half-fetched pieces go,
/// the photos and videos that are already complete stay.
pub(crate) fn discard_unfinished(dir: &Path, folder: &str) {
    let Some(folder) = child(dir, folder) else { return };
    remove_working_files(&folder);
    // Only empty folders are removed by these calls.
    let _ = std::fs::remove_dir(folder.join(IMAGES));
    let _ = std::fs::remove_dir(folder.join(VIDEOS));
    let _ = std::fs::remove_dir(&folder);
}

fn friendly(error: &str) -> String {
    let text = error.rsplit("] ").next().unwrap_or(error).trim_start_matches("ERROR:").trim();
    let lower = error.to_ascii_lowercase();
    if ["login", "log in", "sign in", "unauthorized", "401", "authrequired", "cookies", "private", "403"]
        .iter()
        .any(|w| lower.contains(w))
    {
        "This site only shows this profile to signed-in visitors. Sign in to the site on \
         the Sign in page, then retry"
            .into()
    } else if lower.contains("notfound") || lower.contains("404") || lower.contains("does not exist") {
        "This profile was not found".into()
    } else if lower.contains("429") || lower.contains("rate limit") {
        "The site is limiting requests right now. Wait a while and retry".into()
    } else {
        text.to_string()
    }
}

pub(crate) async fn run(
    inner: &Arc<Inner>,
    id: u64,
    live: &Arc<Live>,
    rec: &Record,
) -> Result<(), Fail> {
    let profile = detect(&rec.url).ok_or_else(|| Fail::Fatal("this link is not a profile".into()))?;
    let by_ytdlp = video_site(&profile.site);
    let tool = if by_ytdlp {
        find_tool("yt-dlp").ok_or_else(|| {
            Fail::Fatal("Profile downloads from this site need yt-dlp, which was not found on this computer".into())
        })?
    } else {
        find_tool("gallery-dl").ok_or_else(|| {
            Fail::Fatal("Profile downloads need gallery-dl, which was not found on this computer".into())
        })?
    };
    std::fs::create_dir_all(&rec.dir)
        .map_err(|e| Fail::Fatal(format!("could not create folder: {e}")))?;

    let playlist = profile.site == PLAYLIST;
    // Resuming reuses the folder; what is already there is not fetched again.
    let name = if rec.part_started && rec.dir.join(&rec.filename).is_dir() {
        rec.filename.clone()
    } else {
        // A playlist's folder is named after the playlist.
        let base = match playlist {
            true => playlist_title(inner, live, &tool, &rec.url).await.unwrap_or_else(|| rec.filename.clone()),
            false => rec.filename.clone(),
        };
        if live.cancel.is_cancelled() {
            return Err(Fail::Cancelled);
        }
        crate::torrent::claim_folder(&rec.dir, &base)?
    };
    let folder = rec.dir.join(&name);
    let incoming = folder.join(INCOMING);
    // A playlist is videos only, in order, straight in its folder; a profile
    // is sorted into Images and Videos.
    let made = match playlist {
        true => vec![incoming.clone()],
        false => vec![folder.join(IMAGES), folder.join(VIDEOS), incoming.clone()],
    };
    for dir in made {
        std::fs::create_dir_all(&dir).map_err(|e| Fail::Fatal(format!("could not create folder: {e}")))?;
    }
    inner.update(id, |r| {
        r.profile = true;
        r.resumable = true;
        r.part_started = true;
        r.filename = name.clone();
        r.segments.clear();
    });

    // RUSHDOWNLOAD_PROFILE_LIMIT stops after that many files, for tests.
    let limit = std::env::var("RUSHDOWNLOAD_PROFILE_LIMIT").ok().filter(|v| v.parse::<u32>().is_ok());
    let rate = inner.limiter.rate();
    let mut cmd = command(&tool);
    if by_ytdlp {
        let mut temp = std::ffi::OsString::from("temp:");
        temp.push(&incoming);
        cmd.args([
            "--encoding",
            "utf-8",
            "--yes-playlist",
            "--no-colors",
            "--windows-filenames",
            "--ignore-errors",
            "--quiet",
            "--no-warnings",
            // Progress is still reported in quiet mode, one line at a time.
            "--progress",
            "--newline",
            "--progress-template",
            PROGRESS_TEMPLATE,
            "--continue",
            "-o",
            // A playlist keeps its order in the file names.
            if playlist { "%(playlist_index)03d - %(title).120B [%(id)s].%(ext)s" } else { "%(title).120B [%(id)s].%(ext)s" },
            "-N",
        ])
        .arg(rec.connections.clamp(1, 16).to_string())
        .arg("-P")
        .arg(if playlist { folder.clone() } else { folder.join(VIDEOS) })
        .arg("-P")
        .arg(temp)
        .arg("--download-archive")
        .arg(folder.join(ARCHIVE));
        if rec.media_options.audio_only {
            cmd.args(["-f", "ba/b", "--extract-audio", "--audio-format", "mp3", "--audio-quality", "0"]);
        } else {
            let format = match rec.media_options.max_height {
                Some(h) => format!("bv*[height<={h}]+ba/b[height<={h}]/wv*+ba/w"),
                None => "bv*+ba/b".to_string(),
            };
            cmd.args(["-f", &format, "--merge-output-format", "mp4/mkv"]);
        }
        if let Some(ffmpeg) = find_tool("ffmpeg") {
            cmd.arg("--ffmpeg-location").arg(ffmpeg);
        }
        if let Some(node) = find_tool("node") {
            cmd.arg("--js-runtimes").arg(format!("node:{}", node.display()));
        }
        if rate > 0 {
            cmd.arg("--limit-rate").arg(rate.to_string());
        }
        if let Some(n) = &limit {
            cmd.arg("--playlist-end").arg(n);
        }
    } else {
        cmd.env("PYTHONUTF8", "1")
            .env("PYTHONIOENCODING", "utf-8")
            .arg("-D")
            .arg(&incoming)
            .arg("--download-archive")
            .arg(folder.join(".rush-archive.sqlite3"));
        if rate > 0 {
            cmd.arg("--limit-rate").arg(rate.to_string());
        }
        if let Some(n) = &limit {
            cmd.arg("--range").arg(format!("1-{n}"));
        }
    }
    // The user's sign-in, if they made one.
    inner.add_sign_in(&mut cmd, Some(&folder));
    if by_ytdlp {
        cmd.arg("--");
    }
    cmd.arg(&rec.url);

    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| Fail::Fatal(format!("could not start the profile downloader: {e}")))?;
    if by_ytdlp {
        // A fresh run starts a fresh list; what is done is read from the folder.
        if let Some(listed) = inner.lists.lock().unwrap().get_mut(&id) {
            listed.finished = false;
        }
        tokio::spawn(list_items(inner.clone(), live.clone(), id, tool.clone(), rec.url.clone(), limit.clone()));
    }
    let _conn = ConnGuard::new(&live.conns);

    let last_error = Arc::new(Mutex::new(None::<String>));
    if let Some(stderr) = child.stderr.take() {
        let last_error = last_error.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if line.contains("[error]") || line.starts_with("ERROR:") {
                    *last_error.lock().unwrap() = Some(line);
                }
            }
        });
    }

    let stdout = child.stdout.take().expect("stdout is piped");
    let mut lines = BufReader::new(stdout).lines();
    let mut started = false;
    let mut open = true;
    let report = |started: &mut bool| {
        let (bytes, count) = tally(&folder);
        live.downloaded.store(bytes, Relaxed);
        live.items.store(count, Relaxed);
        if !*started && bytes > 0 {
            *started = true;
            inner.update(id, |r| r.status = Status::Downloading);
        }
    };
    let mut following = Following::default();
    // Progress lines arrive many times a second; the folder is still
    // measured on its own steady beat.
    let mut beat = tokio::time::interval(Duration::from_millis(500));
    beat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    while open {
        tokio::select! {
            _ = live.cancel.cancelled() => {
                kill_tree(&mut child).await;
                return Err(Fail::Cancelled);
            }
            line = lines.next_line() => match line {
                Ok(Some(line)) => {
                    let line = line.trim();
                    if by_ytdlp {
                        if let Some(file) = following.read(line, playlist) {
                            *live.current.lock().unwrap() = Some(file);
                        }
                    } else if !line.is_empty() && !line.starts_with('#') {
                        // gallery-dl names every file it finished; "# ..."
                        // means it was already there.
                        let done = Path::new(line);
                        if let (Some(name), Ok(meta)) = (done.file_name(), done.metadata()) {
                            *live.current.lock().unwrap() = Some(CurrentFile {
                                id: String::new(),
                                name: name.to_string_lossy().to_string(),
                                downloaded: meta.len(),
                                total: Some(meta.len()),
                                index: None,
                                count: None,
                            });
                        }
                        file_away(&folder, done);
                    }
                }
                _ => open = false,
            },
            _ = beat.tick() => {
                report(&mut started);
                if by_ytdlp {
                    let done = fetched(&folder);
                    if let Some(listed) = inner.lists.lock().unwrap().get_mut(&id) {
                        listed.done = done;
                    }
                } else {
                    if let Some(file) = arriving(&incoming) {
                        *live.current.lock().unwrap() = Some(file);
                    }
                }
            }
        }
    }
    *live.current.lock().unwrap() = None;
    let status = tokio::select! {
        _ = live.cancel.cancelled() => {
            kill_tree(&mut child).await;
            return Err(Fail::Cancelled);
        }
        s = child.wait() => s.map_err(|e| Fail::Fatal(format!("the profile downloader failed: {e}")))?,
    };

    if by_ytdlp {
        // The record of what was fetched goes with the working files; keep its last word.
        let done = fetched(&folder);
        if let Some(listed) = inner.lists.lock().unwrap().get_mut(&id) {
            listed.done = done;
            listed.finished = true;
        }
    }
    // Anything finished that was not announced still gets filed away.
    for (path, _) in files_in(&incoming) {
        if !matches!(extension(&path).as_str(), "part" | "ytdl" | "tmp") {
            file_away(&folder, &path);
        }
    }
    remove_working_files(&folder);
    let (bytes, count) = tally(&folder);
    live.downloaded.store(bytes, Relaxed);
    live.items.store(count, Relaxed);
    if count == 0 {
        discard_unfinished(&rec.dir, &name);
        inner.update(id, |r| r.part_started = false);
        let msg = last_error
            .lock()
            .unwrap()
            .take()
            .map(|e| friendly(&e))
            .unwrap_or_else(|| {
                if status.success() {
                    "Nothing to download was found on this profile".into()
                } else {
                    format!("the profile downloader stopped unexpectedly ({status})")
                }
            });
        return Err(Fail::Fatal(msg));
    }
    // Whatever was fetched is kept, even if the site cut the run short.
    live.total_hint.store(bytes, Relaxed);
    inner.update(id, |r| r.total = Some(bytes));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn who(link: &str) -> Option<(String, String)> {
        detect(link).map(|p| (p.site, p.name))
    }

    #[test]
    fn recognises_profiles_and_leaves_single_posts_alone() {
        let is = |link: &str, site: &str, name: &str| {
            assert_eq!(who(link), Some((site.to_string(), name.to_string())), "{link}");
        };
        is("https://www.instagram.com/natgeo/", "Instagram", "natgeo");
        is("https://x.com/nasa", "X", "nasa");
        is("https://twitter.com/nasa/media", "X", "nasa");
        is("https://www.tiktok.com/@someone", "TikTok", "someone");
        is("https://www.youtube.com/@channel/videos", "YouTube", "channel");
        is("https://www.youtube.com/channel/UCabc", "YouTube", "UCabc");
        is("https://youtube.com/playlist?list=PLabc123&si=xyz", "YouTube playlist", "PLabc123");
        is("https://www.youtube.com/playlist?list=PLabc123", "YouTube playlist", "PLabc123");
        is("https://bsky.app/profile/bsky.app", "Bluesky", "bsky.app");
        is("https://www.reddit.com/user/someone/", "Reddit", "someone");
        is("https://staff.tumblr.com/", "Tumblr", "staff");
        is("https://www.pinterest.com/someone/", "Pinterest", "someone");
        is("https://www.flickr.com/photos/someone", "Flickr", "someone");

        for post in [
            "https://www.instagram.com/p/ABC123/",
            "https://www.instagram.com/reel/ABC123/",
            "https://www.instagram.com/",
            "https://x.com/nasa/status/123",
            "https://x.com/home",
            "https://www.tiktok.com/@someone/video/123",
            "https://www.youtube.com/watch?v=abc",
            // one video opened from a playlist is still one video
            "https://www.youtube.com/watch?v=abc&list=PLabc123",
            "https://www.youtube.com/playlist",
            "https://www.pinterest.com/pin/123/",
            "https://example.com/someone",
            "https://bsky.app/profile/bsky.app/post/abc",
            "magnet:?xt=urn:btih:ABC",
        ] {
            assert_eq!(who(post), None, "{post}");
        }
    }

    #[test]
    fn reads_a_playlists_name_picture_and_size() {
        let info: serde_json::Value = serde_json::from_str(
            r#"{"title":" My uploads ","playlist_count":27,"channel":"Someone",
                "thumbnails":[{"url":"https://i.test/small.jpg","width":120},{"url":"https://i.test/large.jpg","width":336}],
                "entries":[{"id":"a","duration":61.4,"url":"https://v.test/a"},{"id":"b","duration":3600,"url":"https://v.test/b"},null,{"id":"c","url":"https://v.test/c"}]}"#,
        )
        .unwrap();
        assert_eq!(
            read_playlist(&info),
            Some(PlaylistInfo {
                title: "My uploads".into(),
                count: 27,
                photos: 0,
                more: false,
                duration: Some(3661),
                thumbnail: Some("https://i.test/large.jpg".into()),
                channel: Some("Someone".into()),
                samples: vec!["https://v.test/a".into(), "https://v.test/b".into(), "https://v.test/c".into()],
            })
        );
        // without its own picture or count, the first video's picture and the list's length are used
        let bare: serde_json::Value = serde_json::from_str(
            r#"{"title":"Mix","uploader":"Other","entries":[{"thumbnails":[{"url":"https://i.test/v1.jpg"}]},{},{}]}"#,
        )
        .unwrap();
        let read = read_playlist(&bare).unwrap();
        assert_eq!((read.count, read.thumbnail.as_deref(), read.channel.as_deref()), (3, Some("https://i.test/v1.jpg"), Some("Other")));
        assert_eq!(read.duration, None, "no lengths given, none claimed");
        assert!(read.samples.is_empty(), "no links given, none made up");
        assert!(read_playlist(&serde_json::json!({"entries": []})).is_none());
    }

    #[test]
    fn a_channels_videos_are_counted_across_its_tabs() {
        let channel: serde_json::Value = serde_json::from_str(
            r#"{"title":"Someone","playlist_count":2,"entries":[
                {"title":"Someone - Videos","entries":[{"id":"a","duration":100},{"id":"b","duration":200}]},
                {"title":"Someone - Shorts","entries":[{"id":"c","duration":30,"thumbnails":[{"url":"https://i.test/c.jpg"}]}]}]}"#,
        )
        .unwrap();
        let read = read_playlist(&channel).unwrap();
        assert_eq!((read.count, read.duration), (3, Some(330)), "the videos, not the two tabs");
    }

    #[test]
    fn the_file_being_fetched_is_followed_across_its_streams() {
        let mut f = Following::default();
        let at = |f: &mut Following, line: &str| f.read(line, true).map(|c| (c.name, c.downloaded, c.total, c.index, c.count));
        assert_eq!(at(&mut f, "RUSH|100|1000|NA|2|27|abc|First | video"), Some(("First | video".into(), 100, Some(1000), Some(2), Some(27))));
        assert_eq!(at(&mut f, "RUSH|1000|1000|NA|2|27|abc|First | video").unwrap().1, 1000);
        // the sound follows the picture: its bytes are added on
        assert_eq!(at(&mut f, "RUSH|50|NA|200.0|2|27|abc|First | video"), Some(("First | video".into(), 1050, Some(1200), Some(2), Some(27))));
        // the next video starts from nothing; without a title its id stands in
        assert_eq!(at(&mut f, "RUSH|10|NA|NA|3|27|def|NA"), Some(("def".into(), 10, None, Some(3), Some(27))));
        assert_eq!(f.read("RUSH|10|NA|NA|3|27|def|NA", false).map(|c| (c.index, c.count)), Some((None, None)));
        assert_eq!(f.read("[download] something else", true), None);
        assert_eq!(f.read("RUSH|NA|NA|NA|NA|NA|x|y", true), None);
    }

    #[test]
    fn a_playlists_videos_are_listed_with_how_far_each_is() {
        let mut listed = Listed {
            items: read_listing(
                "a\t61.0\thttps://i.test/a.jpg\thttps://v.test/a\tSomeone\tFirst | one\n\
                 b\tNA\tNA\tNA\tNA\tNA\n\nNA\t1\tNA\tNA\tNA\tgone\n\
                 a\t61\tNA\tNA\tNA\tFirst again\nc\t5\tNA\tNA\tNA\tThird\nd\t7\tNA\tNA\tNA\tFourth\n",
            ),
            ..Default::default()
        };
        let titles: Vec<&str> = listed.items.iter().map(|i| i.title.as_str()).collect();
        assert_eq!(titles, ["First | one", "b", "Third", "Fourth"], "no repeats, an id for a missing title");
        assert_eq!(
            listed.items[0],
            ListedItem {
                id: "a".into(),
                title: "First | one".into(),
                duration: Some(61),
                thumbnail: Some("https://i.test/a.jpg".into()),
                url: Some("https://v.test/a".into()),
                channel: Some("Someone".into()),
            }
        );
        assert_eq!(listed.items[1], ListedItem { id: "b".into(), title: "b".into(), ..Default::default() });

        let base = std::env::temp_dir().join(format!("rush-listing-test-{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join(ARCHIVE), "youtube a\nyoutube c\n").unwrap();
        std::fs::write(base.join("001 - First one [a].mp4"), b"12345").unwrap();
        listed.done = fetched(&base);

        let mut f = Following::default();
        let now = f.read("RUSH|40|100|NA|2|4|b|NA", true).unwrap();
        let states = |l: &Listed, c| l.show(c, None).into_iter().map(|i| i.state).collect::<Vec<_>>();
        use ItemState::*;
        assert_eq!(states(&listed, Some(&now)), [Done, Downloading, Done, Waiting]);
        let shown = listed.show(Some(&now), Some(&base));
        assert_eq!(shown[1].size, Some(40));
        assert_eq!((shown[0].file.as_deref(), shown[0].size), (Some("001 - First one [a].mp4"), Some(5)), "the finished file is found by its id");
        assert_eq!(shown[0].thumbnail.as_deref(), Some("https://i.test/a.jpg"));
        let _ = std::fs::remove_dir_all(&base);
        listed.finished = true;
        assert_eq!(states(&listed, None), [Done, Missed, Done, Missed]);
    }

    #[test]
    fn listed_files_are_told_apart_as_photos_and_videos() {
        assert_eq!(listed("kind=jpg"), Some(false));
        assert_eq!(listed("kind=MP4 "), Some(true));
        assert_eq!(listed("kind=m3u8"), Some(true));
        assert_eq!(listed("kind="), Some(false), "a file of no known kind is a photo");
        assert_eq!(listed("https://bsky.test/profile/someone/media"), None, "not a file");
        assert_eq!(listed("   "), None);
    }

    #[test]
    fn files_are_sorted_into_images_and_videos() {
        let base = std::env::temp_dir().join(format!("rush-profile-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let folder = base.join("someone (Site)");
        for dir in [IMAGES, VIDEOS, INCOMING] {
            std::fs::create_dir_all(folder.join(dir)).unwrap();
        }
        for name in ["a.jpg", "b.MP4", "c.webp", "notes.txt", "d.mp4.part"] {
            std::fs::write(folder.join(INCOMING).join(name), b"12345").unwrap();
        }
        std::fs::write(folder.join(".rush-archive.sqlite3"), b"x").unwrap();
        for name in ["a.jpg", "b.MP4", "c.webp", "notes.txt"] {
            file_away(&folder, &folder.join(INCOMING).join(name));
        }
        assert!(folder.join(IMAGES).join("a.jpg").is_file());
        assert!(folder.join(IMAGES).join("c.webp").is_file());
        assert!(folder.join(VIDEOS).join("b.MP4").is_file());
        assert!(folder.join("notes.txt").is_file());
        // four finished files, plus the five bytes still on their way
        assert_eq!(tally(&folder), (25, 4));
        let kept: Vec<String> = on_disk(&folder).into_iter().map(|i| i.title).collect();
        assert_eq!(kept, ["a.jpg", "b.MP4", "c.webp", "notes.txt"], "the finished files, not the working ones");

        // Cancelling keeps what is complete and removes the rest.
        discard_unfinished(&base, "someone (Site)");
        assert!(!folder.join(INCOMING).exists());
        assert!(!folder.join(".rush-archive.sqlite3").exists());
        assert!(folder.join(IMAGES).join("a.jpg").is_file());
        discard_unfinished(&base, "../elsewhere");
        let _ = std::fs::remove_dir_all(&base);
    }
}
