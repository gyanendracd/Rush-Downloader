//! Public Instagram posts, read the way Instagram's own page reads them for
//! a visitor who is not signed in.
//!
//! For a post, that page asks Instagram one question ("what does this post
//! hold, for someone who is not signed in?") and gets its photos and videos
//! back. This asks the same question, sending what any visitor's browser
//! sends: the cookies and the token Instagram's front page hands to
//! everyone. No account and no sign-in are involved, and nothing of the
//! user's own cookies is sent.
//!
//! It is the second way in (instagram.rs). yt-dlp reads a post first; this
//! is asked when yt-dlp cannot: it has nothing to give for a post of
//! photos, and its reader breaks whenever Instagram changes. What Instagram
//! keeps for signed-in viewers stays there: such a post comes back empty
//! here too, and is reported so.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::instagram::{classify, post_of, profile_link, AGE_RESTRICTED, INVALID, NEEDS_SIGN_IN, NO_VIDEO, PRIVATE, UNAVAILABLE};
use crate::media::{find_tool, MediaInfo, Plan, Stream};
use crate::postfiles::{picture_ext, PostFile};
use crate::profile::BROWSER_AGENT;
use crate::task::{Fail, Live};
use crate::{Inner, Record, Status};

const FRONT_PAGE: &str = "https://www.instagram.com/";
const QUESTIONS: &str = "https://www.instagram.com/api/graphql";
/// The question Instagram's page asks for a post when nobody is signed in:
/// its name, and the number Instagram knows it by.
const QUESTION: &str = "PolarisLoggedOutDesktopWWWPostRootContentQuery";
const QUESTION_NUMBER: &str = "27130156389949648";
/// How Instagram's website names itself to Instagram.
const WEBSITE: &str = "936619743392459";

/// Why a post could not be read this way.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Refused {
    /// Instagram answered, and this is what the answer means (one of
    /// instagram.rs's messages).
    Told(&'static str),
    /// Instagram could not be asked, or gave no answer that can be read.
    Unreachable(String),
}

/* ---------- a visit: what Instagram's front page hands every visitor ---------- */

#[derive(Clone)]
struct Visitor {
    /// The cookies of the visit, as a `Cookie` header.
    cookies: String,
    csrf: String,
    /// The token the page's own questions carry.
    token: String,
}

/// One visit serves the posts asked for in the next few minutes, as one
/// opened page does in a browser.
static VISIT: Mutex<Option<(Instant, Visitor)>> = Mutex::new(None);
const VISIT_KEPT: Duration = Duration::from_secs(600);

/// The token in Instagram's page: `["LSD",[],{"token":"…"}`.
fn token_in(page: &str) -> Option<String> {
    let mark = r#""LSD",[],{"token":""#;
    let rest = &page[page.find(mark)? + mark.len()..];
    let token = &rest[..rest.find('"')?];
    let valid = !token.is_empty() && token.len() <= 200 && token.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    valid.then(|| token.to_string())
}

/// The name and value in one `Set-Cookie` header.
fn cookie_of(header: &str) -> Option<(String, String)> {
    let (name, value) = header.split(';').next()?.split_once('=')?;
    let (name, value) = (name.trim(), value.trim());
    let plain = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_graphic() && !matches!(c, ';' | ',' | '"'));
    (plain(name) && plain(value)).then(|| (name.to_string(), value.to_string()))
}

async fn visit(client: &reqwest::Client, anew: bool) -> Result<Visitor, Refused> {
    if !anew {
        if let Some((at, visitor)) = VISIT.lock().unwrap().as_ref() {
            if at.elapsed() < VISIT_KEPT {
                return Ok(visitor.clone());
            }
        }
    }
    let unreachable = |e: reqwest::Error| Refused::Unreachable(format!("Instagram could not be reached: {e}"));
    let answer = client
        .get(FRONT_PAGE)
        .header(reqwest::header::USER_AGENT, BROWSER_AGENT)
        .header(reqwest::header::ACCEPT, "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")
        .header(reqwest::header::ACCEPT_LANGUAGE, "en-US,en;q=0.9")
        .header("Sec-Fetch-Dest", "document")
        .header("Sec-Fetch-Mode", "navigate")
        .header("Sec-Fetch-Site", "none")
        .send()
        .await
        .map_err(unreachable)?;
    if !answer.status().is_success() {
        return Err(Refused::Unreachable(format!("Instagram answered {} for its front page", answer.status().as_u16())));
    }
    let mut cookies: Vec<(String, String)> = Vec::new();
    for header in answer.headers().get_all(reqwest::header::SET_COOKIE) {
        if let Some((name, value)) = header.to_str().ok().and_then(cookie_of) {
            cookies.retain(|c| c.0 != name);
            cookies.push((name, value));
        }
    }
    let page = answer.text().await.map_err(unreachable)?;
    let token = token_in(&page).ok_or_else(|| Refused::Unreachable("Instagram's page gave no visitor token".into()))?;
    let visitor = Visitor {
        csrf: cookies.iter().find(|c| c.0 == "csrftoken").map(|c| c.1.clone()).unwrap_or_default(),
        cookies: cookies.iter().map(|(name, value)| format!("{name}={value}")).collect::<Vec<_>>().join("; "),
        token,
    };
    *VISIT.lock().unwrap() = Some((Instant::now(), visitor.clone()));
    Ok(visitor)
}

/* ---------- asking for a post ---------- */

/// Instagram's own number for a post, which its code in the link spells in
/// 64 letters. (A link shared from a private account carries 28 letters
/// more, which are not part of the number.)
pub(crate) fn media_id(code: &str) -> Option<String> {
    const LETTERS: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let code = if code.len() > 28 { &code[..code.len() - 28] } else { code };
    if code.is_empty() || code.len() > 20 {
        return None;
    }
    let mut number: u128 = 0;
    for letter in code.chars() {
        number = number * 64 + LETTERS.find(letter)? as u128;
    }
    Some(number.to_string())
}

fn form(fields: &[(&str, &str)]) -> String {
    use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
    fields.iter().map(|(name, value)| format!("{name}={}", utf8_percent_encode(value, NON_ALPHANUMERIC))).collect::<Vec<_>>().join("&")
}

/// Asks Instagram what a post holds for a visitor, and gives its answer.
async fn ask(client: &reqwest::Client, code: &str) -> Result<serde_json::Value, Refused> {
    let id = media_id(code).ok_or(Refused::Told(INVALID))?;
    let variables = format!(r#"{{"media_id":"{id}"}}"#);
    let mut last = Refused::Unreachable("Instagram gave no answer".into());
    // A kept visit Instagram no longer honours is made anew, once.
    for anew in [false, true] {
        let visitor = visit(client, anew).await?;
        let body = form(&[
            ("av", "0"),
            ("__d", "www"),
            ("__user", "0"),
            ("dpr", "1"),
            ("lsd", &visitor.token),
            ("fb_api_caller_class", "RelayModern"),
            ("fb_api_req_friendly_name", QUESTION),
            ("variables", &variables),
            ("doc_id", QUESTION_NUMBER),
        ]);
        let answer = client
            .post(QUESTIONS)
            .header(reqwest::header::USER_AGENT, BROWSER_AGENT)
            .header(reqwest::header::ACCEPT, "*/*")
            .header(reqwest::header::ACCEPT_LANGUAGE, "en-US,en;q=0.9")
            .header(reqwest::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .header(reqwest::header::ORIGIN, "https://www.instagram.com")
            .header(reqwest::header::REFERER, format!("https://www.instagram.com/p/{code}/"))
            .header(reqwest::header::COOKIE, &visitor.cookies)
            .header("X-IG-App-ID", WEBSITE)
            .header("X-ASBD-ID", "359341")
            .header("X-IG-WWW-Claim", "0")
            .header("X-FB-LSD", &visitor.token)
            .header("X-FB-Friendly-Name", QUESTION)
            .header("X-CSRFToken", &visitor.csrf)
            .header("X-Requested-With", "XMLHttpRequest")
            .header("Sec-Fetch-Site", "same-origin")
            .body(body)
            .send()
            .await
            .map_err(|e| Refused::Unreachable(format!("Instagram could not be reached: {e}")))?;
        let status = answer.status().as_u16();
        if status == 429 {
            return Err(Refused::Unreachable("Instagram is limiting requests right now. Wait a while and retry".into()));
        }
        let text = answer.text().await.map_err(|e| Refused::Unreachable(format!("Instagram's answer could not be read: {e}")))?;
        let read = serde_json::from_str::<serde_json::Value>(text.trim_start_matches("for (;;);")).ok().filter(|v| v.get("data").is_some());
        match read {
            Some(read) if (200..300).contains(&status) => return Ok(read),
            _ => last = Refused::Unreachable(format!("Instagram answered {status} to a visitor's question about this post")),
        }
    }
    *VISIT.lock().unwrap() = None;
    Err(last)
}

/* ---------- what a post holds ---------- */

/// One size of a video's picture, or its sound, as a file of its own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Rendition {
    pub url: String,
    /// Its size in bytes, where Instagram states it.
    pub size: Option<u64>,
    pub width: u32,
    pub height: u32,
    /// Bits a second.
    pub rate: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Video {
    /// Picture and sound in one file, as Instagram plays it where it cannot
    /// play them apart: smaller than the original.
    pub whole: Option<String>,
    /// The picture alone, in every size Instagram keeps.
    pub pictures: Vec<Rendition>,
    /// The sound alone.
    pub sound: Option<Rendition>,
    /// Its length in seconds.
    pub seconds: Option<u64>,
}

/// One photo or video of a post.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Piece {
    /// The photo in its largest size (for a video, its cover).
    pub photo: Option<String>,
    pub video: Option<Video>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Post {
    pub code: String,
    pub username: String,
    pub full_name: Option<String>,
    pub caption: Option<String>,
    pub likes: Option<u64>,
    pub comments: Option<u64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// The picture Instagram shows for the post.
    pub picture: Option<String>,
    pub pieces: Vec<Piece>,
}

fn https(value: Option<&serde_json::Value>) -> Option<String> {
    value?.as_str().filter(|a| a.starts_with("https://")).map(str::to_string)
}

/// One attribute of an opening tag: ` name="value"`.
fn attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let mark = format!(" {name}=\"");
    let start = tag.find(&mark)? + mark.len();
    Some(&tag[start..start + tag[start..].find('"')?])
}

fn unescaped(text: &str) -> String {
    text.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

/// A length as a stream list writes it: "PT1M31.7S".
fn seconds_in(length: &str) -> Option<u64> {
    let mut total = 0f64;
    let mut number = String::new();
    for c in length.strip_prefix("PT")?.chars() {
        match c {
            '0'..='9' | '.' => number.push(c),
            'H' | 'M' | 'S' => {
                let n: f64 = number.parse().ok()?;
                total += n * [3600.0, 60.0, 1.0][['H', 'M', 'S'].iter().position(|u| *u == c)?];
                number.clear();
            }
            _ => return None,
        }
    }
    number.is_empty().then(|| total.round() as u64)
}

/// Reads the list of streams Instagram gives with a video (a DASH manifest
/// of whole files): the picture's sizes, the sound, and the length.
pub(crate) fn read_streams(list: &str) -> (Vec<Rendition>, Option<Rendition>, Option<u64>) {
    let (mut pictures, mut sound) = (Vec::new(), None);
    for set in list.split("<AdaptationSet").skip(1) {
        let kind = attribute(set.split('>').next().unwrap_or_default(), "contentType");
        for one in set.split("<Representation").skip(1) {
            let tag = one.split('>').next().unwrap_or_default();
            let body = one.split("</Representation>").next().unwrap_or_default();
            let Some(url) = body.split_once("<BaseURL>").and_then(|(_, rest)| rest.split_once("</BaseURL>")).map(|(url, _)| unescaped(url.trim())) else { continue };
            if !url.starts_with("https://") {
                continue;
            }
            let number = |name: &str| attribute(tag, name).and_then(|v| v.parse::<u64>().ok());
            let found = Rendition {
                url,
                size: number("FBContentLength").filter(|s| *s > 0),
                width: number("width").unwrap_or(0) as u32,
                height: number("height").unwrap_or(0) as u32,
                rate: number("bandwidth").unwrap_or(0),
            };
            let is_sound = kind == Some("audio") || attribute(tag, "mimeType").is_some_and(|m| m.starts_with("audio/"));
            if !is_sound {
                pictures.push(found);
            } else if !sound.as_ref().is_some_and(|s: &Rendition| found.rate <= s.rate) {
                sound = Some(found);
            }
        }
    }
    let length = list.split('>').find(|tag| tag.contains("<MPD")).and_then(|tag| attribute(tag, "mediaPresentationDuration")).and_then(seconds_in);
    (pictures, sound, length)
}

impl Video {
    /// The picture to fetch: the largest, or the largest no taller than
    /// asked for (the smallest there is, when all are taller).
    pub(crate) fn picture(&self, max_height: Option<u32>) -> Option<&Rendition> {
        let weight = |r: &&Rendition| (r.width as u64 * r.height as u64, r.rate);
        match max_height {
            Some(h) => self.pictures.iter().filter(|r| r.height <= h).max_by_key(weight).or_else(|| self.pictures.iter().min_by_key(weight)),
            None => self.pictures.iter().max_by_key(weight),
        }
    }

    /// The files to fetch for this video. Picture and sound apart need
    /// FFmpeg to join them; without it, the one file that holds both.
    fn streams(&self, max_height: Option<u32>, audio_only: bool, can_join: bool) -> Vec<Stream> {
        let stream = |url: &str, ext: &str, size| Stream {
            url: url.to_string(),
            headers: vec![("User-Agent".into(), BROWSER_AGENT.into()), ("Referer".into(), FRONT_PAGE.into())],
            ext: ext.to_string(),
            request_cap: 0,
            size,
        };
        let whole = || self.whole.iter().map(|url| stream(url, "mp4", None)).collect::<Vec<_>>();
        match (self.picture(max_height), &self.sound) {
            (_, Some(sound)) if audio_only => vec![stream(&sound.url, "m4a", sound.size)],
            _ if audio_only => whole(),
            (Some(picture), Some(sound)) if can_join => vec![stream(&picture.url, "mp4", picture.size), stream(&sound.url, "m4a", sound.size)],
            // a video with no sound: its picture is all of it
            (Some(picture), None) => vec![stream(&picture.url, "mp4", picture.size)],
            (Some(picture), _) if self.whole.is_none() && !can_join => vec![stream(&picture.url, "mp4", picture.size)],
            _ => whole(),
        }
    }
}

fn piece_in(node: &serde_json::Value) -> Option<Piece> {
    // The photo: Instagram lists its sizes, the original first.
    let sizes = node.get("image_versions2").and_then(|i| i.get("candidates")).and_then(|c| c.as_array());
    let area = |c: &serde_json::Value| c.get("width").and_then(|w| w.as_u64()).unwrap_or(0) * c.get("height").and_then(|h| h.as_u64()).unwrap_or(0);
    let photo = sizes
        .into_iter()
        .flatten()
        .filter(|c| https(c.get("url")).is_some())
        .enumerate()
        .max_by_key(|(place, c)| (area(c), std::cmp::Reverse(*place)))
        .and_then(|(_, c)| https(c.get("url")))
        .or_else(|| https(node.get("display_uri")));

    let whole = node.get("video_versions").and_then(|v| v.as_array()).and_then(|v| v.iter().find_map(|one| https(one.get("url"))));
    let (pictures, sound, seconds) = node.get("video_dash_manifest").and_then(|m| m.as_str()).map(read_streams).unwrap_or_default();
    let video = (whole.is_some() || !pictures.is_empty()).then_some(Video { whole, pictures, sound, seconds });
    (photo.is_some() || video.is_some()).then_some(Piece { photo, video })
}

/// Reads Instagram's answer about a post.
pub(crate) fn read(answer: &serde_json::Value) -> Result<Post, &'static str> {
    // Nothing at all: no such post, or one of an account that is private.
    let Some(media) = answer.get("data").and_then(|d| d.get("xig_polaris_media")).filter(|m| m.is_object()) else {
        return Err(UNAVAILABLE);
    };
    // Instagram has the post, and keeps it from a visitor who is not signed in.
    let Some(shown) = media.get("if_not_gated_logged_out").filter(|s| s.is_object()) else {
        let ruling = media.get("gating_ruling").filter(|r| !r.is_null()).map(|r| r.to_string()).unwrap_or_default();
        return Err(if classify(&ruling) == AGE_RESTRICTED { AGE_RESTRICTED } else { NEEDS_SIGN_IN });
    };
    let user = shown.get("user");
    let text = |from: Option<&serde_json::Value>, key: &str| from?.get(key)?.as_str().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    let pieces: Vec<Piece> = match shown.get("carousel_media").and_then(|c| c.as_array()) {
        Some(several) if !several.is_empty() => several.iter().filter_map(piece_in).collect(),
        _ => piece_in(shown).into_iter().collect(),
    };
    if pieces.is_empty() {
        let private = user.and_then(|u| u.get("is_private")).and_then(|p| p.as_bool()) == Some(true);
        return Err(if private { PRIVATE } else { UNAVAILABLE });
    }
    let side = |key: &str| shown.get(key).and_then(|v| v.as_u64()).filter(|n| *n > 0).map(|n| n as u32);
    Ok(Post {
        code: text(Some(shown), "code").or_else(|| text(Some(media), "code")).unwrap_or_default(),
        username: text(user, "username").filter(|name| profile_link(name).is_some()).unwrap_or_default(),
        full_name: text(user, "full_name"),
        caption: text(shown.get("caption"), "text"),
        likes: shown.get("like_count").and_then(|v| v.as_u64()),
        comments: shown.get("comment_count").and_then(|v| v.as_u64()),
        width: side("original_width"),
        height: side("original_height"),
        picture: https(shown.get("display_uri")).or_else(|| pieces[0].photo.clone()),
        pieces,
    })
}

impl Post {
    fn first_video(&self) -> Option<&Video> {
        self.pieces.iter().find_map(|p| p.video.as_ref())
    }

    /// "Video by", "Photo by", or "Post by" for several things in one.
    fn kind(&self) -> &'static str {
        match self.pieces.as_slice() {
            [one] if one.video.is_some() => "Video",
            [_] => "Photo",
            _ => "Post",
        }
    }

    fn by(&self) -> &str {
        if self.username.is_empty() { "Instagram" } else { &self.username }
    }

    /// What is shown about the post before it is downloaded.
    pub(crate) fn details(&self) -> MediaInfo {
        let named = !self.username.is_empty();
        MediaInfo {
            title: if named { format!("{} by @{}", self.kind(), self.username) } else { format!("Instagram {}", self.kind().to_lowercase()) },
            thumbnail: self.picture.clone(),
            duration: self.first_video().and_then(|v| v.seconds),
            heights: Vec::new(),
            site: Some("Instagram".into()),
            width: self.width,
            height: self.height,
            uploader: self.full_name.clone().or_else(|| named.then(|| self.username.clone())),
            uploader_url: profile_link(&self.username),
            caption: self.caption.clone(),
            likes: self.likes,
            comments: self.comments,
            views: None,
        }
    }
}

/// A post as a visitor sees it: asked for and read.
pub(crate) async fn post(client: &reqwest::Client, url: &str) -> Result<Post, Refused> {
    let (_, code) = post_of(url).ok_or(Refused::Told(INVALID))?;
    let answer = ask(client, &code).await?;
    let mut found = read(&answer).map_err(Refused::Told)?;
    if found.code.is_empty() {
        found.code = code;
    }
    Ok(found)
}

/* ---------- saving it ---------- */

/// A post of one video (or the sound of a post's first video): its picture
/// and sound fetched with many connections and joined, as a reel read by
/// yt-dlp is (media.rs), and named the same way.
async fn save_video(inner: &Arc<Inner>, id: u64, live: &Arc<Live>, rec: &Record, post: &Post, video: &Video) -> Result<(), Fail> {
    let options = rec.media_options;
    let streams = video.streams(options.max_height, options.audio_only, find_tool("ffmpeg").is_some());
    if streams.is_empty() {
        return Err(Fail::Fatal(UNAVAILABLE.into()));
    }
    let stem = if rec.user_named {
        inner.claim_named_stem(id, &crate::media::named_base(rec))
    } else {
        let copy = inner.claim_media_copy(id);
        let again = if copy == 0 { String::new() } else { format!(" ({})", copy + 1) };
        let height = video.picture(options.max_height).map(|p| p.height).filter(|h| *h > 0 && !options.audio_only && streams.len() > 1);
        let size = height.map(|h| format!("{h}p ")).unwrap_or_default();
        crate::probe::sanitize(&format!("Video by {} {size}[{}]{again}", post.by(), post.code))
    };
    let work = crate::media::work_dir(&rec.dir, id);
    std::fs::create_dir_all(&work).map_err(|e| Fail::Fatal(format!("could not create folder: {e}")))?;
    inner.update(id, |r| r.resumable = true);
    crate::media::fetch_direct(inner, id, live, rec, &Plan { streams, stem }, &work, &rec.segments).await
}

/// A post's photos, and the videos among them, as the files they become
/// (postfiles.rs): one file for one photo, a folder named after the post
/// for several.
fn files_of(post: &Post, max_height: Option<u32>, can_join: bool) -> Vec<PostFile> {
    post.pieces
        .iter()
        .filter_map(|piece| match (&piece.video, &piece.photo) {
            (Some(video), _) => {
                let parts: Vec<(String, String)> = video.streams(max_height, false, can_join).into_iter().map(|s| (s.url, s.ext)).collect();
                (!parts.is_empty()).then(|| PostFile { parts, ext: "mp4".into() })
            }
            (None, Some(photo)) => Some(PostFile::whole(photo, picture_ext(photo))),
            (None, None) => None,
        })
        .collect()
}

/// Saves a post that was read with [`post`]: a video as one file, a photo
/// as one file, several things as a folder; or the sound of its video.
pub(crate) async fn save(inner: &Arc<Inner>, id: u64, live: &Arc<Live>, rec: &Record, post: &Post) -> Result<(), Fail> {
    std::fs::create_dir_all(&rec.dir).map_err(|e| Fail::Fatal(format!("could not create folder: {e}")))?;
    inner.update(id, |r| {
        r.media = true;
        r.status = Status::Downloading;
        if post.picture.is_some() && r.thumbnail != post.picture {
            r.thumbnail = post.picture.clone();
        }
    });
    let alone = match post.pieces.as_slice() {
        [one] => one.video.as_ref(),
        _ => None,
    };
    if rec.media_options.audio_only {
        let video = post.first_video().ok_or_else(|| Fail::Fatal(NO_VIDEO.into()))?;
        return save_video(inner, id, live, rec, post, video).await;
    }
    if let Some(video) = alone {
        return save_video(inner, id, live, rec, post, video).await;
    }
    let files = files_of(post, rec.media_options.max_height, find_tool("ffmpeg").is_some());
    if files.is_empty() {
        return Err(Fail::Fatal(UNAVAILABLE.into()));
    }
    crate::postfiles::save(inner, id, live, rec, &format!("{} {}", post.by(), post.code), FRONT_PAGE, &files).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_posts_code_spells_its_number() {
        assert_eq!(media_id("DgSGcWtg4Tx").as_deref(), Some("4040320152515085553"));
        assert_eq!(media_id("DgPIyUdDLu3").as_deref(), Some("4039486033082760119"));
        assert_eq!(media_id("Df147tCiAyB").as_deref(), Some("4032379434794749057"));
        // a link shared from a private account: 28 letters more, not part of the number
        assert_eq!(media_id(&format!("DgSGcWtg4Tx{}", "a".repeat(28))).as_deref(), Some("4040320152515085553"));
        for bad in ["", "has space", "ümlaut", &"A".repeat(25)] {
            assert_eq!(media_id(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn a_visit_gives_its_token_and_cookies() {
        let page = r#"<script>{"require":[["x"]],"define":[["LSD",[],{"token":"AdQ-x_9.Z"},323],["Other",[],{"token":"no"}]]}</script>"#;
        assert_eq!(token_in(page).as_deref(), Some("AdQ-x_9.Z"));
        for none in ["", "<html>Log in</html>", r#"["LSD",[],{"token":""}]"#, r#"["LSD",[],{"token":"a b"}]"#] {
            assert_eq!(token_in(none), None, "{none}");
        }
        assert_eq!(cookie_of("csrftoken=abc123; Domain=.instagram.com; expires=Sat, 27-Nov-2027 10:00:00 GMT; Path=/; Secure"), Some(("csrftoken".into(), "abc123".into())));
        assert_eq!(cookie_of("mid=aS-x; HttpOnly"), Some(("mid".into(), "aS-x".into())));
        for none in ["", "novalue", "name=; Path=/", "na me=v", "n=v\"x"] {
            assert_eq!(cookie_of(none), None, "{none}");
        }
        // what is sent with a question is spelt out safely
        assert_eq!(form(&[("lsd", "A-b_c"), ("variables", r#"{"media_id":"1"}"#)]), "lsd=A%2Db%5Fc&variables=%7B%22media%5Fid%22%3A%221%22%7D");
    }

    /// The list of streams Instagram gives with a video, cut to its shape.
    const STREAMS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" profiles="urn:mpeg:dash:profile:isoff-on-demand:2011" type="static" mediaPresentationDuration="PT1M31.711998S" FBManifestIdentifier="x">
<Period id="0" duration="PT91.711998S"><AdaptationSet id="0" contentType="video" subsegmentAlignment="true">
<Representation id="1v" bandwidth="144560" codecs="vp09.00.31.08" mimeType="video/mp4" width="720" height="1280" FBQualityLabel="240p" FBContentLength="1656874"><BaseURL>https://cdn.test/v240.mp4?a=1&amp;b=2</BaseURL><SegmentBase indexRange="100-200"><Initialization range="0-99"/></SegmentBase></Representation>
<Representation id="2v" bandwidth="817757" codecs="vp09.00.31.08" mimeType="video/mp4" width="720" height="1280" FBQualityLabel="720p" FBContentLength="9372693"><BaseURL>https://cdn.test/v720.mp4</BaseURL></Representation>
<Representation id="3v" bandwidth="1320791" codecs="vp09.00.40.08" mimeType="video/mp4" width="1080" height="1920" FBQualityLabel="1080p" FBContentLength="15138196"><BaseURL>https://cdn.test/v1080.mp4</BaseURL></Representation>
<Representation id="4v" bandwidth="1" mimeType="video/mp4" width="9" height="9"><BaseURL>javascript:alert(1)</BaseURL></Representation>
</AdaptationSet><AdaptationSet id="1" contentType="audio">
<Representation id="1a" bandwidth="49540" codecs="mp4a.40.5" mimeType="audio/mp4" audioSamplingRate="48000" FBContentLength="569324"><BaseURL>https://cdn.test/a.m4a</BaseURL></Representation>
</AdaptationSet></Period></MPD>"#;

    #[test]
    fn a_videos_streams_are_read_and_the_right_ones_chosen() {
        let (pictures, sound, seconds) = read_streams(STREAMS);
        assert_eq!(pictures.len(), 3, "only files on the web are taken");
        assert_eq!(pictures[0], Rendition { url: "https://cdn.test/v240.mp4?a=1&b=2".into(), size: Some(1656874), width: 720, height: 1280, rate: 144560 });
        assert_eq!(sound.as_ref().map(|s| (s.url.as_str(), s.size)), Some(("https://cdn.test/a.m4a", Some(569324))));
        assert_eq!(seconds, Some(92));
        assert_eq!(read_streams("not a list"), (vec![], None, None));
        for (written, means) in [("PT91.4S", Some(91)), ("PT2H1M1S", Some(7261)), ("PT5M", Some(300)), ("P1D", None), ("PT5", None), ("PTxS", None)] {
            assert_eq!(seconds_in(written), means, "{written}");
        }

        let video = Video { whole: Some("https://cdn.test/whole.mp4".into()), pictures, sound, seconds };
        // the original: the largest picture, and of two the same size the finer
        assert_eq!(video.picture(None).map(|p| p.url.as_str()), Some("https://cdn.test/v1080.mp4"));
        assert_eq!(video.picture(Some(1280)).map(|p| p.url.as_str()), Some("https://cdn.test/v720.mp4"));
        assert_eq!(video.picture(Some(100)).map(|p| p.url.as_str()), Some("https://cdn.test/v240.mp4?a=1&b=2"));
        let fetched = |max, audio, join| video.streams(max, audio, join).iter().map(|s| (s.url.clone(), s.ext.clone(), s.size)).collect::<Vec<_>>();
        assert_eq!(
            fetched(None, false, true),
            vec![("https://cdn.test/v1080.mp4".to_string(), "mp4".to_string(), Some(15138196)), ("https://cdn.test/a.m4a".to_string(), "m4a".to_string(), Some(569324))]
        );
        // without FFmpeg nothing can be joined: the one file that holds both
        assert_eq!(fetched(None, false, false), vec![("https://cdn.test/whole.mp4".to_string(), "mp4".to_string(), None)]);
        // the sound alone
        assert_eq!(fetched(None, true, true), vec![("https://cdn.test/a.m4a".to_string(), "m4a".to_string(), Some(569324))]);
        // a video Instagram gives as one file only
        let plain = Video { whole: video.whole.clone(), ..Default::default() };
        assert_eq!(plain.streams(None, false, true).len(), 1);
        assert_eq!(plain.streams(None, true, true)[0].url, "https://cdn.test/whole.mp4");
        // a silent video: its picture is all of it
        let silent = Video { pictures: video.pictures.clone(), ..Default::default() };
        assert_eq!(silent.streams(None, false, true).iter().map(|s| s.url.as_str()).collect::<Vec<_>>(), vec!["https://cdn.test/v1080.mp4"]);
        assert!(Video::default().streams(None, false, true).is_empty());
    }

    /// Instagram's answer about a post, in the shape it has for a visitor.
    fn answer(shown: serde_json::Value) -> serde_json::Value {
        json!({ "data": { "xig_polaris_media": { "__typename": "XIGPolarisVideoMedia", "pk": "1", "code": "DgSGcWtg4Tx", "if_not_gated_logged_out": shown, "gating_ruling": null } }, "extensions": { "is_final": true } })
    }

    fn user() -> serde_json::Value {
        json!({ "username": "someone", "full_name": "Some One", "is_private": false, "pk": "787132" })
    }

    #[test]
    fn a_reel_is_read() {
        let reel = answer(json!({
            "user": user(), "code": "DgSGcWtg4Tx", "media_type": 2, "product_type": "clips",
            "like_count": 13372, "comment_count": 101, "caption": { "text": " Whooping cranes\nnearly vanished " },
            "original_height": 1920, "original_width": 1080, "display_uri": "https://cdn.test/cover.jpg",
            "image_versions2": { "candidates": [{ "url": "https://cdn.test/cover-full.jpg", "height": 1920, "width": 1080 }, { "url": "https://cdn.test/cover-small.jpg", "height": 1280, "width": 720 }] },
            "video_dash_manifest": STREAMS,
            "video_versions": [{ "type": 101, "url": "https://cdn.test/whole.mp4" }, { "type": 102, "url": "https://cdn.test/whole2.mp4" }],
        }));
        let post = read(&reel).unwrap();
        assert_eq!((post.code.as_str(), post.username.as_str(), post.kind()), ("DgSGcWtg4Tx", "someone", "Video"));
        assert_eq!(post.pieces.len(), 1);
        let video = post.pieces[0].video.as_ref().unwrap();
        assert_eq!((video.whole.as_deref(), video.pictures.len(), video.sound.is_some(), video.seconds), (Some("https://cdn.test/whole.mp4"), 3, true, Some(92)));

        let shown = post.details();
        assert_eq!(shown.title, "Video by @someone");
        assert_eq!((shown.site.as_deref(), shown.duration, shown.width, shown.height), (Some("Instagram"), Some(92), Some(1080), Some(1920)));
        assert_eq!((shown.uploader.as_deref(), shown.uploader_url.as_deref()), (Some("Some One"), Some("https://www.instagram.com/someone/")));
        assert_eq!((shown.caption.as_deref(), shown.likes, shown.comments, shown.views), (Some("Whooping cranes\nnearly vanished"), Some(13372), Some(101), None));
        assert_eq!(shown.thumbnail.as_deref(), Some("https://cdn.test/cover.jpg"));
        assert!(shown.heights.is_empty(), "one quality: the original");
    }

    #[test]
    fn photos_are_read_alone_and_several_in_one_post() {
        // one photo: its sizes come without their measures, the original first
        let photo = answer(json!({
            "user": user(), "code": "DgPIyUdDLu3", "media_type": 1, "original_height": 1350, "original_width": 1080,
            "display_uri": "https://cdn.test/shown-640.jpg",
            "image_versions2": { "candidates": [{ "url": "https://cdn.test/original.jpg" }, { "url": "https://cdn.test/s720.jpg" }, { "url": "javascript:alert(1)" }] },
        }));
        let post = read(&photo).unwrap();
        assert_eq!(post.kind(), "Photo");
        assert_eq!(post.pieces, vec![Piece { photo: Some("https://cdn.test/original.jpg".into()), video: None }]);
        assert_eq!(post.details().title, "Photo by @someone");
        assert_eq!(post.details().duration, None);

        // several in one post, a video among them; the largest of each photo's sizes
        let several = answer(json!({
            "user": user(), "code": "DgMTTbyFz-6", "media_type": 8, "display_uri": "https://cdn.test/first.jpg",
            "carousel_media": [
                { "media_type": 1, "image_versions2": { "candidates": [{ "url": "https://cdn.test/1-small.jpg", "width": 640, "height": 480 }, { "url": "https://cdn.test/1-large.jpg", "width": 1440, "height": 1080 }] } },
                { "media_type": 2, "image_versions2": { "candidates": [{ "url": "https://cdn.test/2-cover.jpg" }] }, "video_versions": [{ "url": "https://cdn.test/2.mp4" }] },
                { "media_type": 1 },
                { "media_type": 1, "display_uri": "https://cdn.test/4.webp" },
            ],
        }));
        let post = read(&several).unwrap();
        assert_eq!(post.kind(), "Post");
        assert_eq!(post.pieces.len(), 3, "what holds nothing is left out");
        assert_eq!(post.pieces[0].photo.as_deref(), Some("https://cdn.test/1-large.jpg"));
        assert_eq!(post.pieces[1].video.as_ref().and_then(|v| v.whole.as_deref()), Some("https://cdn.test/2.mp4"));
        assert_eq!(post.pieces[2].photo.as_deref(), Some("https://cdn.test/4.webp"));
        assert_eq!(post.first_video().and_then(|v| v.whole.as_deref()), Some("https://cdn.test/2.mp4"));
        // as files: each photo as it is, the video as the one file Instagram gives for it
        let files = files_of(&post, None, true);
        assert_eq!(files.len(), 3);
        assert_eq!(files[0], PostFile::whole("https://cdn.test/1-large.jpg", "jpg"));
        assert_eq!(files[1], PostFile::whole("https://cdn.test/2.mp4", "mp4"));
        assert_eq!(files[2], PostFile::whole("https://cdn.test/4.webp", "webp"));
    }

    #[test]
    fn what_instagram_keeps_from_a_visitor_is_told_as_such() {
        // no such post (and a private account's, which Instagram answers the same way)
        assert_eq!(read(&json!({ "data": { "xig_polaris_media": null }, "extensions": { "is_final": true } })), Err(UNAVAILABLE));
        assert_eq!(read(&json!({})), Err(UNAVAILABLE));
        // there, and shown to signed-in viewers only
        let kept = |ruling: serde_json::Value| json!({ "data": { "xig_polaris_media": { "code": "X", "if_not_gated_logged_out": null, "gating_ruling": ruling } } });
        assert_eq!(read(&kept(json!(null))), Err(NEEDS_SIGN_IN));
        assert_eq!(read(&kept(json!({ "title": "Restricted Video", "description": "You must be 18 years old or over to see this." }))), Err(AGE_RESTRICTED));
        assert_eq!(read(&kept(json!({ "title": "Sensitive content" }))), Err(NEEDS_SIGN_IN));
        // shown, with nothing in it
        assert_eq!(read(&answer(json!({ "user": user(), "media_type": 1 }))), Err(UNAVAILABLE));
        assert_eq!(read(&answer(json!({ "user": { "username": "someone", "is_private": true }, "media_type": 1 }))), Err(PRIVATE));
        // a name that is not an Instagram name is not passed on
        let odd = read(&answer(json!({ "user": { "username": "../x" }, "display_uri": "https://cdn.test/a.jpg" }))).unwrap();
        assert_eq!((odd.username.as_str(), odd.by(), odd.details().title.as_str(), odd.details().uploader_url), ("", "Instagram", "Instagram photo", None));
    }
}
