//! A new link for a download whose link stopped working (a signed link that
//! expired, a mirror that went away), keeping what is already on disk.
//!
//! What is downloaded so far is kept only when the new link is shown to give
//! the same file: the server must accept byte ranges, report the same size
//! and, where both links say, the same ETag or Last-Modified; and a few small
//! pieces fetched from the new link, at places already on disk, must match
//! the partial file byte for byte. Otherwise the download can only start
//! over: bytes from another file are never added to the partial one.

use std::fs::File;

use reqwest::header::{CONTENT_RANGE, RANGE};
use reqwest::StatusCode;
use serde::Serialize;

use crate::probe::{self, part_path, request};
use crate::task::Fail;
use crate::{Inner, Record, Status};

/// Why what is on disk cannot be kept with the new link.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkProblem {
    /// The server does not send parts of the file, only all of it.
    NoRanges,
    /// The file there is of another size (or its size is not said).
    SizeDiffers,
    /// The file's ETag or Last-Modified is not the one the old link gave.
    VersionDiffers,
    /// Pieces of it differ from what is on disk.
    ContentDiffers,
}

/// What a new link would do.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LinkCheck {
    /// The download carries on from where it was.
    pub resume: bool,
    /// Why it cannot, when it cannot.
    pub problem: Option<LinkProblem>,
    /// Bytes on disk that are kept (when it resumes) or lost (when not).
    pub downloaded: u64,
    /// The file's size by the old link, and by the new one.
    pub old_total: Option<u64>,
    pub new_total: Option<u64>,
    /// The new link's ETag or Last-Modified, kept for the resume.
    #[serde(skip)]
    pub(crate) etag: Option<String>,
}

/// The largest piece compared at one place.
const SAMPLE: u64 = 64 * 1024;

/// Where to compare, as (start, end) byte ranges inside what is already on
/// disk: the start of the file, the end of the furthest finished stretch, and
/// one in between. Each is at most `SAMPLE` long.
pub(crate) fn sample_ranges(segments: &[[u64; 3]]) -> Vec<(u64, u64)> {
    let done: Vec<(u64, u64)> = segments.iter().filter(|s| s[1] > s[0]).map(|s| (s[0], s[1])).collect();
    let mut out: Vec<(u64, u64)> = Vec::new();
    let mut add = |a: u64, b: u64| {
        // a range inside one already taken adds nothing
        if b > a && !out.iter().any(|&(x, y)| x <= a && b <= y) {
            out.push((a, b));
        }
    };
    if let Some(&(a, b)) = done.first() {
        add(a, b.min(a + SAMPLE));
    }
    if let Some(&(a, b)) = done.iter().max_by_key(|(_, b)| *b) {
        add(b.saturating_sub(SAMPLE).max(a), b);
    }
    if let Some(&(a, b)) = done.iter().max_by_key(|(a, b)| b - a) {
        let mid = a + (b - a) / 2;
        add(mid, (mid + SAMPLE).min(b));
    }
    out
}

/// A download that takes a new link: a direct file, not a torrent, a site
/// video, a profile or a set of files picked from a page.
pub(crate) fn takes_new_link(rec: &Record) -> bool {
    !rec.torrent && !rec.media && !rec.profile && rec.bundle.is_empty() && crate::remote::proto_of(&rec.url).is_none()
}

fn web_link(url: &str) -> Result<(), String> {
    match reqwest::Url::parse(url.trim()) {
        Ok(u) if matches!(u.scheme(), "http" | "https") && u.host_str().is_some() => Ok(()),
        _ => Err("Enter a full link, starting with http:// or https://".into()),
    }
}

fn fail_text(f: Fail) -> String {
    match f {
        Fail::Fatal(m) | Fail::Retry(m) => m,
        Fail::Cancelled => "cancelled".into(),
    }
}

/// Looks at what the new link gives, against the download as it is.
pub(crate) async fn check(inner: &Inner, rec: &Record, url: &str, headers: &[(String, String)]) -> Result<LinkCheck, String> {
    web_link(url)?;
    let client = inner.client_for(&rec.proxy)?;
    let probe = probe::probe(&client, url.trim(), headers).await.map_err(fail_text)?;
    if probe.is_html {
        return Err("That link opens a web page, not the file".into());
    }
    let downloaded: u64 = if rec.segments.is_empty() {
        rec.downloaded
    } else {
        rec.segments.iter().map(|s| s[1] - s[0]).sum()
    };
    let (new_total, new_etag) = (probe.total, probe.etag.clone());
    let verdict = |problem: Option<LinkProblem>| LinkCheck {
        resume: problem.is_none(),
        problem,
        downloaded,
        old_total: rec.total,
        new_total,
        etag: new_etag.clone(),
    };
    // Nothing kept yet: the new link simply starts it.
    if !rec.part_started || downloaded == 0 || rec.segments.is_empty() {
        return Ok(LinkCheck { downloaded: 0, ..verdict(None) });
    }
    if !probe.resumable || !rec.resumable {
        return Ok(verdict(Some(LinkProblem::NoRanges)));
    }
    if rec.total.is_none() || rec.total != probe.total {
        return Ok(verdict(Some(LinkProblem::SizeDiffers)));
    }
    if let (Some(old), Some(new)) = (&rec.etag, &probe.etag) {
        // a weak ETag ("W/") names the same version as its strong form
        if old.trim_start_matches("W/") != new.trim_start_matches("W/") {
            return Ok(verdict(Some(LinkProblem::VersionDiffers)));
        }
    }
    drop(probe);

    // Pieces from the new link against the same places on disk.
    let part = part_path(&rec.dir, &rec.filename);
    for (start, end) in sample_ranges(&rec.segments) {
        let len = (end - start) as usize;
        let mut local = vec![0u8; len];
        let path = part.clone();
        let read = tokio::task::spawn_blocking(move || {
            File::open(&path).and_then(|f| crate::webseed::read_exact_at(&f, &mut local, start)).map(|_| local)
        })
        .await
        .map_err(|e| e.to_string())?;
        let Ok(local) = read else {
            // the partial file is gone or short: nothing can be kept
            return Ok(verdict(Some(LinkProblem::ContentDiffers)));
        };
        let resp = request(&client, url.trim(), headers)
            .header(RANGE, format!("bytes={start}-{}", end - 1))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if resp.status() != StatusCode::PARTIAL_CONTENT {
            return Ok(verdict(Some(LinkProblem::NoRanges)));
        }
        let sent = resp.headers().get(CONTENT_RANGE).and_then(|v| v.to_str().ok()).unwrap_or_default();
        if !sent.starts_with(&format!("bytes {start}-")) {
            return Ok(verdict(Some(LinkProblem::NoRanges)));
        }
        let remote = resp.bytes().await.map_err(|e| e.to_string())?;
        if remote.as_ref() != local.as_slice() {
            return Ok(verdict(Some(LinkProblem::ContentDiffers)));
        }
    }
    Ok(verdict(None))
}

/// The download takes the new link: it carries on from what is on disk when
/// `check` says so, or, with `restart`, starts over (the partial file goes).
/// The file keeps its name. Headers given replace the old ones.
pub(crate) fn apply(rec: &mut Record, url: &str, headers: Option<Vec<(String, String)>>, check: &LinkCheck, restart: bool) {
    rec.url = url.trim().to_string();
    if let Some(h) = headers {
        rec.headers = h;
    }
    // the name it has stays, whatever the new link calls the file
    rec.user_named = true;
    if check.resume && !restart {
        // the version marker the new link gives, so the resume is let through
        rec.etag = check.etag.clone().or(rec.etag.take());
    } else {
        if rec.part_started {
            let _ = std::fs::remove_file(part_path(&rec.dir, &rec.filename));
        }
        rec.part_started = false;
        rec.segments.clear();
        rec.downloaded = 0;
        rec.total = None;
        rec.resumable = false;
        rec.etag = None;
    }
    rec.status = Status::Queued;
    rec.error = None;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_are_taken_only_from_what_is_on_disk() {
        // three stretches: the first done to 100 KB, the second not started,
        // the third half done
        let segs = [[0, 100_000, 300_000], [300_000, 300_000, 600_000], [600_000, 700_000, 900_000]];
        let r = sample_ranges(&segs);
        assert!(r.contains(&(0, SAMPLE)), "{r:?}");
        assert!(r.contains(&(700_000 - SAMPLE, 700_000)), "{r:?}");
        for (a, b) in &r {
            assert!(b - a <= SAMPLE);
            assert!(segs.iter().any(|s| *a >= s[0] && *b <= s[1]), "{a}-{b} is not all on disk");
        }
        assert!(sample_ranges(&[[0, 0, 10]]).is_empty(), "nothing on disk, nothing to compare");
        assert_eq!(sample_ranges(&[[0, 10, 10]]), vec![(0, 10)], "a short piece once");
    }
}
