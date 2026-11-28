//! Posts on X (Twitter), from their link alone.
//!
//! yt-dlp reads a post first, as it does for any site, with no sign-in for
//! a public post. When it cannot (it has nothing to give for a post of
//! photos, and its reader breaks whenever X changes), the post is asked for
//! the way any website that shows a post of X's gets it: from X's own
//! service for embedded posts. That takes no account, no sign-in and no
//! cookie, and answers with the post's photos and videos.
//!
//! What X keeps for signed-in viewers stays there: such a post comes back
//! as a notice in place of the post, and is reported so.
//!
//! A profile's list of posts is another matter. X gives it to signed-in
//! visitors only (its service for embedded profiles answers "rate limit
//! exceeded" to everyone else), so downloading a profile needs the user's
//! own sign-in (profile.rs).

use std::sync::Arc;

use crate::media::{find_tool, MediaInfo, Plan, Stream};
use crate::postfiles::{picture_ext, PostFile};
use crate::profile::BROWSER_AGENT;
use crate::task::{Fail, Live};
use crate::{Inner, Record, Status};

/// What the user is told, in each case.
pub(crate) const NOT_FOUND: &str = "This post on X is not available. It may have been deleted, or its account may be private or suspended.";
pub(crate) const NEEDS_SIGN_IN: &str = "X shows this post only to signed-in users. Sign in to X on the Sign in page, then retry.";
pub(crate) const NOTHING: &str = "This post on X has no photo or video to download.";
pub(crate) const NO_VIDEO: &str = "This post on X has no video, so there is no sound to save.";

const SITE: &str = "https://x.com/";
/// X's service for posts shown on other websites.
const EMBEDDED: &str = "https://cdn.syndication.twimg.com/tweet-result";

fn host_is_x(url: &reqwest::Url) -> bool {
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    ["x.com", "twitter.com"].iter().any(|site| host == *site || host.ends_with(&format!(".{site}")))
}

/// The number of the post a link points at ("/someone/status/123",
/// "/i/status/123", with or without "/photo/1" after it), or `None` for any
/// other link (a profile, the home page, another site).
pub(crate) fn post_of(url: &str) -> Option<String> {
    let parsed = reqwest::Url::parse(url.trim()).ok()?;
    if !host_is_x(&parsed) || !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    let parts: Vec<&str> = parsed.path().split('/').filter(|p| !p.is_empty()).collect();
    let at = parts.iter().position(|p| matches!(*p, "status" | "statuses"))?;
    if !(1..=2).contains(&at) {
        return None;
    }
    let id = parts.get(at + 1)?;
    ((1..=20).contains(&id.len()) && id.chars().all(|c| c.is_ascii_digit())).then(|| id.to_string())
}

/// Whether a link is one post on X (not a profile).
pub(crate) fn is_post(url: &str) -> bool {
    post_of(url).is_some()
}

/// Why a post could not be read from X's service for embedded posts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Refused {
    /// X answered, and this is what the answer means.
    Told(&'static str),
    /// X could not be asked, or gave no answer that can be read.
    Unreachable(String),
}

/* ---------- what a post holds ---------- */

/// One size of a video, picture and sound together in one file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Size {
    pub url: String,
    pub width: u32,
    pub height: u32,
    /// Bits a second.
    pub rate: u64,
}

/// One photo or video of a post.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Piece {
    /// A photo, in the size it was posted in.
    Photo { url: String, width: u32, height: u32 },
    /// A video (or a moving picture, which X keeps as a video): its cover,
    /// its sizes and its length in seconds.
    Video { cover: Option<String>, sizes: Vec<Size>, seconds: Option<u64>, width: u32, height: u32 },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Post {
    pub id: String,
    /// "someone", as in "@someone".
    pub username: String,
    pub name: Option<String>,
    pub text: Option<String>,
    pub likes: Option<u64>,
    pub replies: Option<u64>,
    pub pieces: Vec<Piece>,
}

fn https(value: Option<&serde_json::Value>) -> Option<String> {
    value?.as_str().filter(|a| a.starts_with("https://")).map(str::to_string)
}

/// A photo's address in the size it was posted in: X serves a smaller one
/// under the plain address.
pub(crate) fn original(photo: &str) -> String {
    let plain = photo.split(['?', '#']).next().unwrap_or(photo);
    match plain.rsplit_once('.') {
        Some((base, ext)) if plain.starts_with("https://pbs.twimg.com/media/") && !ext.contains('/') => format!("{base}?format={ext}&name=orig"),
        _ => photo.to_string(),
    }
}

/// The width and height in a video file's address: ".../vid/1280x720/x.mp4".
fn measures_in(address: &str) -> Option<(u32, u32)> {
    address.split('/').find_map(|part| {
        let (width, height) = part.split_once('x')?;
        Some((width.parse().ok()?, height.parse().ok()?))
    })
}

fn piece_in(media: &serde_json::Value) -> Option<Piece> {
    let side = |key: &str| media.get("original_info").and_then(|o| o.get(key)).and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let picture = https(media.get("media_url_https"));
    match media.get("type").and_then(|t| t.as_str()) {
        Some("photo") => Some(Piece::Photo { url: original(&picture?), width: side("width"), height: side("height") }),
        Some("video" | "animated_gif") => {
            let details = media.get("video_info");
            let mut sizes: Vec<Size> = details
                .and_then(|d| d.get("variants"))
                .and_then(|v| v.as_array())
                .into_iter()
                .flatten()
                // whole files only: the list in small pieces (HLS) is left to yt-dlp
                .filter(|v| v.get("content_type").and_then(|c| c.as_str()) == Some("video/mp4"))
                .filter_map(|v| {
                    let url = https(v.get("url"))?;
                    let (width, height) = measures_in(&url).unwrap_or((0, 0));
                    Some(Size { url, width, height, rate: v.get("bitrate").and_then(|b| b.as_u64()).unwrap_or(0) })
                })
                .collect();
            sizes.sort_by_key(|s| std::cmp::Reverse((s.width as u64 * s.height as u64, s.rate)));
            let seconds = details.and_then(|d| d.get("duration_millis")).and_then(|d| d.as_u64()).map(|ms| (ms + 500) / 1000);
            let (width, height) = sizes.first().map(|s| (s.width, s.height)).filter(|m| *m != (0, 0)).unwrap_or((side("width"), side("height")));
            (!sizes.is_empty()).then_some(Piece::Video { cover: picture, sizes, seconds, width, height })
        }
        _ => None,
    }
}

/// Reads X's answer about a post.
pub(crate) fn read(answer: &serde_json::Value) -> Result<Post, &'static str> {
    match answer.get("__typename").and_then(|t| t.as_str()) {
        Some("Tweet") => {}
        // A notice in the post's place: "Age-restricted adult content. This
        // content might not be appropriate for people under 18 years old.
        // To view this media, you'll need to log in to X."
        Some("TweetTombstone") => {
            let notice = answer.get("tombstone").map(|t| t.to_string().to_ascii_lowercase()).unwrap_or_default();
            let kept = ["age-restricted", "log in", "sign in", "adult content"].iter().any(|sign| notice.contains(sign));
            return Err(if kept { NEEDS_SIGN_IN } else { NOT_FOUND });
        }
        // nothing at all: no such post
        _ => return Err(NOT_FOUND),
    }
    let text = |from: Option<&serde_json::Value>, key: &str| from?.get(key)?.as_str().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    let user = answer.get("user");
    let valid_name = |name: &String| name.len() <= 20 && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    let pieces: Vec<Piece> = answer.get("mediaDetails").and_then(|m| m.as_array()).into_iter().flatten().filter_map(piece_in).collect();
    if pieces.is_empty() {
        return Err(NOTHING);
    }
    Ok(Post {
        id: text(Some(answer), "id_str").filter(|id| id.chars().all(|c| c.is_ascii_digit())).unwrap_or_default(),
        username: text(user, "screen_name").filter(valid_name).unwrap_or_default(),
        name: text(user, "name"),
        text: text(Some(answer), "text"),
        likes: answer.get("favorite_count").and_then(|v| v.as_u64()),
        replies: answer.get("conversation_count").and_then(|v| v.as_u64()),
        pieces,
    })
}

/// The size of a video to fetch: the largest, or the largest no taller than
/// asked for (the smallest there is, when all are taller).
fn chosen(sizes: &[Size], max_height: Option<u32>) -> Option<&Size> {
    let weight = |s: &&Size| (s.width as u64 * s.height as u64, s.rate);
    match max_height {
        Some(h) => sizes.iter().filter(|s| s.height <= h).max_by_key(weight).or_else(|| sizes.iter().min_by_key(weight)),
        None => sizes.iter().max_by_key(weight),
    }
}

impl Post {
    fn by(&self) -> &str {
        if self.username.is_empty() { "X" } else { &self.username }
    }

    /// The post's words on one line, cut to a title's length.
    fn words(&self, most: usize) -> Option<String> {
        let line = self.text.as_deref()?.split_whitespace().filter(|w| !w.starts_with("https://t.co/")).collect::<Vec<_>>().join(" ");
        let cut = match line.chars().count() > most {
            true => format!("{}…", line.chars().take(most).collect::<String>().trim_end()),
            false => line,
        };
        (!cut.is_empty()).then_some(cut)
    }

    /// "Someone - what they wrote", as yt-dlp titles a post.
    fn title(&self) -> String {
        let who = self.name.clone().unwrap_or_else(|| format!("@{}", self.by()));
        match self.words(80) {
            Some(words) => format!("{who} - {words}"),
            None => format!("Post by {who}"),
        }
    }

    fn first_video(&self) -> Option<&[Size]> {
        self.pieces.iter().find_map(|p| match p {
            Piece::Video { sizes, .. } => Some(sizes.as_slice()),
            Piece::Photo { .. } => None,
        })
    }

    /// What is shown about the post before it is downloaded.
    pub(crate) fn details(&self) -> MediaInfo {
        let alone = match self.pieces.as_slice() {
            [Piece::Video { sizes, seconds, .. }] => Some((sizes, *seconds)),
            _ => None,
        };
        // A post of one video offers its sizes; anything else is saved as it is.
        let mut heights: Vec<u32> = alone.map(|(sizes, _)| sizes.iter().map(|s| s.height).filter(|h| *h > 0).collect()).unwrap_or_default();
        heights.sort_unstable_by(|a, b| b.cmp(a));
        heights.dedup();
        let (thumbnail, width, height) = match self.pieces.first() {
            Some(Piece::Photo { url, width, height }) => (Some(url.clone()), *width, *height),
            Some(Piece::Video { cover, width, height, .. }) => (cover.clone(), *width, *height),
            None => (None, 0, 0),
        };
        let named = !self.username.is_empty();
        MediaInfo {
            title: self.title(),
            thumbnail,
            duration: alone.and_then(|(_, seconds)| seconds),
            heights,
            site: Some("Twitter".into()),
            width: Some(width).filter(|w| *w > 0),
            height: Some(height).filter(|h| *h > 0),
            uploader: self.name.clone().or_else(|| named.then(|| self.username.clone())),
            uploader_url: named.then(|| format!("{SITE}{}", self.username)),
            caption: self.text.clone(),
            likes: self.likes,
            comments: self.replies,
            views: None,
        }
    }

    /// The post's photos and videos, as the files they become (postfiles.rs).
    fn files(&self, max_height: Option<u32>) -> Vec<PostFile> {
        self.pieces
            .iter()
            .filter_map(|piece| match piece {
                Piece::Photo { url, .. } => {
                    // "…/media/ABC?format=jpg&name=orig": its kind is in the address's own words
                    let ext = url.split("format=").nth(1).and_then(|f| f.split('&').next()).filter(|f| ["jpg", "png", "webp"].contains(f)).unwrap_or_else(|| picture_ext(url));
                    Some(PostFile::whole(url, ext))
                }
                Piece::Video { sizes, .. } => chosen(sizes, max_height).map(|size| PostFile::whole(&size.url, "mp4")),
            })
            .collect()
    }
}

/* ---------- asking for a post ---------- */

/// X's service wants a token with the question and takes any: this one is
/// the post's own number, spelt in letters.
fn token(id: &str) -> String {
    let mut number: u128 = id.parse().unwrap_or(0);
    let mut spelt = String::new();
    while number > 0 {
        let digit = (number % 36) as u32;
        if digit != 0 {
            spelt.insert(0, char::from_digit(digit, 36).unwrap_or('a'));
        }
        number /= 36;
    }
    if spelt.is_empty() { "a".into() } else { spelt }
}

/// A post as X gives it to any website that shows it: asked for and read.
pub(crate) async fn embedded(client: &reqwest::Client, id: &str) -> Result<Post, Refused> {
    let answer = client
        .get(EMBEDDED)
        .query(&[("id", id), ("lang", "en"), ("token", &token(id))])
        .header(reqwest::header::USER_AGENT, BROWSER_AGENT)
        .header(reqwest::header::ACCEPT, "application/json")
        .header(reqwest::header::REFERER, "https://platform.twitter.com/")
        .send()
        .await
        .map_err(|e| Refused::Unreachable(format!("X could not be reached: {e}")))?;
    match answer.status().as_u16() {
        404 => return Err(Refused::Told(NOT_FOUND)),
        429 => return Err(Refused::Unreachable("X is limiting requests right now. Wait a while and retry".into())),
        code if !answer.status().is_success() => return Err(Refused::Unreachable(format!("X answered {code} for this post"))),
        _ => {}
    }
    let text = answer.text().await.map_err(|e| Refused::Unreachable(format!("X's answer could not be read: {e}")))?;
    let read_answer: serde_json::Value = serde_json::from_str(&text).map_err(|_| Refused::Unreachable("X's answer about this post could not be read".into()))?;
    let mut post = read(&read_answer).map_err(Refused::Told)?;
    if post.id.is_empty() {
        post.id = id.to_string();
    }
    Ok(post)
}

/// RUSHDOWNLOAD_X_NO_YTDLP leaves yt-dlp out, for tests: every post is then
/// read from X's service for embedded posts.
fn ytdlp_left_out() -> bool {
    std::env::var_os("RUSHDOWNLOAD_X_NO_YTDLP").is_some()
}
const YTDLP_LEFT_OUT: &str = "yt-dlp was left out";

/// What is told when neither way gave the post: `first` is what yt-dlp
/// said, `second` what came of asking X's service for embedded posts.
fn settled(first: String, second: &Refused) -> String {
    let no_video = first.to_ascii_lowercase().contains("no video") || first == YTDLP_LEFT_OUT;
    match second {
        // yt-dlp only found no video; X's answer says what there is to say
        Refused::Told(told) if no_video || *told == NEEDS_SIGN_IN => told.to_string(),
        _ => first,
    }
}

/// A post's title, picture, length and sizes, before downloading.
pub(crate) async fn info(inner: &Arc<Inner>, url: &str) -> Result<MediaInfo, String> {
    let first = match ytdlp_left_out() {
        true => Err(YTDLP_LEFT_OUT.to_string()),
        false => crate::media::ytdlp_info(inner, url).await,
    };
    let (Err(said), Some(id)) = (&first, post_of(url)) else { return first };
    tracing::info!("X post read by yt-dlp failed: {said}");
    let asked = match inner.client() {
        Ok(client) => embedded(&client, &id).await,
        Err(e) => Err(Refused::Unreachable(e)),
    };
    match asked {
        Ok(post) => Ok(post.details()),
        Err(refused) => {
            tracing::info!("X post read from X's service for embedded posts failed: {refused:?}");
            Err(settled(said.clone(), &refused))
        }
    }
}

/// Downloads a post: through yt-dlp first; from X's service for embedded
/// posts when yt-dlp cannot, which has a post's photos too.
pub(crate) async fn run(inner: &Arc<Inner>, id: u64, live: &Arc<Live>, rec: &Record) -> Result<(), Fail> {
    let first = match ytdlp_left_out() {
        true => Err(Fail::Fatal(YTDLP_LEFT_OUT.into())),
        false => crate::media::run_ytdlp(inner, id, live, rec).await,
    };
    let (said, again) = match first {
        Err(Fail::Fatal(said)) => (said, false),
        Err(Fail::Retry(said)) => (said, true),
        other => return other,
    };
    let Some(post_id) = post_of(&rec.url) else {
        return Err(if again { Fail::Retry(said) } else { Fail::Fatal(said) });
    };
    tracing::info!(id, "X post download by yt-dlp failed: {said}");
    let client = live.client();
    let asked = tokio::select! {
        _ = live.cancel.cancelled() => return Err(Fail::Cancelled),
        p = embedded(&client, &post_id) => p,
    };
    match asked {
        Ok(post) => save(inner, id, live, rec, &post).await,
        Err(refused) => {
            tracing::info!(id, "X post read from X's service for embedded posts failed: {refused:?}");
            let told = settled(said, &refused);
            // a failure on the way to X is worth another try; X's own answer is not
            Err(if again && matches!(refused, Refused::Unreachable(_)) { Fail::Retry(told) } else { Fail::Fatal(told) })
        }
    }
}

/// Saves a post that was read with [`embedded`]: a video as one file, a
/// photo as one file, several things as a folder; or its video's sound.
async fn save(inner: &Arc<Inner>, id: u64, live: &Arc<Live>, rec: &Record, post: &Post) -> Result<(), Fail> {
    std::fs::create_dir_all(&rec.dir).map_err(|e| Fail::Fatal(format!("could not create folder: {e}")))?;
    let options = rec.media_options;
    let picture = post.details().thumbnail;
    inner.update(id, |r| {
        r.media = true;
        r.status = Status::Downloading;
        if picture.is_some() && r.thumbnail != picture {
            r.thumbnail = picture.clone();
        }
    });
    // One video, or the sound of the post's first video: fetched with many
    // connections and named as a video read by yt-dlp is (media.rs).
    let alone = match post.pieces.as_slice() {
        [Piece::Video { sizes, .. }] => Some(sizes.as_slice()),
        _ => None,
    };
    let video = match options.audio_only {
        true => Some(post.first_video().ok_or_else(|| Fail::Fatal(NO_VIDEO.into()))?),
        false => alone,
    };
    if let Some(sizes) = video {
        if options.audio_only && find_tool("ffmpeg").is_none() {
            return Err(Fail::Fatal("Saving only the sound needs FFmpeg, which was not found".into()));
        }
        let size = chosen(sizes, options.max_height).ok_or_else(|| Fail::Fatal(NOT_FOUND.into()))?;
        let stem = if rec.user_named {
            inner.claim_named_stem(id, &crate::media::named_base(rec))
        } else {
            let copy = inner.claim_media_copy(id);
            let again = if copy == 0 { String::new() } else { format!(" ({})", copy + 1) };
            let height = Some(size.height).filter(|h| *h > 0 && !options.audio_only).map(|h| format!("{h}p ")).unwrap_or_default();
            crate::probe::sanitize(&format!("{} {height}[{}]{again}", post.title(), post.id))
        };
        let stream = Stream {
            url: size.url.clone(),
            headers: vec![("User-Agent".into(), BROWSER_AGENT.into()), ("Referer".into(), SITE.into())],
            ext: "mp4".into(),
            request_cap: 0,
            size: None,
        };
        let work = crate::media::work_dir(&rec.dir, id);
        std::fs::create_dir_all(&work).map_err(|e| Fail::Fatal(format!("could not create folder: {e}")))?;
        inner.update(id, |r| r.resumable = true);
        return crate::media::fetch_direct(inner, id, live, rec, &Plan { streams: vec![stream], stem }, &work, &rec.segments).await;
    }
    let files = post.files(options.max_height);
    crate::postfiles::save(inner, id, live, rec, &format!("{} {}", post.by(), post.id), SITE, &files).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn links_to_posts_are_told_from_other_links() {
        assert_eq!(post_of("https://x.com/someone/status/2053798185756422302").as_deref(), Some("2053798185756422302"));
        assert_eq!(post_of("https://twitter.com/someone/status/123/photo/1?s=20").as_deref(), Some("123"));
        assert_eq!(post_of("https://mobile.twitter.com/someone/statuses/123").as_deref(), Some("123"));
        assert_eq!(post_of("https://x.com/i/status/20").as_deref(), Some("20"));
        assert_eq!(post_of("https://www.x.com/i/web/status/20?lang=en").as_deref(), Some("20"));
        for not_a_post in [
            "https://x.com/someone",
            "https://x.com/someone?lang=en",
            "https://x.com/someone/media",
            "https://x.com/home",
            "https://x.com/status/123",
            "https://x.com/someone/status/",
            "https://x.com/someone/status/12a",
            "https://x.com/a/b/c/status/123",
            "https://notx.com/someone/status/123",
            "https://example.com/someone/status/123",
            "ftp://x.com/someone/status/123",
            "not a link",
        ] {
            assert!(!is_post(not_a_post), "{not_a_post}");
        }
    }

    /// X's answer about a post, in the shape its service for embedded posts gives.
    fn answer(media: serde_json::Value) -> serde_json::Value {
        json!({
            "__typename": "Tweet", "id_str": "1662069125257269252", "lang": "en", "favorite_count": 125402, "conversation_count": 11901,
            "created_at": "2023-05-26T12:12:13.000Z", "text": "The new building will make\nevery one proud.  https://t.co/abc123",
            "user": { "id_str": "1", "name": "Some One", "screen_name": "some_one", "is_blue_verified": true },
            "mediaDetails": media,
        })
    }

    fn video() -> serde_json::Value {
        json!({
            "type": "video", "media_url_https": "https://pbs.twimg.com/amplify_video_thumb/1/img/cover.jpg", "original_info": { "width": 1920, "height": 1080 },
            "video_info": { "aspect_ratio": [16, 9], "duration_millis": 107600, "variants": [
                { "content_type": "application/x-mpegURL", "url": "https://video.twimg.com/amplify_video/1/pl/list.m3u8?tag=16" },
                { "content_type": "video/mp4", "bitrate": 288000, "url": "https://video.twimg.com/amplify_video/1/vid/480x270/a.mp4?tag=16" },
                { "content_type": "video/mp4", "bitrate": 10368000, "url": "https://video.twimg.com/amplify_video/1/vid/avc1/1920x1080/c.mp4?tag=16" },
                { "content_type": "video/mp4", "bitrate": 2176000, "url": "https://video.twimg.com/amplify_video/1/vid/1280x720/b.mp4?tag=16" },
                { "content_type": "video/mp4", "bitrate": 1, "url": "javascript:alert(1)" },
            ] },
        })
    }

    #[test]
    fn a_post_of_one_video_is_read_with_its_sizes() {
        let post = read(&answer(json!([video()]))).unwrap();
        assert_eq!((post.id.as_str(), post.username.as_str(), post.name.as_deref()), ("1662069125257269252", "some_one", Some("Some One")));
        let Piece::Video { cover, sizes, seconds, width, height } = &post.pieces[0] else { panic!("a video") };
        assert_eq!(cover.as_deref(), Some("https://pbs.twimg.com/amplify_video_thumb/1/img/cover.jpg"));
        assert_eq!(sizes.iter().map(|s| (s.width, s.height, s.rate)).collect::<Vec<_>>(), vec![(1920, 1080, 10368000), (1280, 720, 2176000), (480, 270, 288000)]);
        assert_eq!((*seconds, *width, *height), (Some(108), 1920, 1080));
        // the size fetched: the largest, the one asked for, or the smallest when all are taller
        let fetched = |max| chosen(sizes, max).map(|s| s.height);
        assert_eq!((fetched(None), fetched(Some(720)), fetched(Some(800)), fetched(Some(100))), (Some(1080), Some(720), Some(720), Some(270)));

        let shown = post.details();
        assert_eq!(shown.title, "Some One - The new building will make every one proud.");
        assert_eq!((shown.site.as_deref(), shown.duration, shown.width, shown.height), (Some("Twitter"), Some(108), Some(1920), Some(1080)));
        assert_eq!(shown.heights, vec![1080, 720, 270]);
        assert_eq!((shown.uploader.as_deref(), shown.uploader_url.as_deref()), (Some("Some One"), Some("https://x.com/some_one")));
        assert_eq!((shown.likes, shown.comments, shown.views), (Some(125402), Some(11901), None));
        assert_eq!(shown.thumbnail.as_deref(), Some("https://pbs.twimg.com/amplify_video_thumb/1/img/cover.jpg"));
        assert_eq!(post.files(Some(720)), vec![PostFile::whole("https://video.twimg.com/amplify_video/1/vid/1280x720/b.mp4?tag=16", "mp4")]);
    }

    #[test]
    fn photos_are_read_in_the_size_they_were_posted_in() {
        let photo = |name: &str| json!({ "type": "photo", "media_url_https": format!("https://pbs.twimg.com/media/{name}"), "original_info": { "width": 1600, "height": 796 } });
        let post = read(&answer(json!([photo("AAA.jpg"), photo("BBB.png"), video(), { "type": "photo" }, { "type": "poll" }]))).unwrap();
        assert_eq!(post.pieces.len(), 3, "what holds nothing is left out");
        assert_eq!(post.pieces[0], Piece::Photo { url: "https://pbs.twimg.com/media/AAA?format=jpg&name=orig".into(), width: 1600, height: 796 });
        assert_eq!(
            post.files(None),
            vec![
                PostFile::whole("https://pbs.twimg.com/media/AAA?format=jpg&name=orig", "jpg"),
                PostFile::whole("https://pbs.twimg.com/media/BBB?format=png&name=orig", "png"),
                PostFile::whole("https://video.twimg.com/amplify_video/1/vid/avc1/1920x1080/c.mp4?tag=16", "mp4"),
            ]
        );
        // several things in one post: saved as they are, with no choice of size
        let shown = post.details();
        assert!(shown.heights.is_empty() && shown.duration.is_none());
        assert_eq!((shown.width, shown.height), (Some(1600), Some(796)));
        assert_eq!(shown.thumbnail.as_deref(), Some("https://pbs.twimg.com/media/AAA?format=jpg&name=orig"));
        assert!(post.first_video().is_some());
        // an address that is not one of X's photos is left as it is
        assert_eq!(original("https://cdn.test/media/a.jpg"), "https://cdn.test/media/a.jpg");
        assert_eq!(original("https://pbs.twimg.com/media/AAA.jpg?name=small"), "https://pbs.twimg.com/media/AAA?format=jpg&name=orig");
        assert_eq!(original("https://pbs.twimg.com/media/AAA"), "https://pbs.twimg.com/media/AAA");
    }

    #[test]
    fn what_x_does_not_give_is_told_as_such() {
        // no such post: nothing at all
        assert_eq!(read(&json!({})), Err(NOT_FOUND));
        // a notice in the post's place
        let notice = |words: &str| json!({ "__typename": "TweetTombstone", "tombstone": { "text": { "text": words } } });
        assert_eq!(read(&notice("Age-restricted adult content. This content might not be appropriate for people under 18 years old. To view this media, you’ll need to log in to X. Learn more")), Err(NEEDS_SIGN_IN));
        assert_eq!(read(&notice("This Post is from an account that no longer exists. Learn more")), Err(NOT_FOUND));
        assert_eq!(read(&notice("You’re unable to view this Post because this account owner limits who can view their Posts. Learn more")), Err(NOT_FOUND));
        // words only
        assert_eq!(read(&json!({ "__typename": "Tweet", "id_str": "20", "text": "just setting up", "user": { "screen_name": "someone" } })), Err(NOTHING));
        // a name that is not one of X's names is not passed on
        let odd = read(&json!({ "__typename": "Tweet", "id_str": "x1", "user": { "screen_name": "../x" }, "mediaDetails": [{ "type": "photo", "media_url_https": "https://pbs.twimg.com/media/A.jpg" }] })).unwrap();
        assert_eq!((odd.id.as_str(), odd.username.as_str(), odd.by(), odd.details().uploader_url, odd.details().title.as_str()), ("", "", "X", None, "Post by @X"));
    }

    #[test]
    fn two_refusals_are_settled_into_one_message() {
        let no_video = "[twitter] 123: No video could be found in this tweet".to_string();
        let other = "The connection to this site was cut off".to_string();
        // yt-dlp only found no video: X's own answer is what there is to say
        assert_eq!(settled(no_video.clone(), &Refused::Told(NOTHING)), NOTHING);
        assert_eq!(settled(no_video.clone(), &Refused::Told(NOT_FOUND)), NOT_FOUND);
        assert_eq!(settled(YTDLP_LEFT_OUT.to_string(), &Refused::Told(NOT_FOUND)), NOT_FOUND);
        // X asks for a sign-in: said, whatever yt-dlp made of it
        assert_eq!(settled(other.clone(), &Refused::Told(NEEDS_SIGN_IN)), NEEDS_SIGN_IN);
        // otherwise yt-dlp's own words stand
        assert_eq!(settled(other.clone(), &Refused::Told(NOT_FOUND)), other);
        assert_eq!(settled(no_video.clone(), &Refused::Unreachable("no network".into())), no_video);
        // the token sent with a question: the post's number in letters, never empty
        assert_eq!(token("36"), "1");
        assert_eq!(token("71"), "1z");
        assert_eq!(token(""), "a");
        assert!(!token("2053798185756422302").is_empty() && !token("2053798185756422302").contains('0'));
    }
}
