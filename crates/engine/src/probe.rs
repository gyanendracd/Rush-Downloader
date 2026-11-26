use std::path::Path;

use percent_encoding::percent_decode_str;
use reqwest::header::{HeaderMap, CONTENT_DISPOSITION, CONTENT_RANGE, ETAG, LAST_MODIFIED, RANGE};
use reqwest::{Client, RequestBuilder, Response, StatusCode};

use crate::task::Fail;

/// What the server told us about a URL before the real download starts.
pub(crate) struct Probe {
    pub total: Option<u64>,
    /// Server honours `Range` requests and the size is known.
    pub resumable: bool,
    /// ETag or Last-Modified, used to detect a changed file on resume.
    pub etag: Option<String>,
    pub filename: Option<String>,
    /// The URL is a web page rather than a file.
    pub is_html: bool,
    /// When the server ignored the range and is already sending the whole
    /// file, the response is kept so it can be used as the download stream.
    pub body: Option<Response>,
}

pub(crate) fn request(client: &Client, url: &str, headers: &[(String, String)]) -> RequestBuilder {
    let mut rb = client.get(url);
    for (k, v) in headers {
        rb = rb.header(k.as_str(), v.as_str());
    }
    rb
}

pub(crate) fn net_err(e: reqwest::Error) -> Fail {
    let mut msg = e.to_string();
    let mut src = std::error::Error::source(&e);
    while let Some(s) = src {
        msg = format!("{msg}: {s}");
        src = s.source();
    }
    if e.is_builder() {
        Fail::Fatal(msg)
    } else {
        Fail::Retry(msg)
    }
}

pub(crate) async fn probe(
    client: &Client,
    url: &str,
    headers: &[(String, String)],
) -> Result<Probe, Fail> {
    let resp = request(client, url, headers)
        .header(RANGE, "bytes=0-0")
        .send()
        .await
        .map_err(net_err)?;
    let status = resp.status();
    let h = resp.headers();
    let etag = header_str(h, ETAG.as_str()).or_else(|| header_str(h, LAST_MODIFIED.as_str()));
    let filename = cd_filename(h).or_else(|| url_filename(resp.url().as_str()));
    let is_html = cd_filename(h).is_none()
        && header_str(h, reqwest::header::CONTENT_TYPE.as_str())
            .is_some_and(|t| {
                // Pages and streaming playlists go to the site backend.
                let t = t.to_ascii_lowercase();
                t.starts_with("text/html") || t.contains("mpegurl") || t.contains("dash+xml")
            });

    if status == StatusCode::PARTIAL_CONTENT {
        let total = header_str(h, CONTENT_RANGE.as_str())
            .and_then(|v| v.rsplit('/').next().and_then(|t| t.trim().parse::<u64>().ok()));
        Ok(Probe {
            total,
            resumable: matches!(total, Some(t) if t > 0),
            etag,
            filename,
            is_html,
            body: None,
        })
    } else if status.is_success() {
        Ok(Probe {
            total: resp.content_length(),
            resumable: false,
            etag,
            filename,
            is_html,
            body: Some(resp),
        })
    } else if status == StatusCode::RANGE_NOT_SATISFIABLE {
        // Typically an empty file; fall back to a plain request.
        Ok(Probe {
            total: None,
            resumable: false,
            etag,
            filename,
            is_html,
            body: None,
        })
    } else {
        Err(status_fail(status))
    }
}

pub(crate) fn status_fail(status: StatusCode) -> Fail {
    let msg = format!("HTTP {status}");
    if status.is_server_error()
        || status == StatusCode::TOO_MANY_REQUESTS
        || status == StatusCode::REQUEST_TIMEOUT
    {
        Fail::Retry(msg)
    } else {
        Fail::Fatal(msg)
    }
}

fn header_str(h: &HeaderMap, name: &str) -> Option<String> {
    h.get(name)
        .map(|v| String::from_utf8_lossy(v.as_bytes()).trim().to_string())
        .filter(|s| !s.is_empty())
}

/// File name from a `Content-Disposition` header (RFC 6266).
fn cd_filename(h: &HeaderMap) -> Option<String> {
    let cd = header_str(h, CONTENT_DISPOSITION.as_str())?;
    let mut plain = None;
    for part in cd.split(';') {
        let part = part.trim();
        let lower = part.to_ascii_lowercase();
        if lower.starts_with("filename*=") {
            let v = part[10..].trim().trim_matches('"');
            // charset'lang'percent-encoded
            let enc = v.splitn(3, '\'').nth(2).unwrap_or(v);
            let name = percent_decode_str(enc).decode_utf8_lossy().to_string();
            if !name.is_empty() {
                return Some(name);
            }
        } else if lower.starts_with("filename=") {
            let v = part[9..].trim().trim_matches('"');
            if !v.is_empty() {
                plain = Some(percent_decode_str(v).decode_utf8_lossy().to_string());
            }
        }
    }
    plain
}

pub(crate) fn url_filename(url: &str) -> Option<String> {
    let url = reqwest::Url::parse(url).ok()?;
    let seg = url.path_segments()?.rev().find(|s| !s.is_empty())?;
    let name = percent_decode_str(seg).decode_utf8_lossy().to_string();
    (!name.is_empty()).then_some(name)
}

/// Makes a name safe to use as a file name on every supported platform.
pub(crate) fn sanitize(name: &str) -> String {
    let mut out: String = name
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    out = out.trim().trim_end_matches(['.', ' ']).to_string();
    if out.chars().count() > 180 {
        let (stem, ext) = split_ext(&out);
        let ext: String = ext.chars().take(20).collect();
        let stem: String = stem.chars().take(180 - ext.chars().count()).collect();
        out = format!("{stem}{ext}");
    }
    let stem_upper = split_ext(&out).0.to_ascii_uppercase();
    const RESERVED: [&str; 22] = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
        "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    if RESERVED.contains(&stem_upper.as_str()) {
        out = format!("_{out}");
    }
    if out.is_empty() {
        out = "download".into();
    }
    out
}

/// Splits `name` into (stem, ".ext"); the extension is empty when absent.
fn split_ext(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(i) if i > 0 && i + 1 < name.len() => name.split_at(i),
        _ => (name, ""),
    }
}

/// `name`, `name (1)`, `name (2)`, ... for collision avoidance.
pub(crate) fn candidate(name: &str, n: u32) -> String {
    if n == 0 {
        return name.to_string();
    }
    let (stem, ext) = split_ext(name);
    format!("{stem} ({n}){ext}")
}

pub(crate) fn part_path(dir: &Path, filename: &str) -> std::path::PathBuf {
    dir.join(format!("{filename}.part"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes() {
        assert_eq!(sanitize("a/b:c?.zip"), "a_b_c_.zip");
        assert_eq!(sanitize("con.txt"), "_con.txt");
        assert_eq!(sanitize("  "), "download");
        assert_eq!(candidate("a.tar.gz", 2), "a.tar (2).gz");
        assert_eq!(candidate("noext", 1), "noext (1)");
    }

    #[test]
    fn names_from_url() {
        assert_eq!(
            url_filename("https://x.test/a/My%20File.zip?x=1").as_deref(),
            Some("My File.zip")
        );
        assert_eq!(url_filename("https://x.test/"), None);
    }
}
