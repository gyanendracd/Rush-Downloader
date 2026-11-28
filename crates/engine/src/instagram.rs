//! Instagram posts, reels and videos, from their link alone.
//!
//! Public content is fetched the way any visitor sees it: through yt-dlp,
//! with no sign-in, credential or cookie of any kind (with one, Instagram
//! sends yt-dlp a different way and turns it away). yt-dlp, kept up to date
//! by the app (deps.rs), finds the post's own video files; the engine's
//! usual site download then fetches the best picture and sound streams with
//! several connections, straight to disk, and joins them with FFmpeg, so
//! the original resolution and shape are kept.
//!
//! Nothing here gets around Instagram's access controls. A private account,
//! or a post Instagram shows only to signed-in viewers, is reported as
//! such, and the user is never asked to sign in or sent to Instagram. The
//! one exception is a user who has already signed in to Instagram in the
//! app for their own reasons: then, and only for content Instagram asks a
//! sign-in for, that sign-in is used, through gallery-dl.
//!
//! Instagram changes often, so this is kept in one place: how a link is
//! told apart (`is_post`), what Instagram's refusals mean (`classify`), and
//! which way a post is fetched (`info`, `run`).

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, BufReader};

use crate::media::{command, find_tool, kill_tree, MediaInfo};
use crate::task::{Fail, Live};
use crate::{Inner, ProxyChoice, Record, Status};

/// What the user is told, in each case.
pub(crate) const INVALID: &str = "Please enter a valid Instagram URL.";
pub(crate) const PRIVATE: &str = "This account is private. Only publicly accessible content can be downloaded.";
pub(crate) const UNAVAILABLE: &str = "This Instagram video is currently unavailable.";
pub(crate) const NEEDS_SIGN_IN: &str =
    "This Instagram content is only shown to signed-in Instagram users, so it cannot be accessed or downloaded without authentication.";
pub(crate) const AGE_RESTRICTED: &str =
    "This post is age-restricted by Instagram, which shows it only to signed-in Instagram users. Sign in to Instagram on the Sign in page, then come back to download it.";
pub(crate) const NO_VIDEO: &str = "This Instagram post has no video to download.";
/// Only ever said to a user who had already signed in to Instagram in the
/// app: Instagram shows this post to signed-in users alone, and has stopped
/// accepting that sign-in.
pub(crate) const SIGN_IN_REFUSED: &str =
    "Instagram shows this post only to signed-in users, and it is not accepting the sign-in kept in the app right now. Wait a few minutes and retry; if it keeps happening, sign in to Instagram again on the Sign in page.";

/// What kind of Instagram link it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// "/p/…": a post, which may be a video, photos, or both.
    Post,
    /// "/reel/…" or "/reels/…".
    Reel,
    /// "/tv/…": an older long video.
    Tv,
}

fn host_is_instagram(url: &reqwest::Url) -> bool {
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    host == "instagram.com" || host.ends_with(".instagram.com") || host == "instagr.am"
}

/// The profile page of an Instagram username, when it is one (letters,
/// digits, "." and "_", 30 at most).
pub(crate) fn profile_link(username: &str) -> Option<String> {
    let name = username.trim().trim_start_matches('@');
    let valid = !name.is_empty() && name.len() <= 30 && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_');
    valid.then(|| format!("https://www.instagram.com/{name}/"))
}

/// Whether a link points at Instagram at all.
pub(crate) fn is_instagram(url: &str) -> bool {
    reqwest::Url::parse(url.trim()).is_ok_and(|u| host_is_instagram(&u))
}

/// The kind of post a link points at and its code, or `None` for any other
/// link (a profile, the home page, another site).
pub(crate) fn post_of(url: &str) -> Option<(Kind, String)> {
    let parsed = reqwest::Url::parse(url.trim()).ok()?;
    if !host_is_instagram(&parsed) || !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    let parts: Vec<&str> = parsed.path().split('/').filter(|p| !p.is_empty()).collect();
    // "/p/CODE" or, with the account in front, "/name/p/CODE"
    let at = parts.iter().position(|p| matches!(*p, "p" | "reel" | "reels" | "tv"))?;
    if at > 1 {
        return None;
    }
    let code = parts.get(at + 1)?;
    let valid = (5..=40).contains(&code.len()) && code.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if !valid {
        return None;
    }
    let kind = match parts[at] {
        "p" => Kind::Post,
        "tv" => Kind::Tv,
        _ => Kind::Reel,
    };
    Some((kind, code.to_string()))
}

/// Whether a link is one Instagram post, reel or video (not a profile).
pub(crate) fn is_post(url: &str) -> bool {
    post_of(url).is_some()
}

/// What Instagram's refusal (yt-dlp's or gallery-dl's words) means, as the
/// message the user sees.
pub(crate) fn classify(error: &str) -> &'static str {
    let lower = error.to_ascii_lowercase();
    let says = |signs: &[&str]| signs.iter().any(|s| lower.contains(s));
    if says(&["redirect to home page", "redirect to login page", "redirect to challenge page"]) {
        // gallery-dl, with the user's own sign-in, was sent to Instagram's
        // front page, its sign-in page or a check instead of the post: the
        // sign-in is there, but Instagram has stopped accepting it.
        SIGN_IN_REFUSED
    } else if says(&["private account", "account is private", "is private", "this account is private"]) {
        PRIVATE
    } else if says(&["no video", "there is no video", "no video formats", "no media found"]) {
        NO_VIDEO
    } else if says(&["empty media response", "404", "not found", "has been removed", "was deleted"]) {
        // Instagram answers a post that is gone with nothing at all (its
        // posts for signed-in viewers only are refused in words of their own)
        UNAVAILABLE
    } else if says(&[
        // Instagram's words for an age-restricted post, which it shows only
        // to a signed-in account: "isn't available to everyone … certain
        // audiences", "age-restricted", "confirm your age".
        "isn't available to everyone",
        "not available to everyone",
        "certain audiences",
        "age-restricted",
        "age restricted",
        "confirm your age",
        "18 years old",
        "must be 18",
    ]) {
        AGE_RESTRICTED
    } else if says(&[
        "login required",
        "log in",
        "login_required",
        "sign in",
        "401",
        "403",
        "cookies",
        "rate-limit reached or login",
        "checkpoint",
    ]) {
        NEEDS_SIGN_IN
    } else {
        // gone, removed, never was, or nothing Instagram would give at all
        UNAVAILABLE
    }
}

/// Whether the user has already signed in to Instagram inside the app (on
/// the Sign in page, for their own use). The app never asks them to.
pub(crate) fn has_session(inner: &Inner) -> bool {
    std::fs::read_to_string(inner.cookies_file())
        .map(|cookies| cookies.lines().any(|l| l.contains("instagram.com") && l.contains("\tsessionid\t")))
        .unwrap_or(false)
}

/// Said once Instagram has ended the sign-in kept in the app and the app has
/// let go of it (see `sign_in_ended`).
pub(crate) const SIGN_IN_ENDED: &str =
    "Instagram has ended the sign-in kept in the app, so the app has let go of it. Sign in to Instagram again on the Sign in page, then retry.";

/// The cookies that are the sign-in itself. The others (which device this
/// is, its language) say nothing about who is signed in and are kept.
const SESSION_COOKIES: [&str; 3] = ["sessionid", "ds_user_id", "rur"];

/// One line of the cookie file, when it is a cookie of Instagram's: its
/// name and value.
fn instagram_cookie(line: &str) -> Option<(&str, &str)> {
    let line = line.strip_prefix("#HttpOnly_").unwrap_or(line);
    if line.starts_with('#') {
        return None;
    }
    let fields: Vec<&str> = line.split('\t').collect();
    let host = fields.first()?.trim_start_matches('.').to_ascii_lowercase();
    let ours = fields.len() == 7 && (host == "instagram.com" || host.ends_with(".instagram.com"));
    ours.then(|| (fields[5], fields[6]))
}

/// A cookie file without Instagram's sign-in; `None` when it holds none.
fn without_session(cookies: &str) -> Option<String> {
    let is_session = |line: &str| instagram_cookie(line).is_some_and(|(name, _)| SESSION_COOKIES.contains(&name));
    if !cookies.lines().any(is_session) {
        return None;
    }
    let mut kept: String = cookies.lines().filter(|line| !is_session(line)).collect::<Vec<_>>().join("\n");
    kept.push('\n');
    Some(kept)
}

/// Where Instagram ends up taking a visitor who asks for their own settings
/// page: there, for one it knows; away from it (its front page, its sign-in
/// page) for one whose sign-in it no longer accepts.
fn shows_settings(landed_on: &reqwest::Url) -> bool {
    landed_on.path().starts_with("/accounts/edit")
}

/// Whether Instagram still accepts the sign-in kept in the app. `None`:
/// Instagram could not be asked, so nothing is known.
async fn sign_in_accepted(inner: &Arc<Inner>) -> Option<bool> {
    let cookies = std::fs::read_to_string(inner.cookies_file()).ok()?;
    let sent: Vec<String> = cookies.lines().filter_map(instagram_cookie).map(|(name, value)| format!("{name}={value}")).collect();
    let answer = inner
        .client()
        .ok()?
        .get("https://www.instagram.com/accounts/edit/")
        .header(reqwest::header::USER_AGENT, crate::profile::BROWSER_AGENT)
        .header(reqwest::header::ACCEPT, "text/html")
        .header(reqwest::header::COOKIE, sent.join("; "))
        .send()
        .await;
    match answer {
        Ok(answer) => Some(shows_settings(answer.url())),
        // With a sign-in it has ended, Instagram sends the visitor to its
        // front page, and from there to its front page again, without end.
        Err(e) if e.is_redirect() => Some(false),
        Err(_) => None,
    }
}

/// Instagram ends sign-ins on its own (it does to one it has seen many
/// requests from), and the app would go on believing in the cookies it
/// kept: every download would send them and be turned away the same way,
/// with the Sign in page still showing Instagram as signed in. So when
/// gallery-dl, given that sign-in, is sent to Instagram's front or sign-in
/// page instead of what it asked for, Instagram is asked once whether the
/// sign-in still stands; if it does not, the app lets go of it. Returns
/// whether it did.
pub(crate) async fn sign_in_ended(inner: &Arc<Inner>, said: &str) -> bool {
    let lower = said.to_ascii_lowercase();
    let turned_away = lower.contains("redirect to home page") || lower.contains("redirect to login page");
    if !turned_away || !has_session(inner) || sign_in_accepted(inner).await != Some(false) {
        return false;
    }
    let file = inner.cookies_file();
    let Some(kept) = std::fs::read_to_string(&file).ok().as_deref().and_then(without_session) else {
        return false;
    };
    match std::fs::write(&file, kept) {
        Ok(()) => {
            tracing::info!("Instagram has ended the sign-in kept in the app; it was let go of");
            true
        }
        Err(e) => {
            tracing::warn!("could not let go of the ended Instagram sign-in: {e}");
            false
        }
    }
}

/* ---------- a public profile, as any visitor sees it ---------- */

/// What Instagram tells a visitor who is not signed in about a profile that
/// lists nothing for them.
pub(crate) const NOTHING_SHOWN: &str =
    "Instagram shows none of this profile's posts to a visitor who is not signed in. That is how a private account looks, and an account with no posts. For a private account you follow, sign in to Instagram on the Sign in page, then retry";

/// One post in the list a profile's own page carries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PagePost {
    pub code: String,
    /// A reel or video; otherwise a photo, or several things in one post.
    pub video: bool,
    /// How many photos or videos the post holds.
    pub pieces: u64,
}

/// The posts a public profile's page shows a visitor who is not signed in:
/// its newest twelve. Past them Instagram asks for a sign-in, and so does
/// the app.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct PublicPage {
    pub posts: Vec<PagePost>,
    /// The profile has more posts than the page shows.
    pub more: bool,
}

/// Where Instagram's page keeps the profile's list of posts.
const POSTS_ON_PAGE: &str = "polaris_ordered_timeline_connection";

fn find_key<'a>(value: &'a serde_json::Value, key: &str) -> Option<&'a serde_json::Value> {
    match value {
        serde_json::Value::Object(map) => map.get(key).or_else(|| map.values().find_map(|v| find_key(v, key))),
        serde_json::Value::Array(list) => list.iter().find_map(|v| find_key(v, key)),
        _ => None,
    }
}

/// Reads the list of posts out of a profile's page: Instagram writes it
/// into the page itself, as data for the grid a visitor sees.
pub(crate) fn read_public_page(html: &str) -> PublicPage {
    for (at, _) in html.match_indices(POSTS_ON_PAGE) {
        // the block of data the words are in: `<script ...>{...}</script>`
        let Some(open) = html[..at].rfind("<script") else { continue };
        let Some(start) = html[open..at].find('>').map(|gt| open + gt + 1) else { continue };
        let Some(end) = html[at..].find("</script>").map(|lt| at + lt) else { continue };
        let Ok(data) = serde_json::from_str::<serde_json::Value>(&html[start..end]) else { continue };
        let Some(list) = find_key(&data, POSTS_ON_PAGE) else { continue };
        let mut posts: Vec<PagePost> = Vec::new();
        for node in list.get("edges").and_then(|e| e.as_array()).into_iter().flatten().filter_map(|e| e.get("node")) {
            let Some(code) = node.get("code").and_then(|c| c.as_str()) else { continue };
            let valid = (5..=40).contains(&code.len()) && code.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
            if !valid || posts.iter().any(|p| p.code == code) {
                continue;
            }
            posts.push(PagePost {
                code: code.to_string(),
                // Instagram's own numbers: 1 a photo, 2 a video, 8 several in one post
                video: node.get("media_type").and_then(|m| m.as_u64()) == Some(2),
                pieces: node.get("carousel_media_count").and_then(|c| c.as_u64()).filter(|c| *c > 0).unwrap_or(1),
            });
        }
        if !posts.is_empty() {
            let more = list.get("page_info").and_then(|p| p.get("has_next_page")).and_then(|m| m.as_bool()).unwrap_or(false);
            return PublicPage { posts, more };
        }
    }
    PublicPage::default()
}

/// Asks for a profile's page the way a browser does with nobody signed in,
/// and reads the posts it lists. No cookie is sent.
pub(crate) async fn public_page(client: &reqwest::Client, username: &str) -> Result<PublicPage, String> {
    let link = profile_link(username).ok_or(INVALID)?;
    let answer = client
        .get(&link)
        .header(reqwest::header::USER_AGENT, crate::profile::BROWSER_AGENT)
        .header(reqwest::header::ACCEPT, "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")
        .header(reqwest::header::ACCEPT_LANGUAGE, "en-US,en;q=0.8")
        .header("Sec-Fetch-Dest", "document")
        .header("Sec-Fetch-Mode", "navigate")
        .header("Sec-Fetch-Site", "none")
        .send()
        .await
        .map_err(|e| format!("Instagram could not be reached: {e}"))?;
    match answer.status().as_u16() {
        404 => return Err("This profile was not found".into()),
        429 => return Err("Instagram is limiting requests right now. Wait a while and retry".into()),
        code if !answer.status().is_success() => return Err(format!("Instagram answered {code} for this profile")),
        _ => {}
    }
    let html = answer.text().await.map_err(|e| format!("Instagram's page could not be read: {e}"))?;
    let page = read_public_page(&html);
    if page.posts.is_empty() {
        return Err(NOTHING_SHOWN.into());
    }
    Ok(page)
}

/// What one post holds, as yt-dlp's reader for public posts lists it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct PostFiles {
    /// The videos, by their place in the post (1 for the first).
    pub videos: Vec<usize>,
    /// The photos: their place in the post and where each is kept.
    pub photos: Vec<(usize, String)>,
    /// Several things in one post.
    pub several: bool,
}

pub(crate) fn read_post(info: &serde_json::Value) -> PostFiles {
    let several = info.get("_type").and_then(|t| t.as_str()) == Some("playlist");
    let entries: Vec<&serde_json::Value> = match info.get("entries").and_then(|e| e.as_array()) {
        Some(entries) if several => entries.iter().collect(),
        _ => vec![info],
    };
    let mut found = PostFiles { several, ..Default::default() };
    for (i, entry) in entries.into_iter().enumerate() {
        let plays = entry.get("formats").and_then(|f| f.as_array()).is_some_and(|f| !f.is_empty());
        if plays {
            found.videos.push(i + 1);
        } else if let Some(picture) = entry.get("thumbnail").and_then(|t| t.as_str()).filter(|t| t.starts_with("https://")) {
            found.photos.push((i + 1, picture.to_string()));
        }
    }
    found
}

/// A post's thumbnail, title, length and size, before downloading. Only two
/// choices are offered for Instagram: the original, or its sound alone; so
/// no list of heights, but the original's width and height are told.
pub(crate) async fn info(inner: &Arc<Inner>, url: &str) -> Result<MediaInfo, String> {
    if !is_post(url) {
        return Err(INVALID.into());
    }
    match crate::media::ytdlp_info(inner, url).await {
        Ok(mut found) => {
            found.site = Some("Instagram".into());
            found.heights.clear();
            // Whose post it is: yt-dlp names them in the title ("Video by
            // someone") when it gives no profile link of its own.
            if found.uploader_url.is_none() {
                let named = found.title.strip_prefix("Video by ").map(str::trim).map(str::to_string);
                if let Some(link) = named.as_deref().and_then(profile_link) {
                    found.uploader.get_or_insert_with(|| named.clone().unwrap_or_default());
                    found.uploader_url = Some(link);
                }
            }
            if found.title.trim().is_empty() || found.title.starts_with("Video by ") {
                found.title = found.title.replacen("Video by ", "Video by @", 1);
            }
            Ok(found)
        }
        Err(e) => {
            tracing::info!("Instagram post read without sign-in failed: {e}");
            let meaning = classify(&e);
            if matches!(meaning, NEEDS_SIGN_IN | AGE_RESTRICTED) && has_session(inner) {
                return info_signed_in(inner, url).await;
            }
            Err(meaning.into())
        }
    }
}

/// Downloads a post, reel or video: as any visitor sees it first; with the
/// user's own existing sign-in only where Instagram asks for one.
pub(crate) async fn run(inner: &Arc<Inner>, id: u64, live: &Arc<Live>, rec: &Record) -> Result<(), Fail> {
    if !is_post(&rec.url) {
        return Err(Fail::Fatal(INVALID.into()));
    }
    match crate::media::run_ytdlp(inner, id, live, rec).await {
        Err(Fail::Fatal(e)) | Err(Fail::Retry(e)) => {
            tracing::info!(id, "Instagram download without sign-in failed: {e}");
            let meaning = classify(&e);
            if matches!(meaning, NEEDS_SIGN_IN | AGE_RESTRICTED) && has_session(inner) {
                return run_signed_in(inner, id, live, rec).await;
            }
            // what yt-dlp left behind goes
            crate::media::remove_work_dir(&rec.dir, id);
            Err(Fail::Fatal(meaning.into()))
        }
        other => other,
    }
}

/* ---------- with the user's own existing sign-in, through gallery-dl ---------- */

fn gallery_dl() -> Result<PathBuf, String> {
    find_tool("gallery-dl").ok_or_else(|| UNAVAILABLE.to_string())
}

/// What gallery-dl's `-j` says a post holds: its files, each as
/// (address, details).
fn files_in(listing: &serde_json::Value) -> Vec<(String, serde_json::Value)> {
    listing
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let entry = entry.as_array()?;
            // [3, url, details]: a file
            if entry.first()?.as_u64()? != 3 {
                return None;
            }
            Some((entry.get(1)?.as_str()?.to_string(), entry.get(2)?.clone()))
        })
        .collect()
}

/// The error gallery-dl's `-j` reported, if it did: `[-1, {"message": …}]`.
fn listed_error(listing: &serde_json::Value) -> Option<String> {
    listing.as_array()?.iter().find_map(|entry| {
        let entry = entry.as_array()?;
        (entry.first()?.as_i64()? == -1).then(|| entry.get(1)?.get("message")?.as_str().map(str::to_string))?
    })
}

fn is_video(url: &str, details: &serde_json::Value) -> bool {
    url.starts_with("ytdl:") || details.get("extension").and_then(|e| e.as_str()) == Some("mp4") || details.get("video_url").is_some_and(|v| !v.is_null())
}

async fn info_signed_in(inner: &Arc<Inner>, url: &str) -> Result<MediaInfo, String> {
    let tool = gallery_dl()?;
    let mut cmd = command(&tool);
    cmd.env("PYTHONUTF8", "1").env("PYTHONIOENCODING", "utf-8").arg("-j");
    inner.add_sign_in(&mut cmd, None, &ProxyChoice::Default);
    cmd.arg("--").arg(url).stdout(Stdio::piped()).stderr(Stdio::piped());
    let child = cmd.spawn().map_err(|_| UNAVAILABLE.to_string())?;
    let out = tokio::time::timeout(Duration::from_secs(60), child.wait_with_output())
        .await
        .map_err(|_| UNAVAILABLE.to_string())?
        .map_err(|_| UNAVAILABLE.to_string())?;
    let listing: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap_or_default();
    let files = files_in(&listing);
    let Some((_, details)) = files.iter().find(|(u, d)| is_video(u, d)).or(files.first()).cloned() else {
        let said = listed_error(&listing).unwrap_or_else(|| String::from_utf8_lossy(&out.stderr).to_string());
        if sign_in_ended(inner, &said).await {
            return Err(SIGN_IN_ENDED.into());
        }
        return Err(classify(&said).into());
    };
    let text = |key: &str| details.get(key).and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    let number = |key: &str| details.get(key).and_then(|v| v.as_u64()).map(|n| n as u32);
    let user = text("username").unwrap_or_else(|| "Instagram".into());
    let caption = text("description").and_then(|d| d.lines().map(str::trim).find(|l| !l.is_empty()).map(str::to_string));
    let title = match caption {
        Some(c) if c.chars().count() > 90 => format!("{}…", c.chars().take(90).collect::<String>().trim_end()),
        Some(c) => c,
        None => format!("Video by @{user}"),
    };
    Ok(MediaInfo {
        title,
        thumbnail: text("display_url").filter(|t| t.starts_with("http")),
        duration: details.get("video_duration").and_then(|d| d.as_f64()).map(|d| d.round() as u64),
        heights: Vec::new(),
        site: Some("Instagram".into()),
        width: number("width"),
        height: number("height"),
        uploader: text("fullname").or_else(|| text("username")),
        uploader_url: text("username").as_deref().and_then(profile_link),
        caption: text("description"),
        likes: details.get("likes").and_then(|v| v.as_u64()),
        comments: details.get("comment_count").and_then(|v| v.as_u64()),
        views: details.get("video_view_count").and_then(|v| v.as_u64()),
    })
}

/// The files gallery-dl left in `work`: not its cookie copy, nor anything unfinished.
fn finished_files(work: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(work)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .filter(|p| {
            let name = p.file_name().unwrap_or_default().to_string_lossy().to_ascii_lowercase();
            !name.starts_with(".rush-") && !name.ends_with(".part") && !name.ends_with(".ytdl")
        })
        .collect();
    files.sort();
    files
}

/// Moves `from` into `dir` under `name`, adding a number when that is taken.
fn place(from: &Path, dir: &Path, name: &str) -> Result<String, Fail> {
    for n in 0..10_000 {
        let candidate = crate::probe::candidate(name, n);
        let dest = dir.join(&candidate);
        if dest.exists() {
            continue;
        }
        std::fs::rename(from, &dest).map_err(|e| Fail::Fatal(format!("could not move the finished file: {e}")))?;
        return Ok(candidate);
    }
    Err(Fail::Fatal("could not find a free file name".into()))
}

async fn run_signed_in(inner: &Arc<Inner>, id: u64, live: &Arc<Live>, rec: &Record) -> Result<(), Fail> {
    let tool = gallery_dl().map_err(Fail::Fatal)?;
    std::fs::create_dir_all(&rec.dir).map_err(|e| Fail::Fatal(format!("could not create folder: {e}")))?;
    let work = crate::media::work_dir(&rec.dir, id);
    // A start over begins clean: gallery-dl skips files it already has.
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).map_err(|e| Fail::Fatal(format!("could not create folder: {e}")))?;
    let cleanup = || crate::media::remove_work_dir(&rec.dir, id);
    inner.update(id, |r| {
        r.media = true;
        r.part_started = true;
        r.status = Status::Downloading;
    });

    let mut cmd = command(&tool);
    cmd.env("PYTHONUTF8", "1").env("PYTHONIOENCODING", "utf-8");
    cmd.arg("-D").arg(&work).arg("-f").arg("{username} {post_shortcode} {num}.{extension}");
    if live.limiter.effective_rate() > 0 {
        cmd.arg("--limit-rate").arg(live.limiter.effective_rate().to_string());
    }
    inner.add_sign_in(&mut cmd, Some(&work), &rec.proxy);
    cmd.arg("--").arg(&rec.url).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|_| Fail::Fatal(UNAVAILABLE.into()))?;
    let stderr = child.stderr.take();
    let errors = tokio::spawn(async move {
        let mut text = String::new();
        if let Some(stderr) = stderr {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                text.push_str(&line);
                text.push('\n');
            }
        }
        text
    });
    // gallery-dl names each file once it has it; that is the progress there is.
    if let Some(stdout) = child.stdout.take() {
        let mut lines = BufReader::new(stdout).lines();
        loop {
            let line = tokio::select! {
                _ = live.cancel.cancelled() => {
                    kill_tree(&mut child).await;
                    cleanup();
                    return Err(Fail::Cancelled);
                }
                l = lines.next_line() => l,
            };
            let Ok(Some(line)) = line else { break };
            let path = PathBuf::from(line.trim().trim_start_matches("# "));
            if let Ok(meta) = std::fs::metadata(&path) {
                live.downloaded.fetch_add(meta.len(), std::sync::atomic::Ordering::Relaxed);
                live.items.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }
    }
    let status = tokio::select! {
        _ = live.cancel.cancelled() => {
            kill_tree(&mut child).await;
            cleanup();
            return Err(Fail::Cancelled);
        }
        s = child.wait() => s.map_err(|_| Fail::Fatal(UNAVAILABLE.into()))?,
    };
    let said = errors.await.unwrap_or_default();
    let mut files = finished_files(&work);
    if files.is_empty() {
        tracing::warn!(id, "gallery-dl could not get the Instagram post: {}", said.trim());
        cleanup();
        if sign_in_ended(inner, &said).await {
            return Err(Fail::Fatal(SIGN_IN_ENDED.into()));
        }
        return Err(Fail::Fatal(if status.success() { UNAVAILABLE } else { classify(&said) }.into()));
    }

    // The sound alone: from the post's (first) video, as MP3.
    if rec.media_options.audio_only {
        let Some(video) = files
            .iter()
            .find(|p| p.extension().is_some_and(|e| matches!(e.to_string_lossy().to_ascii_lowercase().as_str(), "mp4" | "mov" | "webm" | "mkv")))
            .cloned()
        else {
            cleanup();
            return Err(Fail::Fatal(NO_VIDEO.into()));
        };
        let ffmpeg = find_tool("ffmpeg").ok_or_else(|| Fail::Fatal("Saving only the sound needs FFmpeg, which was not found".into()))?;
        let mp3 = video.with_extension("mp3");
        let mut cut = command(&ffmpeg);
        cut.args(["-y", "-v", "error", "-i"]).arg(&video).args(["-vn", "-c:a", "libmp3lame", "-q:a", "0"]).arg(&mp3);
        cut.stdout(Stdio::null()).stderr(Stdio::piped());
        let out = tokio::select! {
            _ = live.cancel.cancelled() => {
                cleanup();
                return Err(Fail::Cancelled);
            }
            o = cut.output() => o.map_err(|e| Fail::Fatal(format!("could not start FFmpeg: {e}")))?,
        };
        if !out.status.success() || !mp3.is_file() {
            let why = String::from_utf8_lossy(&out.stderr);
            cleanup();
            return Err(Fail::Fatal(format!("The sound could not be cut out: {}", why.lines().last().unwrap_or("FFmpeg failed"))));
        }
        files = vec![mp3];
    }

    // One file: named after the post (or the name given); several: a folder.
    let stem_of = |p: &Path| -> String {
        let stem = p.file_stem().unwrap_or_default().to_string_lossy().to_string();
        stem.strip_suffix(" 1").map(str::to_string).unwrap_or(stem)
    };
    let given = rec.user_named.then(|| Path::new(&rec.filename).file_stem().unwrap_or_default().to_string_lossy().to_string());
    let (final_name, size) = if let [one] = files.as_slice() {
        let ext = one.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_else(|| "mp4".into());
        let stem = crate::probe::sanitize(&given.unwrap_or_else(|| stem_of(one)));
        let size = std::fs::metadata(one).map(|m| m.len()).unwrap_or(0);
        (place(one, &rec.dir, &format!("{stem}.{ext}"))?, size)
    } else {
        let folder_base = given.unwrap_or_else(|| {
            let stem = stem_of(&files[0]);
            stem.rsplit_once(' ').map(|(head, _)| head.to_string()).unwrap_or(stem)
        });
        let folder_name = crate::torrent::claim_folder(&rec.dir, &crate::probe::sanitize(&folder_base))?;
        let folder = rec.dir.join(&folder_name);
        let mut size = 0;
        for file in &files {
            size += std::fs::metadata(file).map(|m| m.len()).unwrap_or(0);
            let name = file.file_name().unwrap_or_default().to_string_lossy().to_string();
            place(file, &folder, &name)?;
        }
        (folder_name, size)
    };
    live.downloaded.store(size, std::sync::atomic::Ordering::Relaxed);
    live.total_hint.store(size, std::sync::atomic::Ordering::Relaxed);
    inner.update(id, |r| {
        r.filename = final_name.clone();
        r.total = Some(size);
    });
    cleanup();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_username_gives_its_profile_page() {
        assert_eq!(profile_link("some.one_99").as_deref(), Some("https://www.instagram.com/some.one_99/"));
        assert_eq!(profile_link("@someone").as_deref(), Some("https://www.instagram.com/someone/"));
        for bad in ["", "two words", "a/b", "x?y", &"n".repeat(31)] {
            assert_eq!(profile_link(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn yt_dlp_details_name_the_poster() {
        // Instagram's reader names the account but may give no link
        let reel = serde_json::json!({ "title": "Video by someone", "extractor_key": "Instagram", "channel": "someone", "uploader": "Some One" });
        let found = crate::media::read_info_for_tests(&reel).unwrap();
        assert_eq!(found.uploader.as_deref(), Some("Some One"));
        assert_eq!(found.uploader_url.as_deref(), Some("https://www.instagram.com/someone/"));
        // the post's caption and numbers, where yt-dlp gives them
        let counted = serde_json::json!({ "title": "Video by someone", "description": " Hello there 
", "like_count": 1234, "comment_count": 56 });
        let found = crate::media::read_info_for_tests(&counted).unwrap();
        assert_eq!(found.caption.as_deref(), Some("Hello there"));
        assert_eq!((found.likes, found.comments, found.views), (Some(1234), Some(56), None));
        // other sites: their own link, as given
        let video = serde_json::json!({ "title": "A talk", "extractor_key": "Youtube", "uploader": "A Channel", "uploader_url": "https://www.youtube.com/@achannel" });
        let found = crate::media::read_info_for_tests(&video).unwrap();
        assert_eq!(found.uploader_url.as_deref(), Some("https://www.youtube.com/@achannel"));
        // nothing that is not a web address is passed on
        let odd = serde_json::json!({ "title": "x", "uploader_url": "javascript:alert(1)" });
        assert_eq!(crate::media::read_info_for_tests(&odd).unwrap().uploader_url, None);
    }

    #[test]
    fn links_are_told_apart_by_kind() {
        let reel = "https://www.instagram.com/reel/DgNy4xCSXwj/?utm_source=ig_web_copy_link&lqvp=NTc4MTIwNjQ2YQ==";
        assert_eq!(post_of(reel), Some((Kind::Reel, "DgNy4xCSXwj".to_string())));
        assert_eq!(post_of("https://instagram.com/p/ABC123xyz/").map(|p| p.0), Some(Kind::Post));
        assert_eq!(post_of("https://www.instagram.com/reels/ABC123xyz").map(|p| p.0), Some(Kind::Reel));
        assert_eq!(post_of("https://www.instagram.com/tv/ABC123xyz/").map(|p| p.0), Some(Kind::Tv));
        assert_eq!(post_of("https://www.instagram.com/someone/p/ABC123xyz/").map(|p| p.1), Some("ABC123xyz".to_string()));
        for not_a_post in [
            "https://www.instagram.com/someone/",
            "https://www.instagram.com/",
            "https://www.instagram.com/p/",
            "https://www.instagram.com/p/ab/",
            "https://www.instagram.com/p/bad%20code/",
            "https://example.com/reel/ABC123xyz",
            "ftp://www.instagram.com/p/ABC123xyz/",
            "not a link",
        ] {
            assert!(!is_post(not_a_post), "{not_a_post}");
        }
        assert!(is_instagram("https://www.instagram.com/someone/"));
        assert!(!is_instagram("https://example.com/p/ABC123xyz"));
    }

    #[test]
    fn instagrams_refusals_are_told_in_plain_words() {
        for (said, told) in [
            ("ERROR: [Instagram] X: This content isn't available to everyone: It can't be seen by certain audiences.", AGE_RESTRICTED),
            ("ERROR: [Instagram] X: Restricted Video: You must be 18 years old or over to see this.", AGE_RESTRICTED),
            ("ERROR: [Instagram] X: Requested content is not available, rate-limit reached or login required. Use --cookies", NEEDS_SIGN_IN),
            ("[instagram][error] HttpError: '401 Unauthorized' for 'https://www.instagram.com/api/v1/media/1/info/'", NEEDS_SIGN_IN),
            ("ERROR: [Instagram] X: This account is private", PRIVATE),
            ("ERROR: [Instagram] X: There is no video in this post", NO_VIDEO),
            ("ERROR: [Instagram] X: HTTP Error 404: Not Found", UNAVAILABLE),
            (
                "ERROR: [Instagram] X: Instagram sent an empty media response. Check if this post is accessible in your browser without being logged-in. If it is not, then use --cookies-from-browser or --cookies",
                UNAVAILABLE,
            ),
            ("ERROR: [Instagram] X: Unable to extract data", UNAVAILABLE),
            // gallery-dl with a sign-in Instagram has stopped accepting
            ("[instagram][error] HTTP redirect to home page (https://www.instagram.com/)", SIGN_IN_REFUSED),
            ("[instagram][error] HTTP redirect to login page (https://www.instagram.com/accounts/login/)", SIGN_IN_REFUSED),
            ("[instagram][error] HTTP redirect to challenge page (https://www.instagram.com/challenge/)", SIGN_IN_REFUSED),
        ] {
            assert_eq!(classify(said), told, "{said}");
        }
        // These messages never send the user off to sign in; only the
        // age-restricted one points at the Sign in page, on purpose, since
        // that is the one way through Instagram's own gate.
        for message in [INVALID, PRIVATE, UNAVAILABLE, NEEDS_SIGN_IN, NO_VIDEO] {
            assert!(!message.to_ascii_lowercase().contains("sign in on") && !message.contains("Sign in page"), "{message}");
        }
        assert!(AGE_RESTRICTED.contains("Sign in page"));
        // ... and the one said only to a user who had already signed in
        assert!(SIGN_IN_REFUSED.contains("Sign in page"));
    }

    #[test]
    fn a_public_profiles_page_lists_its_newest_posts() {
        // the shape Instagram writes into a profile's page for a visitor who is not signed in
        let page = r#"<html><head><script type="application/json" data-sjs>{"require":[["x",null,null,[{"__bbox":{"result":{"data":
            {"xig_user_by_username":{"polaris_ordered_timeline_connection":{"edges":[
              {"node":{"__typename":"XIGPolarisVideoMedia","code":"DgRvfmrpsPN","media_type":2,"product_type":"clips"},"cursor":"a"},
              {"node":{"__typename":"XIGPolarisCarouselMedia","code":"DgRoIDWI7tl","media_type":8,"carousel_media_count":4},"cursor":"b"},
              {"node":{"__typename":"XIGPolarisImageMedia","code":"DgMLuV8IZDS","media_type":1},"cursor":"c"},
              {"node":{"code":"DgMLuV8IZDS","media_type":1}},
              {"node":{"code":"bad code","media_type":1}}
            ],"page_info":{"end_cursor":"c","has_next_page":true}}}}}}}]]]}</script>
            <script type="application/json">{"other":"polaris_ordered_timeline_connection"}</script></head></html>"#;
        let found = read_public_page(page);
        let post = |code: &str, video, pieces| PagePost { code: code.to_string(), video, pieces };
        assert_eq!(found.posts, vec![post("DgRvfmrpsPN", true, 1), post("DgRoIDWI7tl", false, 4), post("DgMLuV8IZDS", false, 1)]);
        assert!(found.more);
        // a private account's page, a sign-in page, or nothing like a page at all
        for nothing in ["", "<html><body>Log in</body></html>", "<script>polaris_ordered_timeline_connection</script>"] {
            assert_eq!(read_public_page(nothing), PublicPage::default(), "{nothing}");
        }
    }

    #[test]
    fn a_posts_photos_and_videos_are_told_apart() {
        // one reel
        let reel = serde_json::json!({ "id": "A", "formats": [{ "url": "https://cdn.test/v.mp4" }], "thumbnail": "https://cdn.test/v.jpg" });
        assert_eq!(read_post(&reel), PostFiles { videos: vec![1], photos: vec![], several: false });
        // one photo: nothing to play, only its picture
        let photo = serde_json::json!({ "id": "B", "formats": [], "thumbnail": "https://cdn.test/b.jpg" });
        assert_eq!(read_post(&photo), PostFiles { videos: vec![], photos: vec![(1, "https://cdn.test/b.jpg".into())], several: false });
        // several in one post, a video among the photos
        let mixed = serde_json::json!({ "_type": "playlist", "entries": [
            { "id": "1", "thumbnail": "https://cdn.test/1.jpg" },
            { "id": "2", "formats": [{ "url": "https://cdn.test/2.mp4" }], "thumbnail": "https://cdn.test/2.jpg" },
            { "id": "3", "thumbnail": "javascript:alert(1)" },
            { "id": "4", "formats": [], "thumbnail": "https://cdn.test/4.jpg" },
        ] });
        let found = read_post(&mixed);
        assert_eq!(found.videos, vec![2]);
        assert_eq!(found.photos, vec![(1, "https://cdn.test/1.jpg".to_string()), (4, "https://cdn.test/4.jpg".to_string())]);
        assert!(found.several);
    }

    #[test]
    fn an_ended_sign_in_is_let_go_of_and_nothing_else() {
        let line = |host: &str, name: &str| format!("{host}\tTRUE\t/\tTRUE\t1900000000\t{name}\tv-{name}");
        let file = [
            "# Netscape HTTP Cookie File".to_string(),
            line("#HttpOnly_.instagram.com", "mid"),
            line(".instagram.com", "sessionid"),
            line("#HttpOnly_.instagram.com", "sessionid"),
            line(".instagram.com", "ds_user_id"),
            line(".instagram.com", "rur"),
            line(".instagram.com", "csrftoken"),
            // another site's sign-in, under the same name, is not Instagram's
            line(".tiktok.com", "sessionid"),
            line(".notinstagram.com", "sessionid"),
        ]
        .join("\n");
        let kept = without_session(&file).expect("there was a sign-in");
        let names: Vec<(&str, &str)> = kept.lines().filter_map(instagram_cookie).collect();
        assert_eq!(names, vec![("mid", "v-mid"), ("csrftoken", "v-csrftoken")]);
        assert!(kept.contains(".tiktok.com\t") && kept.contains(".notinstagram.com\t") && kept.starts_with("# Netscape"));
        // nothing to let go of: the file is left alone
        assert_eq!(without_session(&kept), None);

        // Instagram shows a visitor it knows their settings, and sends anyone else away
        let at = |link: &str| shows_settings(&reqwest::Url::parse(link).unwrap());
        assert!(at("https://www.instagram.com/accounts/edit/"));
        assert!(!at("https://www.instagram.com/"));
        assert!(!at("https://www.instagram.com/accounts/login/?next=%2Faccounts%2Fedit%2F"));
    }

    #[test]
    fn reads_what_gallery_dl_lists() {
        let listing: serde_json::Value = serde_json::from_str(
            r#"[[2, {"username": "someone"}],
                [3, "ytdl:https://www.instagram.com/p/X/1.mp4", {"extension": "mp4", "username": "someone", "description": "Hello\nworld", "display_url": "https://cdn.test/a.jpg", "video_duration": 12.4, "width": 1080, "height": 1920}],
                [3, "https://cdn.test/b.jpg", {"extension": "jpg"}]]"#,
        )
        .unwrap();
        let files = files_in(&listing);
        assert_eq!(files.len(), 2);
        assert!(is_video(&files[0].0, &files[0].1));
        assert!(!is_video(&files[1].0, &files[1].1));
        let refused: serde_json::Value = serde_json::from_str(r#"[[-1, {"error": "HttpError", "message": "'401 Unauthorized' for 'x'"}]]"#).unwrap();
        assert_eq!(listed_error(&refused).as_deref(), Some("'401 Unauthorized' for 'x'"));
        assert_eq!(classify(&listed_error(&refused).unwrap()), NEEDS_SIGN_IN);
    }
}
