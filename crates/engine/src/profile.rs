//! A social media profile link downloads everything the profile has posted,
//! into a folder of its own with `Images` and `Videos` inside. gallery-dl
//! does the fetching for most sites; yt-dlp for the video-only ones.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use regex::Regex;
use serde::{Deserialize, Serialize};
use crate::media::{command, debug, find_tool, kill_tree, OutputLines};
use crate::probe::{candidate, sanitize};
use crate::task::{ConnGuard, Fail, Live};
use crate::{CurrentFile, Inner, ItemInfo, ItemState, Record, Status};

/// A profile recognised in a link.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileRef {
    /// The site's name, e.g. "Instagram".
    pub site: String,
    /// The account's name as written in the link.
    pub name: String,
    /// Which program fetches it, for a profile found by asking the programs
    /// themselves (any site they know). `None` for the sites recognised by
    /// their links, where the site says which.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<Fetcher>,
}

/// The two programs that fetch whole profiles: yt-dlp for videos and sound,
/// gallery-dl for photos (and the videos among them).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Fetcher {
    Ytdlp,
    Gallerydl,
}

/// Whether yt-dlp fetches this profile (otherwise gallery-dl does).
fn uses_ytdlp(profile: &ProfileRef) -> bool {
    match profile.by {
        Some(by) => by == Fetcher::Ytdlp,
        None => video_site(&profile.site),
    }
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
            by: None,
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
/// A profile downloads into a new folder named after the profile itself
/// ("natgeo"), inside the usual download folder. Two profiles that share a
/// name (on different sites, say) do not clash: `claim_folder` adds a number.
pub(crate) fn folder_name(profile: &ProfileRef) -> String {
    sanitize(&profile.name)
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
    // A public Instagram profile with nobody signed in: what its own page
    // shows any visitor.
    let instagram = profile.site == "Instagram";
    if instagram && !crate::instagram::has_session(inner) {
        return instagram_public_counts(inner, profile).await;
    }
    let tool =find_tool("gallery-dl").ok_or("gallery-dl is not installed; install it under Settings, Advanced, Engines")?;
    let mut cmd = command(&tool);
    cmd.env("PYTHONUTF8", "1")
        .env("PYTHONIOENCODING", "utf-8")
        // Each file's kind is printed in place of fetching it; the address
        // alone does not always say whether it is a photo or a video.
        .arg("--print")
        .arg(format!("{LISTED}{{extension}}\t{WHOSE_PICTURE}\t{WHOSE_NAME}"))
        .arg("--range")
        .arg(format!("1-{COUNT_UP_TO}"));
    inner.add_sign_in(&mut cmd, None, &crate::ProxyChoice::Default);
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
        // A sign-in Instagram has ended is let go of (instagram.rs); the
        // profile is then read as any visitor sees it.
        if let Some(error) = said.as_deref().filter(|e| instagram && instagram_wants_sign_in(e)) {
            crate::instagram::sign_in_ended(inner, error).await;
            return instagram_public_counts(inner, profile).await;
        }
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
    /// Its name on the site, as in its link, without the "@".
    pub username: Option<String>,
    /// The links it lists: its website, its other accounts.
    pub links: Vec<String>,
    /// How many likes its posts have had together, where the site shows that.
    pub likes: Option<u64>,
    /// The counts are as the page writes them, rounded ("32K posts").
    pub rounded: bool,
}

type Face = ProfileFace;

/// Sites put a profile's name and picture in their page for link previews
/// (the card a chat app shows for a pasted link), and show them to a preview
/// reader whether or not anyone is signed in. This is how one introduces itself.
const PREVIEW_AGENT: &str = "facebookexternalhit/1.1 (+http://www.facebook.com/externalhit_uatext.php)";
const FACE_FOR: Duration = Duration::from_secs(10);

/// An ordinary browser, for the sites that turn a link-preview reader away
/// (SoundCloud answers it with "too many requests") but show a visitor the
/// same preview.
pub(crate) const BROWSER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36";

/// Reads the profile's page for its link preview: first as a link-preview
/// reader, and where that is turned away or shown nothing, once more as an
/// ordinary browser. Nothing here is needed for downloading, so any trouble
/// simply leaves the card plainer.
async fn face(inner: &Arc<Inner>, url: &str, profile: &ProfileRef) -> Face {
    // A page whose numbers are in its body is only shown in full to a browser.
    let agents = match reads_body(&profile.site) {
        true => [BROWSER_AGENT, PREVIEW_AGENT],
        false => [PREVIEW_AGENT, BROWSER_AGENT],
    };
    for agent in agents {
        let seen = match tokio::time::timeout(FACE_FOR, page_head(inner, url, profile, agent)).await {
            Ok(Some(html)) => read_face(&html, profile),
            _ => Face::default(),
        };
        if seen.name.is_some() || seen.picture.is_some() {
            return seen;
        }
    }
    Face::default()
}

/// The start of a page, as far as its preview goes (further, for sites whose
/// profile picture is in the page itself), or `None` when the site says no.
async fn page_head(inner: &Arc<Inner>, url: &str, profile: &ProfileRef, agent: &str) -> Option<String> {
    let in_body = reads_body(&profile.site);
    let mut ask = inner
        .client()
        .ok()?
        .get(url)
        .header(reqwest::header::USER_AGENT, agent)
        .header(reqwest::header::ACCEPT, "text/html")
        .header(reqwest::header::ACCEPT_LANGUAGE, "en-US,en;q=0.8");
    if let Some(cookie) = page_cookie(&profile.site) {
        ask = ask.header(reqwest::header::COOKIE, cookie);
    }
    let mut answer = ask.send().await.ok().filter(|a| a.status().is_success())?;
    let declared = answer
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    // The preview is in the page's head; the rest is not waited for.
    let mut html: Vec<u8> = Vec::new();
    while let Ok(Some(chunk)) = answer.chunk().await {
        let from = html.len().saturating_sub(6);
        html.extend_from_slice(&chunk);
        if html.len() > 3 << 20 || (!in_body && html[from..].windows(7).any(|w| w.eq_ignore_ascii_case(b"</head>"))) {
            break;
        }
    }
    Some(page_text(&html, declared.as_deref()))
}

/// A page's text, read in the encoding it is written in: the one its server
/// names, else the one the page names in its head, else UTF-8. (VK, for one,
/// writes its pages in the old Windows Cyrillic encoding.)
fn page_text(bytes: &[u8], content_type: Option<&str>) -> String {
    static CHARSET: OnceLock<Regex> = OnceLock::new();
    let charset = CHARSET.get_or_init(|| Regex::new(r#"(?i)charset\s*=\s*["']?([\w:.-]+)"#).unwrap());
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]);
    let named = content_type
        .and_then(|t| charset.captures(t))
        .or_else(|| charset.captures(&head))
        .and_then(|c| encoding_rs::Encoding::for_label(c[1].as_bytes()));
    let (text, _, _) = named.unwrap_or(encoding_rs::UTF_8).decode(bytes);
    text.into_owned()
}

/// Sites whose profile picture is in the page itself rather than in its
/// link preview (where theirs is one of the videos, or the site's logo).
fn avatar_in_body(site: &str) -> bool {
    site == "Pornhub"
}

/// Sites whose page is read past its head: for the profile's picture, or
/// for the numbers, description and links its header shows (YouTube).
fn reads_body(site: &str) -> bool {
    avatar_in_body(site) || site == "YouTube"
}

/// A cookie a site wants before it shows a page at all: an age check, or a
/// consent page (answered "no" to everything optional) that would otherwise
/// stand in front of the profile.
fn page_cookie(site: &str) -> Option<&'static str> {
    match site {
        "Pornhub" => Some("age_verified=1; accessAgeDisclaimerPH=1; accessAgeDisclaimerUK=1"),
        "YouTube" => Some("SOCS=CAI"),
        _ => None,
    }
}

/// The profile's own picture as the page shows it: the `<img>` the page
/// marks as the avatar (by its id, or its class), at its full address.
fn avatar_img(html: &str) -> Option<String> {
    static TAG: OnceLock<Regex> = OnceLock::new();
    static PART: OnceLock<Regex> = OnceLock::new();
    let tag = TAG.get_or_init(|| Regex::new(r"(?is)<img\s[^>]*>").unwrap());
    let part = PART.get_or_init(|| Regex::new(r#"(?is)([a-z:_-]+)\s*=\s*(?:"([^"]*)"|'([^']*)')"#).unwrap());
    tag.find_iter(html).find_map(|found| {
        let (mut marked, mut src, mut lazy) = (false, None, None);
        for p in part.captures_iter(found.as_str()) {
            let value = p.get(2).or(p.get(3)).map(|v| v.as_str()).unwrap_or_default();
            match p[1].to_ascii_lowercase().as_str() {
                "id" | "class" => marked |= value.to_ascii_lowercase().contains("avatar"),
                "src" => src = Some(value),
                "data-src" | "data-image" => lazy = Some(value),
                _ => {}
            }
        }
        // A picture loaded later carries its address in `data-src`; `src` is then a blank stand-in.
        let address = plain(lazy.filter(|l| l.starts_with("http")).or(src)?);
        (marked && address.starts_with("http")).then_some(address)
    })
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
    // A preview's title wraps the name: "Name (@handle) on X", "Name | Site",
    // "Name's Porn Videos | Pornhub".
    let name = meta(html, "og:title").and_then(|title| {
        let mut name = title.as_str();
        for cut in [" (@", " (", " | ", " \u{2022} ", " \u{2014} ", " - Profile", " - Pornhub", "'s Porn Videos", " Porn Videos"] {
            name = name.find(cut).map_or(name, |at| &name[..at]);
        }
        let name = name.strip_suffix(&format!(" on {}", profile.site)).unwrap_or(name).trim();
        // "Name - Site": the site's own name, after a dash, is not part of whose page it is.
        let name = match name.rsplit_once(" - ") {
            Some((before, after)) if after.trim().eq_ignore_ascii_case(&profile.site) => before.trim(),
            _ => name,
        };
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
    // A collection found on a site the app has no rules for: its preview
    // picture is the best there is.
    let own = profile.by.is_some()
        || matches!(
            profile.site.as_str(),
            "Instagram" | "X" | "Facebook" | "DeviantArt" | "YouTube" | "TikTok" | "Pornhub"
        );
    // The site's own logo or stand-in picture says nothing about whose page it is.
    let plain = |p: &String| ["logo", "default", "placeholder", "no-avatar", "noavatar"].iter().any(|w| p.to_ascii_lowercase().contains(w));
    let picture = avatar_in_body(&profile.site)
        .then(|| avatar_img(html))
        .flatten()
        .or_else(|| meta(html, "og:image").filter(|p| own && p.starts_with("http")))
        .filter(|p| !plain(p));
    let mut face = Face { name, picture, username: Some(profile.name.clone()), ..Face::default() };
    // Many sites write the numbers into the line a link preview shows:
    // "267M Followers, 196 Following, 32K Posts - ...".
    let described = [meta(html, "description"), meta(html, "og:description")];
    for text in described.iter().flatten() {
        read_counts(text, &mut face);
    }
    if profile.site == "YouTube" {
        read_youtube(html, &mut face);
    }
    if face.about.is_none() && face.name.is_some() {
        face.about = described.iter().flatten().find_map(|text| bio_in(text));
    }
    if let Some(about) = &face.about {
        for found in links_in(about) {
            if !face.links.iter().any(|l| l.eq_ignore_ascii_case(&found)) {
                face.links.push(found);
            }
        }
        face.about = Some(clip(about, ABOUT_MOST));
    }
    face
}

/// The most of a profile's description the card is given.
const ABOUT_MOST: usize = 600;

/// Text cut to at most `most` characters, ending in "…" when cut.
fn clip(text: &str, most: usize) -> String {
    match text.chars().count() > most {
        true => format!("{}\u{2026}", text.chars().take(most - 1).collect::<String>().trim_end()),
        false => text.to_string(),
    }
}

/// Text with its runs of spaces made one and its line breaks kept, with at
/// most one empty line between paragraphs.
fn tidy(text: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    for line in text.lines() {
        let line = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if line.is_empty() && out.last().is_none_or(|l| l.is_empty()) {
            continue;
        }
        out.push(line);
    }
    while out.last().is_some_and(|l| l.is_empty()) {
        out.pop();
    }
    out.join("\n")
}

/// A number as a page writes it: "1,234", "32K", "15.1M", "1.2B".
fn count_text(figure: &str, unit: Option<&str>) -> Option<u64> {
    let figure = figure.trim().trim_end_matches(['.', ',']);
    let scale = match unit.map(|u| u.to_ascii_uppercase()).as_deref() {
        Some("K") => 1e3,
        Some("M") => 1e6,
        Some("B") => 1e9,
        _ => 1.0,
    };
    let value: f64 = if scale > 1.0 {
        figure.replace(',', ".").parse().ok()?
    } else {
        // Without a unit, "1,234" and "1.234" are both thousands.
        figure.replace([',', '.', ' ', '\u{a0}'], "").parse().ok()?
    };
    Some((value * scale).round() as u64)
}

/// The numbers a profile's preview line gives: followers, following, posts
/// (videos, pins), likes. Only what is not known yet is filled in.
fn read_counts(text: &str, face: &mut Face) {
    static COUNT: OnceLock<Regex> = OnceLock::new();
    let count = COUNT.get_or_init(|| {
        Regex::new(r"(?i)(\d(?:[\d.,]*\d)?)\s*([KMB])?\+?\s+(followers?|following|subscribers?|posts?|videos?|pins?|likes?)\b").unwrap()
    });
    for c in count.captures_iter(text) {
        let Some(n) = count_text(&c[1], c.get(2).map(|u| u.as_str())) else { continue };
        let word = c[3].to_ascii_lowercase();
        let slot = match word.trim_end_matches('s') {
            "follower" | "subscriber" => &mut face.followers,
            "following" => &mut face.following,
            "post" | "video" | "pin" => &mut face.posts,
            "like" => &mut face.likes,
            _ => continue,
        };
        if slot.is_none() {
            *slot = Some(n);
            face.rounded = true;
        }
    }
}

/// The profile's own words in its preview line, without the site's: the
/// part in quotes after "on Instagram:", else the line once its numbers are
/// taken off, unless what is left is only the site's own wording.
fn bio_in(text: &str) -> Option<String> {
    static QUOTED: OnceLock<Regex> = OnceLock::new();
    static COUNTS: OnceLock<Regex> = OnceLock::new();
    let quoted = QUOTED.get_or_init(|| Regex::new(r#"(?s)\bon [^:"]{1,40}:\s*"(.+)"\s*$"#).unwrap());
    let counts = COUNTS.get_or_init(|| {
        Regex::new(r"(?i)^(?:\s*\d(?:[\d.,]*\d)?\s*[KMB]?\+?\s+(?:followers?|following|subscribers?|posts?|videos?|pins?|likes?)[,.]?)+\s*(?:[-\u{2013}\u{2014}|\u{b7}]\s*)?").unwrap()
    });
    let words = match quoted.captures(text) {
        Some(c) => c[1].to_string(),
        None => {
            let rest = counts.replace(text, "").to_string();
            // "See Instagram photos and videos from ...", "Watch the latest videos from ..."
            let lower = rest.trim().to_ascii_lowercase();
            if ["see ", "watch ", "check out ", "discover ", "explore "].iter().any(|w| lower.starts_with(w)) {
                return None;
            }
            rest
        }
    };
    Some(tidy(&words)).filter(|w| !w.is_empty())
}

/// Web addresses written in a profile's description, each once, at most ten.
fn links_in(text: &str) -> Vec<String> {
    static LINK: OnceLock<Regex> = OnceLock::new();
    let link = LINK.get_or_init(|| {
        Regex::new(r"(?i)\b(?:https?://[^\s<>()\[\]]+|(?:www\.)?[a-z0-9-]+(?:\.[a-z0-9-]+)*\.(?:com|org|net|io|co|me|gov|edu|tv|gg|ee|app|link|bio|ly|page|store|shop|in|uk|de|fr|jp|xyz|art|dev|blog|info)\b(?:/[^\s<>()\[\]]*)?)").unwrap()
    });
    let mut found: Vec<String> = Vec::new();
    for m in link.find_iter(text) {
        // "name@site.com" is an e-mail address, not a page.
        if text[..m.start()].ends_with('@') {
            continue;
        }
        let raw = m.as_str().trim_end_matches(['.', ',', ';', ':', '!', '?', '\'', '"']);
        let url = match raw.to_ascii_lowercase().starts_with("http") {
            true => raw.to_string(),
            false => format!("https://{raw}"),
        };
        if reqwest::Url::parse(&url).is_ok() && !found.iter().any(|f| f.eq_ignore_ascii_case(&url)) {
            found.push(url);
        }
        if found.len() == 10 {
            break;
        }
    }
    found
}

/// What a YouTube channel's page header shows: its subscribers and videos,
/// its description, its first link, and its handle.
fn read_youtube(html: &str, face: &mut Face) {
    static PART: OnceLock<Regex> = OnceLock::new();
    static ABOUT: OnceLock<Regex> = OnceLock::new();
    static LINK: OnceLock<Regex> = OnceLock::new();
    static HANDLE: OnceLock<Regex> = OnceLock::new();
    let part = PART.get_or_init(|| Regex::new(r#""content":"([^"]{1,40} (?:subscribers?|videos?))""#).unwrap());
    let about = ABOUT.get_or_init(|| Regex::new(r#""channelMetadataRenderer":\{[^{}]*?"description":"((?:[^"\\]|\\.)*)""#).unwrap());
    let link = LINK.get_or_init(|| Regex::new(r#""attribution":\{"attributionViewModel":\{"text":\{"content":"\s*([^"\s]+)"#).unwrap());
    let handle = HANDLE.get_or_init(|| Regex::new(r#""vanityChannelUrl":"https?://(?:www\.)?youtube\.com/@([^"/?]+)""#).unwrap());
    // The header's own numbers win over the preview line's.
    let mut header = Face::default();
    for c in part.captures_iter(html) {
        read_counts(&c[1], &mut header);
    }
    if header.followers.is_some() || header.posts.is_some() {
        face.followers = header.followers.or(face.followers);
        face.posts = header.posts.or(face.posts);
        face.rounded = true;
    }
    if let Some(c) = about.captures(html) {
        let text: Option<String> = serde_json::from_str(&format!("\"{}\"", &c[1])).ok();
        face.about = text.map(|t| tidy(&t)).filter(|t| !t.is_empty());
    }
    if let Some(c) = handle.captures(html) {
        face.username = Some(percent_encoding::percent_decode_str(&c[1]).decode_utf8_lossy().to_string());
    }
    if let Some(first) = link.captures(html).and_then(|c| links_in(&c[1]).into_iter().next()) {
        face.links.push(first);
    }
}

/// Looks up what a playlist or a profile holds: its name, picture, how many
/// videos and photos, and how long the videos run. Nothing is downloaded.
pub(crate) async fn playlist_info(inner: &Arc<Inner>, url: &str) -> Result<PlaylistInfo, String> {
    // Whose profile it is (name, picture) is asked for on its own, by
    // `profile_face`: it is quick, and works where the counting does not.
    if let Some(profile) = inner.resolve(url).filter(|p| !uses_ytdlp(p)) {
        return profile_counts(inner, url, &profile).await;
    }
    listed_info(inner, url).await
}

/// Whose profile a link points at: the name it shows, its picture and what
/// it says about itself. Nothing for a link that is not a profile. The
/// site's downloader is not needed for it, so it is there within a second
/// or two, signed in or not.
pub(crate) async fn profile_face(inner: &Arc<Inner>, url: &str) -> ProfileFace {
    match inner.resolve(url).filter(|p| p.site != PLAYLIST) {
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
    inner.add_proxy(&mut cmd, &crate::ProxyChoice::Default);
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
        // X calls the handle "name" (and the shown name "nick"); Bluesky, "handle".
        username: text(&["handle", "name"]).map(|h| h.trim_start_matches('@').to_string()),
        links: {
            let mut links: Vec<String> = link(&["url"]).into_iter().collect();
            for found in text(&["description"]).map(|d| links_in(&d)).unwrap_or_default() {
                if !links.iter().any(|l| l.eq_ignore_ascii_case(&found)) {
                    links.push(found);
                }
            }
            links
        },
        likes: None,
        rounded: false,
    })
}

/// How long each program is given to say what a link holds.
const ASK_FOR: Duration = Duration::from_secs(40);

/// Asks both programs, side by side, whether a link they know is a whole
/// collection: someone's videos, photos or tracks, or a set of them. One
/// that lists two or more items is one. This is how a profile is recognised
/// on any of the many sites the programs know, not only the ones whose
/// links the app reads itself (`detect`).
///
/// A single video stays a single video: yt-dlp is told not to widen a link
/// to a video into the playlist it was opened from, and its catch-all
/// reader for unknown pages is not taken at its word (an article with two
/// clips in it is not someone's profile).
pub(crate) async fn find_collection(inner: &Arc<Inner>, url: &str) -> Option<ProfileRef> {
    let parsed = reqwest::Url::parse(url).ok().filter(|u| matches!(u.scheme(), "http" | "https"))?;
    let (videos, photos) = tokio::join!(ask_ytdlp(inner, url), ask_gallerydl(inner, url));
    // Photos are gallery-dl's work; a list of videos (or sound) is yt-dlp's.
    let by = match (&videos, &photos) {
        (_, Some(true)) => Fetcher::Gallerydl,
        (Some(_), _) => Fetcher::Ytdlp,
        (None, Some(false)) => Fetcher::Gallerydl,
        (None, None) => return None,
    };
    let title = videos.flatten();
    Some(ProfileRef {
        site: site_name(&parsed),
        name: name_in_link(&parsed).or(title).unwrap_or_else(|| site_name(&parsed)),
        by: Some(by),
    })
}

/// yt-dlp's view: `Some(title)` when the link lists two or more items.
async fn ask_ytdlp(inner: &Arc<Inner>, url: &str) -> Option<Option<String>> {
    let tool = find_tool("yt-dlp")?;
    let mut cmd = command(&tool);
    cmd.args([
        "--encoding",
        "utf-8",
        "--no-colors",
        "--no-warnings",
        "--no-playlist",
        "--flat-playlist",
        "--playlist-end",
        "3",
        "-J",
    ]);
    inner.add_sign_in(&mut cmd, None, &crate::ProxyChoice::Default);
    cmd.arg("--").arg(url).stdout(Stdio::piped()).stderr(Stdio::piped());
    let child = cmd.spawn().ok()?;
    let out = tokio::time::timeout(ASK_FOR, child.wait_with_output()).await.ok()?.ok()?;
    // A failed look prints "null" as its answer: that is no answer.
    match serde_json::from_slice::<serde_json::Value>(&out.stdout).ok().filter(|v| v.is_object()) {
        Some(info) => read_ytdlp_answer(&info),
        // No answer, but the error names the reader that took the link: one
        // for someone's channel or videos means the link is a collection
        // the site would not list just now (a sign-in, or a bot check). It is
        // still shown as one, so that the user is told what it needs.
        None => collection_reader(&String::from_utf8_lossy(&out.stderr)).then_some(None),
    }
}

/// Whether yt-dlp's error came from one of its readers for a whole channel,
/// user, playlist or list of videos ("[vk:uservideos]", "[youtube:tab]",
/// "[PornHubPagedVideoList]"), rather than from one for a single video.
fn collection_reader(stderr: &str) -> bool {
    stderr.lines().filter(|line| line.starts_with("ERROR: [")).any(|line| {
        let reader = line["ERROR: [".len()..].split(']').next().unwrap_or_default().to_ascii_lowercase();
        reader != "generic"
            && ["user", "channel", "playlist", ":tab", "album", "profile", "videolist", "model", "pornstar", "creator", "videos"]
                .iter()
                .any(|word| reader.contains(word))
    })
}

fn read_ytdlp_answer(info: &serde_json::Value) -> Option<Option<String>> {
    let text = |key: &str| info.get(key).and_then(|v| v.as_str()).map(str::trim).filter(|v| !v.is_empty());
    let listed = info.get("entries").and_then(|e| e.as_array()).map_or(0, |e| e.len() as u64);
    let counted = info.get("playlist_count").and_then(|c| c.as_u64()).unwrap_or(0);
    let many = text("_type") == Some("playlist") && listed.max(counted) >= 2;
    let generic = text("extractor_key").is_some_and(|key| key.eq_ignore_ascii_case("generic"));
    (many && !generic).then(|| text("title").or(text("uploader")).or(text("channel")).map(str::to_string))
}

/// gallery-dl's view: `Some(photos)` when the link lists two or more files;
/// `photos` says whether any of them is not a video.
async fn ask_gallerydl(inner: &Arc<Inner>, url: &str) -> Option<bool> {
    let tool = find_tool("gallery-dl")?;
    let mut cmd = command(&tool);
    cmd.env("PYTHONUTF8", "1")
        .env("PYTHONIOENCODING", "utf-8")
        .args(["--range", "1-3", "--print", "{category}\t{subcategory}\t{extension}"]);
    inner.add_sign_in(&mut cmd, None, &crate::ProxyChoice::Default);
    cmd.arg(url).stdout(Stdio::piped()).stderr(Stdio::null());
    let child = cmd.spawn().ok()?;
    let out = tokio::time::timeout(ASK_FOR, child.wait_with_output()).await.ok()?.ok()?;
    read_gallerydl_answer(&String::from_utf8_lossy(&out.stdout))
}

fn read_gallerydl_answer(printed: &str) -> Option<bool> {
    let files: Vec<(&str, &str)> = printed
        .lines()
        .filter_map(|line| {
            let mut fields = line.trim().split('\t');
            let category = fields.next()?.trim();
            let _subcategory = fields.next()?;
            let extension = fields.next()?.trim();
            Some((category, extension))
        })
        // A plain link to a file, or the catch-all reader: not a collection.
        .filter(|(category, _)| !matches!(*category, "directlink" | "generic" | ""))
        .collect();
    (files.len() >= 2).then(|| files.iter().any(|(_, ext)| !VIDEO_EXT.contains(&ext.to_ascii_lowercase().as_str())))
}

/// A site's name from its address: "www.dailymotion.com" is "Dailymotion".
fn site_name(url: &reqwest::Url) -> String {
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    let labels: Vec<&str> = host.split('.').filter(|l| !l.is_empty()).collect();
    // The label before the ending ("co.uk" counts as one ending).
    let short_end = labels.len() >= 3 && labels[labels.len() - 2].len() <= 3 && labels[labels.len() - 1].len() == 2;
    let at = labels.len().saturating_sub(if short_end { 3 } else { 2 });
    let name = labels.get(at).copied().unwrap_or(host.as_str());
    let mut letters = name.chars();
    letters.next().map(|first| first.to_uppercase().chain(letters).collect()).unwrap_or_default()
}

/// Whose collection it is, as the link says: the first part of its path that
/// is not a word every site uses ("user", "channel", "videos").
fn name_in_link(url: &reqwest::Url) -> Option<String> {
    const SITE_WORDS: &[&str] = &[
        "user", "users", "u", "c", "channel", "channels", "profile", "profiles", "artist", "artists", "model",
        "models", "pornstar", "creator", "creators", "member", "members", "people", "author", "authors", "photos",
        "videos", "video", "tracks", "albums", "playlists", "playlist", "gallery", "galleries", "media", "posts",
        "sets", "collections", "collection", "browse", "en", "www",
    ];
    url.path_segments()?
        .map(|segment| segment.trim().trim_start_matches('@'))
        .find(|segment| !segment.is_empty() && !SITE_WORDS.contains(&segment.to_ascii_lowercase().as_str()))
        .map(|segment| {
            let readable = urlencoding_lossy(segment);
            sanitize(&readable).chars().take(60).collect::<String>()
        })
        .filter(|name| !name.is_empty())
}

/// "%20"-style escapes in a link turned back into letters, where they make sense.
fn urlencoding_lossy(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push(hi * 16 + lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

/// What yt-dlp lists at a link: a playlist's or a channel's videos.
async fn listed_info(inner: &Arc<Inner>, url: &str) -> Result<PlaylistInfo, String> {
    let tool = find_tool("yt-dlp").ok_or("yt-dlp is not installed; install it under Settings, Advanced, Engines")?;
    let mut cmd = command(&tool);
    cmd.args(["--encoding", "utf-8", "--no-colors", "--flat-playlist", "-J"]);
    inner.add_sign_in(&mut cmd, None, &crate::ProxyChoice::Default);
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
    inner.add_sign_in(&mut cmd, None, &live.route());
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
                    downloaded: None,
                    net: None,
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
    inner.add_sign_in(&mut cmd, None, &live.route());
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
            downloaded: None,
            net: None,
        })
        .collect()
}

/// yt-dlp is asked to report each file's progress on lines marked like this.
/// The title is asked for as JSON, which is plain ASCII: on Windows these
/// lines are not written as UTF-8, and a title's other letters would be lost.
const PROGRESS: &str = "RUSH|";
const PROGRESS_TEMPLATE: &str = "download:RUSH|%(progress.downloaded_bytes)s|%(progress.total_bytes)s|\
     %(progress.total_bytes_estimate)s|%(info.playlist_index)s|%(info.n_entries)s|%(info.thumbnail)s|%(info.id)s|%(info.title)j";

/// Follows the file yt-dlp is fetching. A video usually arrives as two
/// streams, picture then sound; the bytes of the first are carried over.
#[derive(Default)]
struct Following {
    id: String,
    /// The picture the site shows for the file now arriving, once read.
    thumbnail: Option<String>,
    /// Bytes of this file's streams that are already complete.
    base: u64,
    /// Bytes of the stream arriving now.
    stream: u64,
}

impl Following {
    /// Reads one progress line; `numbered` keeps the place in the playlist.
    fn read(&mut self, line: &str, numbered: bool) -> Option<CurrentFile> {
        let mut fields = line.strip_prefix(PROGRESS)?.splitn(8, '|');
        let number = |field: Option<&str>| field?.trim().parse::<f64>().ok().map(|v| v as u64);
        let done = number(fields.next())?;
        let exact = number(fields.next());
        let estimate = number(fields.next());
        let total = exact.or(estimate);
        let index = number(fields.next()).filter(|_| numbered);
        let count = number(fields.next()).filter(|_| numbered);
        let thumbnail = fields.next()?.trim();
        let id = fields.next()?.trim();
        let title = fields.next().map(str::trim).filter(|t| !t.is_empty() && *t != "NA");
        let title = title.map(|t| serde_json::from_str::<String>(t).unwrap_or_else(|_| t.to_string()));
        let title = title.as_deref().map(str::trim).filter(|t| !t.is_empty());
        if id != self.id {
            *self = Following {
                id: id.to_string(),
                thumbnail: Some(thumbnail).filter(|t| t.starts_with("http")).map(str::to_string),
                ..Default::default()
            };
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

/// How long a part-fetched file may stay the same size and still count as
/// arriving. gallery-dl leaves behind the piece of a try it gave up on (a
/// video's stream that failed, before it fell back to another copy), and
/// that piece never grows again.
const STILL_ARRIVING: Duration = Duration::from_secs(45);

/// The file gallery-dl is part-way through, read from the incoming folder:
/// it reports nothing itself until a file is complete. Each `.part` file's
/// size is followed from one look to the next, so only one that is still
/// growing is shown as downloading, the most recently grown first.
#[derive(Default)]
struct Arrivals {
    /// Each part-fetched file: its size, and when that size was first seen.
    seen: HashMap<PathBuf, (u64, Instant)>,
}

impl Arrivals {
    fn look(&mut self, incoming: &Path, now: Instant) -> Option<CurrentFile> {
        let parts: Vec<(PathBuf, u64)> =
            arriving_files(incoming).into_iter().filter(|(path, _)| extension(path) == "part").collect();
        self.seen.retain(|path, _| parts.iter().any(|(p, _)| p == path));
        let mut newest: Option<(&PathBuf, u64, Instant)> = None;
        for (path, size) in &parts {
            let entry = self.seen.entry(path.clone()).or_insert((*size, now));
            if entry.0 != *size {
                *entry = (*size, now);
            }
            let grew = entry.1;
            if now.saturating_duration_since(grew) <= STILL_ARRIVING && newest.is_none_or(|(_, _, at)| grew > at) {
                newest = Some((path, *size, grew));
            }
        }
        let (path, size, _) = newest?;
        Some(CurrentFile {
            id: String::new(),
            name: path.file_stem()?.to_string_lossy().to_string(),
            downloaded: size,
            total: None,
            index: None,
            count: None,
        })
    }
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

/* ---------- a public Instagram profile, with nobody signed in ---------- */

/// Instagram turned the downloader away for want of a sign-in it accepts:
/// the one kept in the app has ended, or was never good. A public profile
/// can still be fetched as any visitor sees it.
fn instagram_wants_sign_in(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    ["redirect to home page", "redirect to login page", "401", "unauthorized", "login required", "login_required"]
        .iter()
        .any(|sign| lower.contains(sign))
}

/// What a public Instagram profile's page lists for a visitor who is not
/// signed in, as the count the Home card shows.
async fn instagram_public_counts(inner: &Arc<Inner>, profile: &ProfileRef) -> Result<PlaylistInfo, String> {
    let client = inner.client()?;
    let page = crate::instagram::public_page(&client, &profile.name).await?;
    let videos = page.posts.iter().filter(|p| p.video).count() as u64;
    let photos: u64 = page.posts.iter().filter(|p| !p.video).map(|p| p.pieces).sum();
    Ok(PlaylistInfo {
        title: profile.name.clone(),
        count: videos,
        photos,
        more: page.more,
        duration: None,
        thumbnail: None,
        channel: None,
        samples: Vec::new(),
    })
}

/// The last thing yt-dlp called an error, in the words it used.
fn last_error_line(stderr: &[u8]) -> Option<String> {
    String::from_utf8_lossy(stderr).lines().rev().find(|l| l.starts_with("ERROR:")).map(|l| l.trim_start_matches("ERROR:").trim().to_string())
}

/// The ending a picture's address has ("jpg" for ".../photo.jpg?x=1").
fn picture_ext(address: &str) -> &'static str {
    let path = address.split(['?', '#']).next().unwrap_or_default().to_ascii_lowercase();
    ["jpg", "jpeg", "png", "webp", "heic"].into_iter().find(|ext| path.ends_with(&format!(".{ext}"))).unwrap_or("jpg")
}

/// One post of a public Instagram profile: its photos into Images, its
/// videos into Videos. yt-dlp reads the post the way it reads any public
/// post (instagram.rs), with no sign-in.
async fn fetch_public_post(
    inner: &Arc<Inner>,
    live: &Arc<Live>,
    rec: &Record,
    ytdlp: &Path,
    folder: &Path,
    username: &str,
    post: &crate::instagram::PagePost,
) -> Result<(), Fail> {
    let incoming = folder.join(INCOMING);
    let link = format!("https://www.instagram.com/{}/{}/", if post.video { "reel" } else { "p" }, post.code);
    // what the post holds
    let mut ask = command(ytdlp);
    ask.args(["--encoding", "utf-8", "--no-colors", "--no-warnings", "--ignore-no-formats-error", "-J"]);
    inner.add_proxy(&mut ask, &rec.proxy);
    ask.arg("--").arg(&link).stdout(Stdio::piped()).stderr(Stdio::piped());
    let child = ask.spawn().map_err(|e| Fail::Fatal(format!("could not start yt-dlp: {e}")))?;
    // Dropping the unfinished call stops yt-dlp (kill_on_drop).
    let out = tokio::select! {
        _ = live.cancel.cancelled() => return Err(Fail::Cancelled),
        o = child.wait_with_output() => o.map_err(|e| Fail::Fatal(format!("yt-dlp failed: {e}")))?,
    };
    let info = serde_json::from_slice::<serde_json::Value>(&out.stdout).ok().filter(|i| i.is_object());
    let Some(info) = info else {
        return Err(Fail::Fatal(last_error_line(&out.stderr).unwrap_or_else(|| "the post could not be read".into())));
    };
    let found = crate::instagram::read_post(&info);
    let audio_only = rec.media_options.audio_only;

    // The photos, as Instagram serves them.
    for (place, address) in found.photos.iter().filter(|_| !audio_only) {
        let name = sanitize(&format!("{username} {} {place}.{}", post.code, picture_ext(address)));
        let dest = folder.join(IMAGES).join(&name);
        if dest.exists() {
            continue;
        }
        let ask = live
            .client()
            .get(address)
            .header(reqwest::header::USER_AGENT, BROWSER_AGENT)
            .header(reqwest::header::REFERER, "https://www.instagram.com/");
        let bytes = tokio::select! {
            _ = live.cancel.cancelled() => return Err(Fail::Cancelled),
            b = async { ask.send().await?.error_for_status()?.bytes().await } => b,
        };
        let bytes = bytes.map_err(|e| Fail::Retry(format!("a photo could not be fetched: {e}")))?;
        let part = incoming.join(format!("{name}.part"));
        std::fs::write(&part, &bytes)
            .and_then(|_| std::fs::rename(&part, &dest))
            .map_err(|e| Fail::Fatal(format!("could not save a photo: {e}")))?;
        *live.current.lock().unwrap() =
            Some(CurrentFile { id: String::new(), name, downloaded: bytes.len() as u64, total: Some(bytes.len() as u64), index: None, count: None });
    }
    if found.videos.is_empty() {
        return Ok(());
    }

    // The videos: yt-dlp fetches them from what it has just read, so the
    // post is not asked for a second time.
    let read = incoming.join(".rush-post.json");
    std::fs::write(&read, &out.stdout).map_err(|e| Fail::Fatal(format!("could not save the post's details: {e}")))?;
    let mut temp = std::ffi::OsString::from("temp:");
    temp.push(&incoming);
    let mut cmd = command(ytdlp);
    cmd.args([
        "--encoding",
        "utf-8",
        "--no-colors",
        "--windows-filenames",
        "--ignore-errors",
        "--ignore-no-formats-error",
        "--quiet",
        "--no-warnings",
        "--continue",
        "--retries",
        "10",
        "--fragment-retries",
        "10",
        "-o",
        "%(title).120B [%(id)s].%(ext)s",
        "-N",
    ])
    .arg(rec.connections.clamp(1, 16).to_string())
    .arg("-P")
    .arg(folder.join(VIDEOS))
    .arg("-P")
    .arg(temp);
    if audio_only {
        cmd.args(["-f", "ba/b", "--extract-audio", "--audio-format", "mp3", "--audio-quality", "0"]);
    } else {
        let format = match rec.media_options.max_height {
            Some(h) => format!("bv*[height<={h}]+ba/b[height<={h}]/wv*+ba/w"),
            None => "bv*+ba/b".to_string(),
        };
        // MP4, as a single reel is saved (media.rs): Instagram's picture
        // and sound both sit in one as they are.
        cmd.args(["-f", &format, "--merge-output-format", "mp4"]);
    }
    if let Some(ffmpeg) = find_tool("ffmpeg") {
        cmd.arg("--ffmpeg-location").arg(ffmpeg);
    }
    let rate = live.limiter.effective_rate();
    if rate > 0 {
        cmd.arg("--limit-rate").arg(rate.to_string());
    }
    if found.several {
        cmd.arg("--playlist-items").arg(found.videos.iter().map(|n| n.to_string()).collect::<Vec<_>>().join(","));
    }
    inner.add_proxy(&mut cmd, &rec.proxy);
    cmd.arg("--load-info-json").arg(&read).stdout(Stdio::null()).stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| Fail::Fatal(format!("could not start yt-dlp: {e}")))?;
    let said = child.stderr.take().map(|stderr| {
        tokio::spawn(async move {
            let (mut lines, mut last) = (OutputLines::new(stderr), None);
            while let Some(line) = lines.next_line().await {
                if line.starts_with("ERROR:") {
                    last = Some(line.trim_start_matches("ERROR:").trim().to_string());
                }
            }
            last
        })
    });
    let mut beat = tokio::time::interval(Duration::from_millis(500));
    beat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let status = loop {
        tokio::select! {
            _ = live.cancel.cancelled() => {
                kill_tree(&mut child).await;
                return Err(Fail::Cancelled);
            }
            _ = beat.tick() => {
                let (bytes, count) = tally(folder);
                live.downloaded.store(bytes, Relaxed);
                live.items.store(count, Relaxed);
            }
            s = child.wait() => break s.map_err(|e| Fail::Fatal(format!("yt-dlp failed: {e}")))?,
        }
    };
    let _ = std::fs::remove_file(&read);
    let error = match said {
        Some(said) => said.await.ok().flatten(),
        None => None,
    };
    match (status.success(), error) {
        (true, _) => Ok(()),
        (false, Some(error)) => Err(Fail::Retry(error)),
        (false, None) => Err(Fail::Retry(format!("yt-dlp stopped unexpectedly ({status})"))),
    }
}

/// A public Instagram profile with nobody signed in: the posts its own page
/// shows any visitor (its newest twelve), each fetched the way a single
/// public post is. Everything the profile ever posted needs a sign-in,
/// because Instagram shows the rest to signed-in visitors only.
async fn run_instagram_public(
    inner: &Arc<Inner>,
    id: u64,
    live: &Arc<Live>,
    rec: &Record,
    profile: &ProfileRef,
    name: &str,
) -> Result<(), Fail> {
    let ytdlp = find_tool("yt-dlp").ok_or_else(|| {
        Fail::Fatal("Downloading a public Instagram profile needs yt-dlp, which is not installed; install it under Settings, Advanced, Engines".into())
    })?;
    let folder = rec.dir.join(name);
    for dir in [folder.join(IMAGES), folder.join(VIDEOS), folder.join(INCOMING)] {
        std::fs::create_dir_all(&dir).map_err(|e| Fail::Fatal(format!("could not create folder: {e}")))?;
    }
    let give_up = |why: String| {
        discard_unfinished(&rec.dir, name);
        inner.update(id, |r| r.part_started = false);
        Fail::Fatal(why)
    };
    let _conn = ConnGuard::new(&live.conns);
    let client = live.client();
    let page = tokio::select! {
        _ = live.cancel.cancelled() => return Err(Fail::Cancelled),
        p = crate::instagram::public_page(&client, &profile.name) => p,
    };
    let page = page.map_err(&give_up)?;
    // RUSHDOWNLOAD_PROFILE_LIMIT stops after that many posts, for tests.
    let limit = std::env::var("RUSHDOWNLOAD_PROFILE_LIMIT").ok().and_then(|v| v.parse::<usize>().ok()).unwrap_or(usize::MAX);
    debug(|| format!("Instagram shows a visitor {} posts of {}; more behind a sign-in: {}", page.posts.len(), profile.name, page.more));

    let mut last_error = None;
    let mut started = false;
    for (n, post) in page.posts.iter().take(limit).enumerate() {
        // One post at a time, with a pause between, as a visitor would open them.
        if n > 0 {
            tokio::select! {
                _ = live.cancel.cancelled() => return Err(Fail::Cancelled),
                _ = tokio::time::sleep(Duration::from_millis(800)) => {}
            }
        }
        match fetch_public_post(inner, live, rec, &ytdlp, &folder, &profile.name, post).await {
            Ok(()) => {}
            Err(Fail::Cancelled) => return Err(Fail::Cancelled),
            Err(Fail::Fatal(why)) | Err(Fail::Retry(why)) => {
                debug(|| format!("post {} was not fetched: {why}", post.code));
                last_error = Some(why);
            }
        }
        let (bytes, count) = tally(&folder);
        live.downloaded.store(bytes, Relaxed);
        live.items.store(count, Relaxed);
        if !started && bytes > 0 {
            started = true;
            inner.update(id, |r| r.status = Status::Downloading);
        }
    }
    *live.current.lock().unwrap() = None;
    remove_working_files(&folder);
    let (bytes, count) = tally(&folder);
    live.downloaded.store(bytes, Relaxed);
    live.items.store(count, Relaxed);
    if count == 0 {
        let why = last_error.map(|e| crate::instagram::classify(&e).to_string()).unwrap_or_else(|| crate::instagram::NOTHING_SHOWN.into());
        return Err(give_up(why));
    }
    // Whatever was fetched is kept, even if a post or two could not be.
    live.total_hint.store(bytes, Relaxed);
    inner.update(id, |r| r.total = Some(bytes));
    Ok(())
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
    // gallery-dl asked for the profile and the site sent it somewhere else
    // instead. To its front page: the sign-in kept in the app is there, but
    // the site has stopped accepting it (Instagram does this to a sign-in it
    // has seen too many requests from, and says "wait a few minutes").
    if lower.contains("redirect to home page") || lower.contains("please wait a few minutes") {
        return "The site sent the downloader back to its front page instead of showing this profile. \
                It does that when it has stopped accepting the sign-in kept in the app, often after many \
                requests in a short time. Wait a few minutes and retry; if it keeps happening, sign in \
                to the site again on the Sign in page"
            .into();
    }
    // To a check that the visitor is a person, which only they can finish.
    if lower.contains("redirect to challenge page") || lower.contains("challenge_required") || lower.contains("checkpoint_required") {
        return "The site wants to check that it is really you before it shows this profile. Open the \
                site on the Sign in page, finish the check it shows there, then retry"
            .into();
    }
    if ["login", "log in", "sign in", "unauthorized", "401", "authrequired", "cookies", "private", "403"]
        .iter()
        .any(|w| lower.contains(w))
    {
        "This site only shows this profile to signed-in visitors. Sign in to the site on \
         the Sign in page, then retry"
            .into()
    } else if lower.contains("requested user could not be found") {
        // Instagram and X answer a visitor who is not signed in this way,
        // even for a profile that is there for everyone to see.
        "The site said it could not find this profile. Instagram and X say that to visitors who \
         are not signed in: sign in to the site on the Sign in page, then retry. If you are \
         signed in, check the link"
            .into()
    } else if lower.contains("notfound") || lower.contains("404") || lower.contains("does not exist") {
        "This profile was not found".into()
    } else if lower.contains("unable to extract") || lower.contains("unable to download webpage") {
        // The reader found the page but not the list on it: the site showed
        // something else in its place, which is what a sign-in wall or a
        // check for automated visitors looks like.
        "The site did not show this profile's list of videos to the downloader. Sites often keep it for signed-in visitors: sign in to the site on the Sign in page, then retry"
            .into()
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
    // A profile the app reads from its link, or one the programs said is a
    // collection when the link was added.
    let profile = detect(&rec.url)
        .or_else(|| rec.collection.clone())
        .ok_or_else(|| Fail::Fatal("this link is not a profile".into()))?;
    let by_ytdlp = uses_ytdlp(&profile);
    let tool = if by_ytdlp {
        find_tool("yt-dlp").ok_or_else(|| {
            Fail::Fatal("Profile downloads from this site need yt-dlp, which is not installed; install it under Settings, Advanced, Engines".into())
        })?
    } else {
        find_tool("gallery-dl").ok_or_else(|| {
            Fail::Fatal("Profile downloads need gallery-dl, which is not installed; install it under Settings, Advanced, Engines".into())
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

    // Instagram lists a profile's every post to signed-in visitors only.
    // With nobody signed in, a public profile is fetched as any visitor
    // sees it: the posts its own page shows.
    let instagram = !by_ytdlp && profile.site == "Instagram";
    if instagram && !crate::instagram::has_session(inner) {
        return run_instagram_public(inner, id, live, rec, &profile, &name).await;
    }

    // RUSHDOWNLOAD_PROFILE_LIMIT stops after that many files, for tests.
    let limit = std::env::var("RUSHDOWNLOAD_PROFILE_LIMIT").ok().filter(|v| v.parse::<u32>().is_ok());
    // the lower of its own limit and the global one, as it starts
    let rate = live.limiter.effective_rate();
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
    inner.add_sign_in(&mut cmd, Some(&folder), &rec.proxy);
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
    let mut arrivals = Arrivals::default();
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
                            // The first video's picture stands for the whole
                            // download where nothing better is known.
                            if let Some(picture) = following.thumbnail.take() {
                                if inner.record(id).is_some_and(|r| r.thumbnail.is_none()) {
                                    inner.update(id, |r| r.thumbnail = Some(picture));
                                }
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
                    let now = arrivals.look(&incoming, Instant::now());
                    let mut current = live.current.lock().unwrap();
                    match now {
                        Some(file) => *current = Some(file),
                        // Nothing is growing: gallery-dl is between files
                        // (it waits between requests to the site), and a
                        // piece left from a failed try is not "downloading".
                        None if current.as_ref().is_some_and(|c| c.total.is_none()) => *current = None,
                        None => {}
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
        let said = last_error.lock().unwrap().take();
        // A sign-in Instagram has ended is let go of (instagram.rs), and the
        // profile is fetched as any visitor sees it instead.
        if let Some(error) = said.as_deref().filter(|e| instagram && instagram_wants_sign_in(e)) {
            crate::instagram::sign_in_ended(inner, error).await;
            inner.update(id, |r| r.part_started = true);
            return run_instagram_public(inner, id, live, rec, &profile, &name).await;
        }
        let msg = match said {
            Some(error) => friendly(&error),
            None if status.success() => "Nothing to download was found on this profile".into(),
            None => format!("the profile downloader stopped unexpectedly ({status})"),
        };
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
        let on = |site: &str| ProfileRef { site: site.into(), name: "someone".into(), by: None };
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
                // the profile's own line breaks are kept
                about: Some("Makes things.
Posts them here.".into()),
                username: Some("someone".into()),
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
        assert_eq!(titled("Videos of Someone - Dailymotion", "Dailymotion").as_deref(), Some("Videos of Someone"));
        // "Name \u{2014} watch all 375 videos | VK Video": the name is before the long dash
        assert_eq!(
            titled("\u{411}\u{43b}\u{43e}\u{433} \u{2014} 375 \u{432}\u{438}\u{434}\u{435}\u{43e} | VK", "Vkvideo").as_deref(),
            Some("\u{411}\u{43b}\u{43e}\u{433}")
        );
        // a page written in Windows Cyrillic is read as such, named by its server or by itself
        let cyrillic: &[u8] = b"<meta charset=\"windows-1251\"><title>\xc1\xeb\xee\xe3</title>";
        assert!(page_text(cyrillic, None).contains("\u{411}\u{43b}\u{43e}\u{433}"));
        assert!(page_text(b"\xc1\xeb\xee\xe3", Some("text/html; charset=windows-1251")).contains("\u{411}\u{43b}\u{43e}\u{433}"));
        assert_eq!(page_text("caf\u{e9}".as_bytes(), Some("text/html")), "caf\u{e9}", "UTF-8 when nothing is named");
        assert_eq!(titled("Mary-Jane - Smith", "X").as_deref(), Some("Mary-Jane - Smith"), "a dash in a name stays");
        // a sign-in wall, or a title that names nobody
        assert_eq!(titled("Login \u{2022} Instagram", "Instagram"), None);
        assert_eq!(titled("Instagram", "Instagram"), None);
        assert_eq!(titled("@staff", "Tumblr"), None);
        assert_eq!(titled("Check out someone&#39;s Reddit profile", "Reddit"), None);
        // a page that says nothing: only the name in the link is known
        assert_eq!(read_face("<html></html>", &on("X")), Face { username: Some("someone".into()), ..Face::default() });
    }

    #[test]
    fn a_collection_on_any_site_is_recognised_from_what_the_programs_say() {
        let json = |text: &str| serde_json::from_str::<serde_json::Value>(text).unwrap();
        // yt-dlp: someone's videos or tracks, as it answered for Dailymotion and SoundCloud
        let user = json(r#"{"_type":"playlist","extractor_key":"DailymotionUser","entries":[{},{},{}]}"#);
        assert_eq!(read_ytdlp_answer(&user), Some(None));
        let tracks = json(r#"{"_type":"playlist","extractor_key":"SoundcloudUser","title":"Forss (All)","entries":[{},{},{}]}"#);
        assert_eq!(read_ytdlp_answer(&tracks), Some(Some("Forss (All)".into())));
        // a single video, also one opened from inside a playlist, stays one video
        assert_eq!(read_ytdlp_answer(&json(r#"{"_type":"video","extractor_key":"Youtube","title":"A video"}"#)), None);
        // one item is not a collection; an article with two clips in it is not someone's page
        assert_eq!(read_ytdlp_answer(&json(r#"{"_type":"playlist","extractor_key":"X","entries":[{}]}"#)), None);
        assert_eq!(read_ytdlp_answer(&json(r#"{"_type":"playlist","extractor_key":"Generic","entries":[{},{}]}"#)), None);
        // no answer, but a channel reader's error: a collection the site did not list (VK's bot check)
        assert!(collection_reader("[vk:uservideos] Extracting URL: x
ERROR: [vk:uservideos] @someone: Unable to extract cursor data"));
        assert!(collection_reader("ERROR: [youtube:tab] UC123: This channel does not have a videos tab"));
        assert!(!collection_reader("ERROR: [vk] 123_456: This video is unavailable"), "a single video's reader");
        assert!(!collection_reader("ERROR: [generic] Unsupported URL: https://site.test/models/someone/"));
        assert!(!collection_reader("ERROR: Unsupported URL: https://site.test/x"));
        // gallery-dl: someone's pictures, as it answered for ArtStation
        assert_eq!(read_gallerydl_answer("artstation\tuser\twebp\nartstation\tuser\twebp\nartstation\tuser\twebp\n"), Some(true));
        assert_eq!(read_gallerydl_answer("site\tuser\tmp4\nsite\tuser\tmp4\n"), Some(false), "videos only");
        assert_eq!(read_gallerydl_answer("site\tpost\tjpg\n"), None, "one file");
        assert_eq!(read_gallerydl_answer("directlink\t\tjpg\ndirectlink\t\tjpg\n"), None, "a plain file link");
        assert_eq!(read_gallerydl_answer("[gallery-dl][error] Unsupported URL 'x'\n"), None);
        // the site's name and whose collection it is, from the link
        let at = |link: &str| reqwest::Url::parse(link).unwrap();
        assert_eq!(site_name(&at("https://www.dailymotion.com/france24")), "Dailymotion");
        assert_eq!(site_name(&at("https://m.example.co.uk/a")), "Example");
        assert_eq!(name_in_link(&at("https://www.dailymotion.com/france24")).as_deref(), Some("france24"));
        assert_eq!(name_in_link(&at("https://site.test/user/Some%20One/videos")).as_deref(), Some("Some One"));
        assert_eq!(name_in_link(&at("https://site.test/@someone")).as_deref(), Some("someone"));
        assert_eq!(name_in_link(&at("https://site.test/videos")), None);
        // a found collection says which program fetches it; the known sites keep their own way
        let found = ProfileRef { site: "Artstation".into(), name: "a".into(), by: Some(Fetcher::Gallerydl) };
        assert!(!uses_ytdlp(&found));
        assert!(uses_ytdlp(&ProfileRef { by: Some(Fetcher::Ytdlp), ..found.clone() }));
        assert!(uses_ytdlp(&detect("https://www.youtube.com/@someone").unwrap()));
        assert!(!uses_ytdlp(&detect("https://www.instagram.com/someone/").unwrap()));
        // kept with a download and read back, it is the same
        let kept: ProfileRef = serde_json::from_str(&serde_json::to_string(&found).unwrap()).unwrap();
        assert_eq!(kept, found);
        // the known sites are sent to the window without the field at all
        assert!(!serde_json::to_string(&detect("https://x.com/someone").unwrap()).unwrap().contains("\"by\""));
    }

    #[test]
    fn a_pornhub_profile_picture_is_read_from_the_page_itself() {
        let on = ProfileRef { site: "Pornhub".into(), name: "someone".into(), by: None };
        // The preview's picture is one of the videos; the avatar is in the body.
        let page = r#"<html><head>
            <meta property="og:title" content="Some One's Porn Videos | Pornhub" />
            <meta property="og:image" content="https://ei.cdn.test/videos/thumb/1.jpg" />
            <meta property="og:description" content="Watch Some One's videos." />
            </head><body>
            <img src="https://ei.cdn.test/www-static/images/logo.png" alt="Pornhub" />
            <div class="topProfileHeader"><img id="getAvatar" class="avatar" src="https://ei.cdn.test/pics/users/1/200x200/someone.jpg" alt="Some One"></div>
            </body></html>"#;
        let seen = read_face(page, &on);
        assert_eq!(seen.name.as_deref(), Some("Some One"));
        assert_eq!(seen.picture.as_deref(), Some("https://ei.cdn.test/pics/users/1/200x200/someone.jpg"));
        // a picture loaded later: its real address is in data-src
        let lazy = r#"<img class="userAvatar lazy" src="data:image/gif;base64,R0lGOD" data-src="https://ei.cdn.test/pics/users/2/someone.jpg">"#;
        assert_eq!(avatar_img(lazy).as_deref(), Some("https://ei.cdn.test/pics/users/2/someone.jpg"));
        // no avatar in the page: the preview's picture, unless it is the site's logo
        let bare = r#"<meta property="og:title" content="Some One - Pornhub.com"><meta property="og:image" content="https://cdn.test/pics/users/3/a.jpg">"#;
        let seen = read_face(bare, &on);
        assert_eq!((seen.name.as_deref(), seen.picture.as_deref()), (Some("Some One"), Some("https://cdn.test/pics/users/3/a.jpg")));
        let logo = r#"<meta property="og:image" content="https://cdn.test/www-static/images/pornhub_logo.png">"#;
        assert_eq!(read_face(logo, &on).picture, None);
        // other sites are not read this way
        assert!(!avatar_in_body("X") && page_cookie("X").is_none());
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
        // what Instagram and X say to a visitor who is not signed in
        let unsigned = friendly("[instagram][error] NotFoundError: Requested user could not be found");
        assert!(unsigned.contains("sign in to the site on the Sign in page"), "{unsigned}");
        assert_eq!(friendly("ERROR: [site] NotFoundError: profile does not exist"), "This profile was not found");
        // Instagram wanting a sign-in it accepts: a public profile is then read as a visitor
        assert!(instagram_wants_sign_in("[instagram][error] HTTP redirect to home page (https://www.instagram.com/)"));
        assert!(instagram_wants_sign_in("[instagram][error] HttpError: '401 Unauthorized' for 'https://www.instagram.com/graphql/query'"));
        assert!(!instagram_wants_sign_in("[instagram][error] HttpError: '429 Too Many Requests' for 'https://www.instagram.com/x'"));
        assert_eq!(picture_ext("https://cdn.test/v/t51/photo_n.webp?stp=dst-jpg&x=1"), "webp");
        assert_eq!(picture_ext("https://cdn.test/v/t51/photo"), "jpg");
        // a sign-in the site has stopped accepting: gallery-dl is sent to the front page
        let refused = friendly("[instagram][error] HTTP redirect to home page (https://www.instagram.com/)");
        assert!(refused.contains("stopped accepting the sign-in kept in the app"), "{refused}");
        assert!(refused.contains("Wait a few minutes") && refused.contains("Sign in page"), "{refused}");
        // a check only the person can finish
        let check = friendly("[instagram][error] HTTP redirect to challenge page (https://www.instagram.com/challenge/)");
        assert!(check.contains("really you") && check.contains("Sign in page"), "{check}");
        // not signed in at all: still the plain request to sign in
        let wall = friendly("[instagram][error] HTTP redirect to login page (https://www.instagram.com/accounts/login/)");
        assert!(wall.contains("signed-in visitors"), "{wall}");
        // a list the site showed something else in place of
        let withheld =friendly("ERROR: [vk:uservideos] @someone: Unable to extract cursor data; please report this issue");
        assert!(withheld.contains("sign in to the site on the Sign in page"), "{withheld}");
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
                username: Some("someone".into()),
                links: Vec::new(),
                likes: None,
                rounded: false,
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

    #[test]
    fn a_profiles_numbers_bio_and_links_are_read_from_its_page() {
        let on = |site: &str| ProfileRef { site: site.into(), name: "natgeo".into(), by: None };
        // Instagram's page, as any visitor gets it
        let page = r#"<head><meta property="og:title" content="National Geographic (&#064;natgeo) &#x2022; Instagram photos and videos" />
            <meta property="og:image" content="https://cdn.site.test/natgeo.jpg" />
            <meta property="og:description" content="267M Followers, 196 Following, 32K Posts - See Instagram photos and videos from National Geographic (&#064;natgeo)" />
            <meta content="267M Followers, 196 Following, 32K Posts - National Geographic (&#064;natgeo) on Instagram: &quot;Step into wonder &#x1f30e; natgeo.com/explore and https://link.site.test/x.&quot;" name="description" /></head>"#;
        let face = read_face(page, &on("Instagram"));
        assert_eq!(face.name.as_deref(), Some("National Geographic"));
        assert_eq!(face.username.as_deref(), Some("natgeo"));
        assert_eq!((face.followers, face.following, face.posts, face.rounded), (Some(267_000_000), Some(196), Some(32_000), true));
        assert_eq!(face.about.as_deref(), Some("Step into wonder \u{1f30e} natgeo.com/explore and https://link.site.test/x."));
        assert_eq!(face.links, ["https://natgeo.com/explore", "https://link.site.test/x"]);
        // where the preview line is only the site's own words, there is no bio
        let plain = r#"<head><meta property="og:title" content="Some One (@natgeo)" />
            <meta property="og:description" content="1,234 Followers, 5 Following, 10 Posts - See Instagram photos and videos from Some One" /></head>"#;
        let face = read_face(plain, &on("Instagram"));
        assert_eq!((face.followers, face.about), (Some(1234), None));

        // numbers as pages write them
        assert_eq!(count_text("15.1", Some("M")), Some(15_100_000));
        assert_eq!(count_text("6.2", Some("k")), Some(6_200));
        assert_eq!(count_text("1,234", None), Some(1234));
        assert_eq!(count_text("1.234", None), Some(1234));
        // e-mail addresses are not links
        assert_eq!(links_in("write to me@mail.com or see www.site.com/a, thanks"), ["https://www.site.com/a"]);

        // YouTube's channel header
        let yt = r#"<head><meta property="og:title" content="NASA"></head><body><script>var ytInitialData = {
            "channelMetadataRenderer":{"title":"NASA","description":"NASA's mission.\r\n\r\n\r\nMore here.","rssUrl":"x"},
            "metadataParts":[{"text":{"content":"15.1M subscribers"}},{"text":{"content":"6.2K videos"}}],
            "attribution":{"attributionViewModel":{"text":{"content":" nasa.gov","styleRuns":[]},"suffix":{"content":"and 5 more links"}}},
            "vanityChannelUrl":"http://www.youtube.com/@NASA"};</script></body>"#;
        let face = read_face(yt, &ProfileRef { site: "YouTube".into(), name: "UCabc".into(), by: None });
        assert_eq!((face.followers, face.posts), (Some(15_100_000), Some(6_200)));
        assert_eq!(face.about.as_deref(), Some("NASA's mission.\n\nMore here."));
        assert_eq!(face.username.as_deref(), Some("NASA"));
        assert_eq!(face.links, ["https://nasa.gov"]);
        assert!(reads_body("YouTube") && page_cookie("YouTube").is_some());
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
        assert_eq!(at(&mut f, "RUSH|100|1000|NA|2|27|NA|abc|First | video"), Some(("First | video".into(), 100, Some(1000), Some(2), Some(27))));
        assert_eq!(at(&mut f, "RUSH|1000|1000|NA|2|27|NA|abc|First | video").unwrap().1, 1000);
        // the sound follows the picture: its bytes are added on
        assert_eq!(at(&mut f, "RUSH|50|NA|200.0|2|27|NA|abc|First | video"), Some(("First | video".into(), 1050, Some(1200), Some(2), Some(27))));
        // the next video starts from nothing; without a title its id stands in
        assert_eq!(at(&mut f, "RUSH|10|NA|NA|3|27|NA|def|NA"), Some(("def".into(), 10, None, Some(3), Some(27))));
        assert_eq!(f.read("RUSH|10|NA|NA|3|27|NA|def|NA", false).map(|c| (c.index, c.count)), Some((None, None)));
        // a title arrives as JSON, with letters outside ASCII written as escapes
        assert_eq!(
            at(&mut f, r#"RUSH|5|NA|NA|4|27|NA|ghi|"🎮 Part 3 – \"Pagoda\" | live""#).unwrap().0,
            "\u{1f3ae} Part 3 \u{2013} \"Pagoda\" | live"
        );
        // the picture of the file arriving is read once, when that file begins
        let mut g = Following::default();
        g.read("RUSH|1|NA|NA|1|3|https://img.test/v1.jpg|v1|\"One\"", true).unwrap();
        assert_eq!(g.thumbnail.take().as_deref(), Some("https://img.test/v1.jpg"));
        g.read("RUSH|9|NA|NA|1|3|https://img.test/v1.jpg|v1|\"One\"", true).unwrap();
        assert_eq!(g.thumbnail, None, "not again for the same file");
        g.read("RUSH|1|NA|NA|2|3|NA|v2|\"Two\"", true).unwrap();
        assert_eq!(g.thumbnail, None, "none given, none kept");
        assert_eq!(f.read("[download] something else", true), None);
        assert_eq!(f.read("RUSH|NA|NA|NA|NA|NA|NA|x|y", true), None);
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
        let now = f.read("RUSH|40|100|NA|2|4|NA|b|NA", true).unwrap();
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

    #[test]
    fn only_a_growing_piece_counts_as_arriving() {
        let dir = std::env::temp_dir().join(format!("rush-arrivals-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let left = dir.join("123.fdash-1v.mp4.part");
        std::fs::write(&left, vec![0u8; 300]).unwrap();
        std::fs::write(dir.join("done.jpg"), b"x").unwrap();
        let mut arrivals = Arrivals::default();
        let t0 = Instant::now();
        assert_eq!(arrivals.look(&dir, t0).map(|f| f.name), Some("123.fdash-1v.mp4".into()), "new pieces count at first");
        let later = t0 + STILL_ARRIVING + Duration::from_secs(1);
        assert!(arrivals.look(&dir, later).is_none(), "a piece that stopped growing is not downloading");
        // the next file starts arriving beside the leftover piece
        let next = dir.join("456.mp4.part");
        std::fs::write(&next, vec![0u8; 10]).unwrap();
        let file = arrivals.look(&dir, later).unwrap();
        assert_eq!((file.name.as_str(), file.downloaded), ("456.mp4", 10));
        std::fs::write(&next, vec![0u8; 50]).unwrap();
        let file = arrivals.look(&dir, later + Duration::from_secs(40)).unwrap();
        assert_eq!((file.name.as_str(), file.downloaded), ("456.mp4", 50), "still growing");
        std::fs::remove_file(&next).unwrap();
        assert!(arrivals.look(&dir, later + Duration::from_secs(41)).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
