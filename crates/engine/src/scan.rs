//! Deep scan: reads a web page (and the frames embedded in it) and lists
//! every media link found in the markup, scripts and embedded data.
//!
//! This only reads what the page serves to any visitor. It does not solve
//! captchas, follow link shorteners or get around access protection.

use std::collections::HashSet;
use std::sync::OnceLock;
use std::time::Duration;

use percent_encoding::percent_decode_str;
use regex::Regex;
use reqwest::header::{ACCEPT, CONTENT_TYPE, REFERER};
use reqwest::{Client, Url};
use serde::Serialize;
use tokio::task::JoinSet;

use crate::probe;

const MAX_PAGE_BYTES: usize = 4 << 20;
const MAX_FRAMES: usize = 6;
const MAX_RESULTS: usize = 80;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FoundKind {
    Video,
    /// HLS or DASH playlist; downloaded through the site backend.
    Stream,
    Audio,
    /// An embedded player from another site; may or may not be supported.
    Embed,
}

#[derive(Clone, Debug, Serialize)]
pub struct Found {
    pub url: String,
    pub kind: FoundKind,
    pub name: String,
    pub size: Option<u64>,
    /// Page the link was found on; sent as `Referer` when downloading.
    pub referer: String,
}

fn classify(url: &Url) -> Option<FoundKind> {
    let last = url.path_segments()?.next_back()?;
    let ext = last.rsplit_once('.')?.1.to_ascii_lowercase();
    Some(match ext.as_str() {
        "mp4" | "mkv" | "webm" | "mov" | "m4v" | "avi" | "flv" | "wmv" | "mpg" | "mpeg" | "3gp" => {
            FoundKind::Video
        }
        "m3u8" | "mpd" => FoundKind::Stream,
        "mp3" | "m4a" | "aac" | "flac" | "ogg" | "opus" | "wav" | "wma" => FoundKind::Audio,
        _ => return None,
    })
}

fn absolute_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"(?i)https?://[^\s"'<>\\`]+"#).unwrap())
}

fn attr_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r#"(?i)(?:src|href|content|file|url|source|data-[a-z-]+)["']?\s*[=:]\s*["']([^"'<>\s]+)["']"#,
        )
        .unwrap()
    })
}

fn iframe_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"(?is)<(?:iframe|embed)\b[^>]*?\bsrc\s*=\s*["']([^"']+)["']"#).unwrap()
    })
}

/// Pulls media links and frame addresses out of one document.
fn extract(html: &str, base: &Url, media: &mut Vec<(Url, FoundKind)>, frames: &mut Vec<Url>) {
    // Links inside scripts and JSON are usually escaped.
    let text = html
        .replace("\\/", "/")
        .replace("\\u002F", "/")
        .replace("\\u0026", "&")
        .replace("&amp;", "&");

    let mut consider = |raw: &str| {
        let raw = raw.trim_end_matches([',', '.', ';', ')', ']', '}']);
        if let Ok(url) = base.join(raw) {
            if matches!(url.scheme(), "http" | "https") {
                if let Some(kind) = classify(&url) {
                    media.push((url, kind));
                }
            }
        }
    };
    for m in absolute_re().find_iter(&text) {
        consider(m.as_str());
    }
    for c in attr_re().captures_iter(&text) {
        consider(&c[1]);
    }
    for c in iframe_re().captures_iter(&text) {
        if let Ok(url) = base.join(c[1].trim()) {
            if matches!(url.scheme(), "http" | "https") && classify(&url).is_none() {
                frames.push(url);
            }
        }
    }
}

enum Page {
    Html(String),
    /// The address is itself a media file, not a page.
    Media,
}

async fn fetch(client: &Client, url: &Url, referer: Option<&Url>) -> Result<Page, String> {
    let mut req = client
        .get(url.clone())
        .header(ACCEPT, "text/html,application/xhtml+xml,*/*;q=0.8");
    if let Some(r) = referer {
        req = req.header(REFERER, r.as_str());
    }
    let mut resp = req
        .send()
        .await
        .map_err(|e| format!("Could not open the page: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!(
            "The site refused the request (HTTP {})",
            resp.status().as_u16()
        ));
    }
    let content_type = resp
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    if content_type.starts_with("video/") || content_type.starts_with("audio/") {
        return Ok(Page::Media);
    }
    let mut body = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|e| format!("Could not read the page: {e}"))? {
        body.extend_from_slice(&chunk);
        if body.len() >= MAX_PAGE_BYTES {
            break;
        }
    }
    Ok(Page::Html(String::from_utf8_lossy(&body).into_owned()))
}

fn display_name(url: &Url, kind: FoundKind) -> String {
    if kind == FoundKind::Embed {
        return format!("Embedded player ({})", url.host_str().unwrap_or("unknown site"));
    }
    url.path_segments()
        .and_then(|mut s| s.next_back())
        .map(|s| percent_decode_str(s).decode_utf8_lossy().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| url.to_string())
}

pub(crate) async fn scan(client: &Client, page: &str) -> Result<Vec<Found>, String> {
    let base = Url::parse(page.trim()).map_err(|_| "That is not a valid URL".to_string())?;
    if !matches!(base.scheme(), "http" | "https") {
        return Err("Only http and https pages can be searched".into());
    }

    // (link, kind, page it was found on)
    let mut items: Vec<(Url, FoundKind, Url)> = Vec::new();
    let html = match fetch(client, &base, None).await? {
        Page::Html(html) => html,
        Page::Media => {
            let kind = classify(&base).unwrap_or(FoundKind::Video);
            return Ok(vec![Found {
                name: display_name(&base, kind),
                url: base.to_string(),
                kind,
                size: None,
                referer: base.to_string(),
            }]);
        }
    };

    let mut media = Vec::new();
    let mut frames = Vec::new();
    extract(&html, &base, &mut media, &mut frames);
    items.extend(media.drain(..).map(|(u, k)| (u, k, base.clone())));

    let mut seen_frames = HashSet::new();
    frames.retain(|f| seen_frames.insert(f.to_string()));
    for frame in frames.into_iter().take(MAX_FRAMES) {
        items.push((frame.clone(), FoundKind::Embed, base.clone()));
        let fetched = tokio::time::timeout(
            Duration::from_secs(12),
            fetch(client, &frame, Some(&base)),
        )
        .await;
        if let Ok(Ok(Page::Html(inner))) = fetched {
            let mut nested = Vec::new();
            extract(&inner, &frame, &mut media, &mut nested);
            items.extend(media.drain(..).map(|(u, k)| (u, k, frame.clone())));
        }
    }

    let mut seen = HashSet::new();
    items.retain(|(u, _, _)| seen.insert(u.to_string()));
    items.sort_by_key(|(_, kind, _)| *kind);
    items.truncate(MAX_RESULTS);

    // Ask the servers how large the direct files are.
    let mut sizes: Vec<Option<u64>> = vec![None; items.len()];
    let mut set = JoinSet::new();
    for (i, (url, kind, referer)) in items.iter().enumerate() {
        if !matches!(kind, FoundKind::Video | FoundKind::Audio) {
            continue;
        }
        let client = client.clone();
        let url = url.to_string();
        let headers = vec![("Referer".to_string(), referer.to_string())];
        set.spawn(async move {
            let probed = tokio::time::timeout(
                Duration::from_secs(8),
                probe::probe(&client, &url, &headers),
            )
            .await;
            (i, probed.ok().and_then(|p| p.ok()).and_then(|p| p.total))
        });
    }
    while let Some(Ok((i, size))) = set.join_next().await {
        sizes[i] = size;
    }

    Ok(items
        .into_iter()
        .zip(sizes)
        .map(|((url, kind, referer), size)| Found {
            name: display_name(&url, kind),
            url: url.to_string(),
            kind,
            size,
            referer: referer.to_string(),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_links_from_markup_scripts_and_frames() {
        let base = Url::parse("https://example.test/watch/page.html").unwrap();
        let html = r#"
            <video src="/media/clip.mp4"></video>
            <a href="files/Movie%20One.mkv">get</a>
            <source data-src='https://cdn.example.test/a/song.mp3?token=1&amp;x=2'>
            <script>var p = {"file":"https:\/\/cdn.example.test\/hls\/master.m3u8","poster":"x.jpg"};</script>
            <iframe width="1" src="https://player.example.test/embed/42"></iframe>
            <a href="/about.html">about</a>
        "#;
        let mut media = Vec::new();
        let mut frames = Vec::new();
        extract(html, &base, &mut media, &mut frames);
        let mut urls: Vec<String> = media.iter().map(|(u, _)| u.to_string()).collect();
        urls.sort();
        urls.dedup();
        assert_eq!(
            urls,
            [
                "https://cdn.example.test/a/song.mp3?token=1&x=2",
                "https://cdn.example.test/hls/master.m3u8",
                "https://example.test/media/clip.mp4",
                "https://example.test/watch/files/Movie%20One.mkv",
            ]
        );
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].as_str(), "https://player.example.test/embed/42");
        let mkv = Url::parse(&urls[3]).unwrap();
        assert_eq!(display_name(&mkv, FoundKind::Video), "Movie One.mkv");
    }
}
