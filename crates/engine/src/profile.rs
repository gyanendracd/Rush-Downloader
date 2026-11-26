//! A social media profile link downloads everything the profile has posted,
//! into a folder of its own with `Images` and `Videos` inside. gallery-dl
//! does the fetching for most sites; yt-dlp for the video-only ones.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use regex::Regex;
use serde::Serialize;
use crate::media::{command, debug, find_tool, kill_tree, OutputLines};
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
            // Pornhub, under any of its addresses ("pornhub.org", "de.pornhub.com"):
            // a model's, a performer's, a channel's or a member's page lists their videos.
            let site = host.rsplit('.').nth(1).unwrap_or_default();
            if matches!(site, "pornhub" | "pornhubpremium") {
                let whose = matches!(lower.as_str(), "model" | "pornstar" | "channels" | "users");
                return (whose && segments.len() >= 2).then(|| found("Pornhub", segments[1]))?;
            }
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
    matches!(site, "YouTube" | "TikTok" | "Pornhub" | PLAYLIST)
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
        .arg(format!("{LISTED}{{extension}}\t{WHOSE_PICTURE}\t{WHOSE_NAME}"))
        .arg("--range")
        .arg(format!("1-{COUNT_UP_TO}"));
    inner.add_sign_in(&mut cmd, None);
    cmd.arg(url).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("could not start gallery-dl: {e}"))?;

    let last_error = Arc::new(Mutex::new(None::<String>));
    if let Some(stderr) = child.stderr.take() {
        let last_error = last_error.clone();
        tokio::spawn(async move {
            let mut lines = OutputLines::new(stderr);
            while let Some(line) = lines.next_line().await {
                if line.contains("[error]") || line.starts_with("ERROR:") {
                    *last_error.lock().unwrap() = Some(line);
                }
            }
        });
    }
    let stdout = child.stdout.take().expect("stdout is piped");
    let mut lines = OutputLines::new(stdout);
    let until = tokio::time::Instant::now() + COUNT_FOR;
    let (mut videos, mut photos, mut cut_off) = (0u64, 0u64, false);
    let mut face = Face::default();
    loop {
        match tokio::time::timeout_at(until, lines.next_line()).await {
            Ok(Some(line)) => {
                let mut fields = line.split('\t');
                match listed(fields.next().unwrap_or_default()) {
                    Some(true) => videos += 1,
                    Some(false) => photos += 1,
                    None => continue,
                }
                // Whose profile it is comes with every file; the first say is kept.
                let mut said = || fields.next().map(str::trim).filter(|f| !f.is_empty() && *f != "None");
                let (picture, name) = (said().filter(|p| p.starts_with("http")), said());
                face.picture = face.picture.or(picture.map(str::to_string));
                face.name = face.name.or(name.map(str::to_string));
            }
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
        title: face.name.unwrap_or_else(|| profile.name.clone()),
        count: videos,
        photos,
        more: cut_off || videos + photos >= COUNT_UP_TO,
        duration: None,
        thumbnail: face.picture,
        channel: None,
        samples: Vec::new(),
    })
}

/// Where gallery-dl keeps a profile's picture and the name it shows: each
/// site has its own words for them, and the first one that is there is used.
/// `user` is the profile itself; `author` can be someone it reposted.
const WHOSE_PICTURE: &str =
    "{user[avatar]|user[profile_image]|author[avatar]|author[profile_image]|author[usericon]|blog[avatar]}";
const WHOSE_NAME: &str = "{user[displayName]|user[nick]|author[displayName]|author[nick]|fullname|blog[title]|\
     owner[realname]|owner[username]|author[username]}";

/// A profile's picture, the name it shows and what it says about itself,
/// where they could be found.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ProfileFace {
    pub name: Option<String>,
    pub picture: Option<String>,
    /// The profile's own line about itself, or the site's line about it.
    pub about: Option<String>,
    /// The wide picture across the top of the profile.
    pub banner: Option<String>,
    /// How many posts it has made, and how many of them carry a photo or video.
    pub posts: Option<u64>,
    pub media: Option<u64>,
    pub followers: Option<u64>,
    pub following: Option<u64>,
    /// The day it joined the site, as `YYYY-MM-DD`.
    pub joined: Option<String>,
}

type Face = ProfileFace;

/// Sites put a profile's name and picture in their page for link previews
/// (the card a chat app shows for a pasted link), and show them to a preview
/// reader whether or not anyone is signed in. This is how one introduces itself.
const PREVIEW_AGENT: &str = "facebookexternalhit/1.1 (+http://www.facebook.com/externalhit_uatext.php)";
const FACE_FOR: Duration = Duration::from_secs(10);

/// Reads the profile's page for its link preview. Nothing here is needed
/// for downloading, so any trouble simply leaves the card plainer.
async fn face(inner: &Arc<Inner>, url: &str, profile: &ProfileRef) -> Face {
    let page = async {
        let mut answer = inner
            .client
            .get(url)
            .header(reqwest::header::USER_AGENT, PREVIEW_AGENT)
            .header(reqwest::header::ACCEPT, "text/html")
            .header(reqwest::header::ACCEPT_LANGUAGE, "en-US,en;q=0.8")
            .send()
            .await
            .ok()
            .filter(|a| a.status().is_success())?;
        // The preview is in the page's head; the rest is not waited for.
        let mut html: Vec<u8> = Vec::new();
        while let Ok(Some(chunk)) = answer.chunk().await {
            let from = html.len().saturating_sub(6);
            html.extend_from_slice(&chunk);
            if html.len() > 3 << 20 || html[from..].windows(7).any(|w| w.eq_ignore_ascii_case(b"</head>")) {
                break;
            }
        }
        Some(String::from_utf8_lossy(&html).to_string())
    };
    match tokio::time::timeout(FACE_FOR, page).await {
        Ok(Some(html)) => read_face(&html, profile),
        _ => Face::default(),
    }
}

/// One `<meta>` tag's value, by the name the page gives it.
fn meta(html: &str, wanted: &str) -> Option<String> {
    static TAG: OnceLock<Regex> = OnceLock::new();
    static PART: OnceLock<Regex> = OnceLock::new();
    let tag = TAG.get_or_init(|| Regex::new(r"(?is)<meta\s[^>]*>").unwrap());
    let part = PART.get_or_init(|| Regex::new(r#"(?is)([a-z:_-]+)\s*=\s*(?:"([^"]*)"|'([^']*)')"#).unwrap());
    tag.find_iter(html).find_map(|found| {
        let (mut named, mut content) = (false, None);
        for p in part.captures_iter(found.as_str()) {
            let value = p.get(2).or(p.get(3)).map(|v| v.as_str()).unwrap_or_default();
            match p[1].to_ascii_lowercase().as_str() {
                "property" | "name" => named = value.eq_ignore_ascii_case(wanted),
                "content" => content = Some(value),
                _ => {}
            }
        }
        content.filter(|_| named).map(plain).filter(|c| !c.is_empty())
    })
}

/// Text as it reads, from text as a page writes it ("&amp;" for "&").
fn plain(text: &str) -> String {
    static CODE: OnceLock<Regex> = OnceLock::new();
    let code = CODE.get_or_init(|| Regex::new(r"&(#[0-9]{1,7}|#[xX][0-9a-fA-F]{1,6}|[a-zA-Z]{2,6});").unwrap());
    code.replace_all(text, |c: &regex::Captures| {
        let number = |digits: &str, radix| u32::from_str_radix(digits, radix).ok().and_then(char::from_u32);
        let letter = match &c[1] {
            "amp" => Some('&'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "nbsp" => Some(' '),
            other => match other.strip_prefix('#') {
                Some(hex) if hex.starts_with(['x', 'X']) => number(&hex[1..], 16),
                Some(digits) => number(digits, 10),
                None => None,
            },
        };
        letter.map(String::from).unwrap_or_else(|| c[0].to_string())
    })
    .trim()
    .to_string()
}

/// The name and picture in a profile page's link preview.
fn read_face(html: &str, profile: &ProfileRef) -> Face {
    // A preview's title wraps the name: "Name (@handle) on X", "Name | Site".
    let name = meta(html, "og:title").and_then(|title| {
        let mut name = title.as_str();
        for cut in [" (@", " (", " | ", " \u{2022} ", " - Profile"] {
            name = name.find(cut).map_or(name, |at| &name[..at]);
        }
        let name = name.strip_suffix(&format!(" on {}", profile.site)).unwrap_or(name).trim();
        let lower = name.to_lowercase();
        // A page that only asks to sign in, or says nothing about whose it is.
        let empty = name.is_empty()
            || name.starts_with('@')
            || lower == profile.site.to_lowercase()
            || ["log in", "login", "sign up", "sign in", "check out", "page not found"].iter().any(|w| lower.contains(w));
        (!empty).then(|| name.chars().take(80).collect::<String>())
    });
    // Only where the preview's picture is the profile's own: elsewhere it is
    // a banner, the site's logo or one of the posts.
    let own = matches!(profile.site.as_str(), "Instagram" | "X" | "Facebook" | "DeviantArt" | "YouTube" | "TikTok");
    let picture = meta(html, "og:image").filter(|p| own && p.starts_with("http"));
    // One line of it: a card has no room for more.
    let about = meta(html, "og:description")
        .map(|text| text.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|text| !text.is_empty() && name.is_some())
        .map(|text| match text.chars().count() > 160 {
            true => format!("{}\u{2026}", text.chars().take(159).collect::<String>().trim_end()),
            false => text,
        });
    Face { name, picture, about, ..Face::default() }
}

/// Looks up what a playlist or a profile holds: its name, picture, how many
/// videos and photos, and how long the videos run. Nothing is downloaded.
pub(crate) async fn playlist_info(inner: &Arc<Inner>, url: &str) -> Result<PlaylistInfo, String> {
    // Whose profile it is (name, picture) is asked for on its own, by
    // `profile_face`: it is quick, and works where the counting does not.
    if let Some(profile) = detect(url).filter(|p| !video_site(&p.site)) {
        return profile_counts(inner, url, &profile).await;
    }
    listed_info(inner, url).await
}

/// Whose profile a link points at: the name it shows, its picture and what
/// it says about itself. Nothing for a link that is not a profile. The
/// site's downloader is not needed for it, so it is there within a second
/// or two, signed in or not.
pub(crate) async fn profile_face(inner: &Arc<Inner>, url: &str) -> ProfileFace {
    match detect(url).filter(|p| p.site != PLAYLIST) {
        Some(profile) => face(inner, url, &profile).await,
        None => ProfileFace::default(),
    }
}

/// Where gallery-dl answers "who is this" for a profile, for the sites that
/// tell a visitor who is not signed in.
fn about_link(profile: &ProfileRef) -> Option<String> {
    match profile.site.as_str() {
        "X" => Some(format!("https://x.com/{}/info", profile.name)),
        "Bluesky" => Some(format!("https://bsky.app/profile/{}/info", profile.name)),
        _ => None,
    }
}

const STATS_FOR: Duration = Duration::from_secs(20);

/// Everything a profile shows a visitor who is not signed in: its name and
/// pictures, its line about itself, how many posts, followers and follows
/// it has, and when it joined. Asked as such a visitor, with no sign-in
/// sent, so it works whether or not the user has one, and never touches
/// their account. Empty for sites that do not tell.
pub(crate) async fn profile_stats(inner: &Arc<Inner>, url: &str) -> ProfileFace {
    let nothing = ProfileFace::default;
    let Some(link) = detect(url).as_ref().and_then(about_link) else { return nothing() };
    let Some(tool) = find_tool("gallery-dl") else { return nothing() };
    let mut cmd = command(&tool);
    cmd.env("PYTHONUTF8", "1").env("PYTHONIOENCODING", "utf-8").arg("-j");
    // The lookup still goes the way the app's other requests do.
    if let Some(proxy) = inner.dns.proxy() {
        cmd.arg("--proxy").arg(format!("http://{proxy}"));
    }
    cmd.arg(link).stdout(Stdio::piped()).stderr(Stdio::null());
    let Ok(child) = cmd.spawn() else { return nothing() };
    let Ok(Ok(out)) = tokio::time::timeout(STATS_FOR, child.wait_with_output()).await else { return nothing() };
    serde_json::from_slice::<serde_json::Value>(&out.stdout).ok().as_ref().and_then(read_stats).unwrap_or_default()
}

/// Reads a profile out of what gallery-dl printed about it. Each site has
/// its own words for the same things; the first that is there is used.
fn read_stats(printed: &serde_json::Value) -> Option<ProfileFace> {
    const COUNTS: [&str; 4] = ["followers_count", "followersCount", "statuses_count", "postsCount"];
    // The profile is the first object that counts followers or posts.
    fn find(value: &serde_json::Value) -> Option<&serde_json::Map<String, serde_json::Value>> {
        match value {
            serde_json::Value::Object(map) if COUNTS.iter().any(|key| map.contains_key(*key)) => Some(map),
            serde_json::Value::Object(map) => map.values().find_map(find),
            serde_json::Value::Array(list) => list.iter().find_map(find),
            _ => None,
        }
    }
    let user = find(printed)?;
    let text = |keys: &[&str]| {
        keys.iter()
            .find_map(|key| user.get(*key)?.as_str())
            .map(|found| found.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|found| !found.is_empty())
    };
    let link = |keys: &[&str]| text(keys).filter(|found| found.starts_with("http"));
    let count = |keys: &[&str]| keys.iter().find_map(|key| user.get(*key)?.as_u64());
    Some(ProfileFace {
        name: text(&["nick", "displayName", "full_name"]),
        picture: link(&["profile_image", "avatar"]),
        about: text(&["description"]),
        banner: link(&["profile_banner", "banner"]),
        posts: count(&["statuses_count", "postsCount"]),
        media: count(&["media_count"]),
        followers: count(&["followers_count", "followersCount"]),
        following: count(&["following_count", "friends_count", "followsCount"]),
        // "2017-12-14 17:55:51" and "2023-04-12T04:53:57.057Z" both start with the day.
        joined: text(&["date", "createdAt"])
            .and_then(|at| at.get(..10).map(str::to_string))
            .filter(|day| day.as_bytes().get(4) == Some(&b'-')),
    })
}

/// What yt-dlp lists at a link: a playlist's or a channel's videos.
async fn listed_info(inner: &Arc<Inner>, url: &str) -> Result<PlaylistInfo, String> {
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
        let files: Vec<(String, u64, PathBuf)> = folder
            .map(|f| [f.to_path_buf(), f.join(VIDEOS)])
            .into_iter()
            .flatten()
            .flat_map(|dir| files_in(&dir))
            .filter_map(|(path, size)| Some((path.file_name()?.to_string_lossy().to_string(), size, path)))
            .collect();
        self.items
            .iter()
            .map(|item| {
                let now = current.filter(|c| c.id == item.id);
                let done = self.done.contains(&item.id);
                let marked = format!("[{}].", item.id);
                let file = files.iter().find(|(name, _, _)| done && name.contains(&marked));
                ItemInfo {
                    title: item.title.clone(),
                    // Fetched, and its file is there. Once the run is over,
                    // one that was fetched and is not in the folder that was
                    // looked in has been deleted.
                    state: if done && (file.is_some() || !self.finished || folder.is_none()) {
                        ItemState::Done
                    } else if done {
                        ItemState::Deleted
                    } else if now.is_some() {
                        ItemState::Downloading
                    } else if self.finished {
                        ItemState::Missed
                    } else {
                        ItemState::Waiting
                    },
                    size: file.map(|(_, size, _)| *size).or(now.map(|c| c.downloaded)),
                    file: file.map(|(name, _, _)| name.clone()),
                    path: file.map(|(_, _, path)| path.clone()),
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
    // A playlist or channel is shown by its first video's picture.
    if let Some(picture) = items.iter().find_map(|item| item.thumbnail.clone()) {
        inner.update(id, |r| r.thumbnail = r.thumbnail.take().or(Some(picture)));
    }
    inner.lists.lock().unwrap().entry(id).or_default().items = items;
}

/// What a profile's folder holds, as finished items in name order.
pub(crate) fn on_disk(folder: &Path) -> Vec<ItemInfo> {
    let mut files: Vec<(String, u64, PathBuf)> = [folder.to_path_buf(), folder.join(IMAGES), folder.join(VIDEOS)]
        .iter()
        .flat_map(|dir| files_in(dir))
        .filter_map(|(path, size)| Some((path.file_name()?.to_string_lossy().to_string(), size, path)))
        .filter(|(name, _, _)| !name.starts_with(".rush-"))
        .collect();
    files.sort();
    files
        .into_iter()
        .map(|(title, size, path)| ItemInfo {
            file: Some(title.clone()),
            title,
            state: ItemState::Done,
            size: Some(size),
            // The file itself is its picture: a photo as it is, a video by its first frame.
            path: Some(path),
            thumbnail: None,
            duration: None,
            channel: None,
            url: None,
        })
        .collect()
}

/// yt-dlp is asked to report each file's progress on lines marked like this.
/// The title is asked for as JSON, which is plain ASCII: on Windows these
/// lines are not written as UTF-8, and a title's other letters would be lost.
const PROGRESS: &str = "RUSH|";
const PROGRESS_TEMPLATE: &str = "download:RUSH|%(progress.downloaded_bytes)s|%(progress.total_bytes)s|\
     %(progress.total_bytes_estimate)s|%(info.playlist_index)s|%(info.n_entries)s|%(info.id)s|%(info.title)j";

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
        let title = title.map(|t| serde_json::from_str::<String>(t).unwrap_or_else(|_| t.to_string()));
        let title = title.as_deref().map(str::trim).filter(|t| !t.is_empty());
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
    let (path, size) = arriving_files(incoming).into_iter().find(|(path, _)| extension(path) == "part")?;
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

/// The files in the incoming folder, each with the size it has right now.
/// A folder's listing is not enough for these: on Windows it gives the size
/// a file had when it was last closed, so one that is still being written
/// shows as empty for as long as it stays open (minutes, for a large video).
/// Asking about each file itself gives its real size.
fn arriving_files(dir: &Path) -> Vec<(PathBuf, u64)> {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            let meta = std::fs::metadata(&path).ok()?;
            meta.is_file().then(|| (path, meta.len()))
        })
        .collect()
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
    bytes += arriving_files(&folder.join(INCOMING)).iter().map(|(_, s)| s).sum::<u64>();
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
    // The connection was cut while it was being set up: not the site saying
    // no, but something on the way to it. Said first, because the wording of
    // such an error can also contain words the checks below look for.
    if ["connection closed abruptly", "ssl_error_syscall", "connection reset", "connection was reset"]
        .iter()
        .any(|w| lower.contains(w))
    {
        return "The connection to this site was cut off before it was set up. That is how it looks \
                when a network blocks a site. Secure DNS cannot get past that; a different network \
                or a VPN can"
            .into();
    }
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
        let asked = std::time::Instant::now();
        let base = match playlist {
            true => playlist_title(inner, live, &tool, &rec.url).await.unwrap_or_else(|| rec.filename.clone()),
            false => rec.filename.clone(),
        };
        debug(|| format!("folder name {base:?} settled after {:.1}s", asked.elapsed().as_secs_f64()));
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
        // With RUSHDOWNLOAD_DEBUG=verbose, yt-dlp says everything it does;
        // otherwise only how far along it is, and what went wrong.
        if std::env::var_os("RUSHDOWNLOAD_DEBUG").is_some_and(|v| v == "verbose") {
            cmd.arg("--verbose");
        } else {
            cmd.args(["--quiet", "--no-warnings"]);
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
            let mut lines = OutputLines::new(stderr);
            let began = std::time::Instant::now();
            while let Some(line) = lines.next_line().await {
                debug(|| format!("{:6.1}s {}", began.elapsed().as_secs_f64(), line.chars().take(300).collect::<String>()));
                if line.contains("[error]") || line.starts_with("ERROR:") {
                    *last_error.lock().unwrap() = Some(line);
                }
            }
        });
    }

    let stdout = child.stdout.take().expect("stdout is piped");
    let mut lines = OutputLines::new(stdout);
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
                Some(line) => {
                    let line = line.trim();
                    if by_ytdlp {
                        if !line.starts_with(PROGRESS) {
                            debug(|| format!("said: {}", line.chars().take(300).collect::<String>()));
                        }
                        if let Some(file) = following.read(line, playlist) {
                            // yt-dlp's own word that bytes are arriving: the
                            // download has started, whatever the folder shows yet.
                            if !started && file.downloaded > 0 {
                                started = true;
                                inner.update(id, |r| r.status = Status::Downloading);
                            }
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
                None => open = false,
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
    debug(|| format!("the downloader closed its output; in the folder now: {:?}", tally(&folder)));
    let status = tokio::select! {
        _ = live.cancel.cancelled() => {
            kill_tree(&mut child).await;
            return Err(Fail::Cancelled);
        }
        s = child.wait() => s.map_err(|e| Fail::Fatal(format!("the profile downloader failed: {e}")))?,
    };
    debug(|| format!("the downloader ended: {status}"));

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
        // one site under several addresses, and four kinds of page that list someone's videos
        is("https://www.pornhub.org/model/someone", "Pornhub", "someone");
        is("https://de.pornhub.com/pornstar/some-one/videos", "Pornhub", "some-one");
        is("https://www.pornhub.com/channels/somestudio", "Pornhub", "somestudio");
        is("https://pornhub.com/users/someone", "Pornhub", "someone");
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
            // one video, a list of everyone, and a look-alike address
            "https://www.pornhub.org/view_video.php?viewkey=abc123",
            "https://www.pornhub.org/model",
            "https://notpornhub.example.com/model/someone",
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
    fn a_profiles_name_and_picture_are_read_from_its_link_preview() {
        let on = |site: &str| ProfileRef { site: site.into(), name: "someone".into() };
        let page = r#"<html><head><title>x</title>
            <meta content="https://cdn.site.test/pic.jpg?a=1&amp;b=2" property="og:image" />
            <meta property='og:title' content='Some &quot;One&quot; &#38; Co (@someone) &#x2022; Instagram photos and videos'>
            <meta property="og:description" content="Makes things.
               Posts them here.">
            </head><body></body></html>"#;
        assert_eq!(
            read_face(page, &on("Instagram")),
            Face {
                name: Some("Some \"One\" & Co".into()),
                picture: Some("https://cdn.site.test/pic.jpg?a=1&b=2".into()),
                about: Some("Makes things. Posts them here.".into()),
                ..Face::default()
            }
        );
        // where the preview's picture is not the profile's own, only the name is taken
        assert_eq!(read_face(page, &on("Flickr")).picture, None);
        let titled = |title: &str, site: &str| {
            read_face(&format!(r#"<meta property="og:title" content="{title}">"#), &on(site)).name
        };
        assert_eq!(titled("NASA (@NASA) on X", "X").as_deref(), Some("NASA"));
        assert_eq!(titled("Team on DeviantArt", "DeviantArt").as_deref(), Some("Team"));
        assert_eq!(titled("Jane Doe (janedoe) - Profile | Pinterest", "Pinterest").as_deref(), Some("Jane Doe"));
        // a sign-in wall, or a title that names nobody
        assert_eq!(titled("Login \u{2022} Instagram", "Instagram"), None);
        assert_eq!(titled("Instagram", "Instagram"), None);
        assert_eq!(titled("@staff", "Tumblr"), None);
        assert_eq!(titled("Check out someone&#39;s Reddit profile", "Reddit"), None);
        assert_eq!(read_face("<html></html>", &on("X")), Face::default());
    }

    #[test]
    fn a_connection_cut_off_on_the_way_is_said_plainly() {
        // What yt-dlp reports when a network cuts the connection during the handshake.
        let cut = "ERROR: model/someone/videos: Unable to download webpage: Failed to perform, curl: (35) \
                   BoringSSL SSL_connect: Connection closed abruptly (SSL_ERROR_SYSCALL; error queue empty) \
                   in connection to www.site.test:443";
        let said = friendly(cut);
        assert!(said.contains("cut off before it was set up"), "{said}");
        assert!(said.contains("a different network or a VPN"), "{said}");
        // It is not mistaken for a sign-in wall, and a sign-in wall is not mistaken for it.
        assert!(!said.contains("signed-in"), "{said}");
        assert!(friendly("ERROR: [site] AuthRequired: authenticated cookies needed").contains("signed-in visitors"));
        assert_eq!(friendly("ERROR: [site] NotFoundError: Requested user could not be found"), "This profile was not found");
    }

    #[test]
    fn a_profiles_numbers_are_read_from_what_the_downloader_prints() {
        // X's words for them
        let x: serde_json::Value = serde_json::from_str(
            r#"[[2, {"category":"twitter","date":"2017-12-14 17:55:51","description":"Makes things.\n Posts them here.",
                "followers_count":94442,"following_count":166,"media_count":988,"name":"someone","nick":"Some One",
                "profile_banner":"https://img.site.test/banner/1","profile_image":"https://img.site.test/face.jpg",
                "statuses_count":4678}]]"#,
        )
        .unwrap();
        assert_eq!(
            read_stats(&x),
            Some(ProfileFace {
                name: Some("Some One".into()),
                picture: Some("https://img.site.test/face.jpg".into()),
                about: Some("Makes things. Posts them here.".into()),
                banner: Some("https://img.site.test/banner/1".into()),
                posts: Some(4678),
                media: Some(988),
                followers: Some(94442),
                following: Some(166),
                joined: Some("2017-12-14".into()),
            })
        );
        // Bluesky's, one level further in
        let sky: serde_json::Value = serde_json::from_str(
            r#"[[2, {"user": {"displayName":"Sky","avatar":"https://cdn.site.test/a","createdAt":"2023-04-12T04:53:57.057Z",
                "followersCount":12,"followsCount":3,"postsCount":40}}]]"#,
        )
        .unwrap();
        let read = read_stats(&sky).unwrap();
        assert_eq!((read.name.as_deref(), read.posts, read.followers, read.following), (Some("Sky"), Some(40), Some(12), Some(3)));
        assert_eq!((read.joined.as_deref(), read.media, read.banner), (Some("2023-04-12"), None, None));
        // an error instead of a profile
        assert_eq!(read_stats(&serde_json::json!([[-1, {"error": "NotFoundError"}]])), None);
        // which sites are asked at all
        let at = |link: &str| about_link(&detect(link).unwrap());
        assert_eq!(at("https://x.com/someone?s=20").as_deref(), Some("https://x.com/someone/info"));
        assert_eq!(at("https://bsky.app/profile/some.one").as_deref(), Some("https://bsky.app/profile/some.one/info"));
        assert_eq!(at("https://www.instagram.com/someone/"), None);
    }

    /// Needs internet: `cargo test -p rushdownload-engine --lib -- --ignored link_previews --nocapture`
    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn link_previews_of_real_profiles_are_read() {
        let dir = std::env::temp_dir().join(format!("rushdownload-face-{}", std::process::id()));
        let engine = crate::Engine::new(&dir, crate::Settings::new(dir.clone())).unwrap();
        let mut named = 0;
        for link in [
            "https://x.com/nasa",
            "https://www.instagram.com/instagram/",
            "https://www.facebook.com/nasa",
            "https://www.deviantart.com/deviantart",
            "https://www.tiktok.com/@nasa",
        ] {
            let seen = face(&engine.inner, link, &detect(link).unwrap()).await;
            println!("{link}: {seen:?}");
            named += (seen.name.is_some() && seen.picture.is_some()) as u32;
        }
        // RUSH_TEST_PROFILE_URL: one more profile to look at, through the engine's own door.
        if let Ok(link) = std::env::var("RUSH_TEST_PROFILE_URL") {
            println!("{link}: {:?}", engine.profile_face(&link).await);
            println!("its numbers: {:?}", engine.profile_stats(&link).await);
        }
        let _ = std::fs::remove_dir_all(&dir);
        // Sites change what they show; most of them still say whose page it is.
        assert!(named >= 3, "only {named} of 5 profiles showed a name and a picture");
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
        // a title arrives as JSON, with letters outside ASCII written as escapes
        assert_eq!(
            at(&mut f, r#"RUSH|5|NA|NA|4|27|ghi|"🎮 Part 3 – \"Pagoda\" | live""#).unwrap().0,
            "\u{1f3ae} Part 3 \u{2013} \"Pagoda\" | live"
        );
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
