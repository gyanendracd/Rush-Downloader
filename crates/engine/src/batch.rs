//! A batch: many links downloaded together under one name, into a folder
//! of that name. The batch is one record in the list; each of its links is
//! an ordinary download (direct file, site video, playlist...) whose record
//! names the batch as its `parent`. Those downloads go through the usual
//! queue, engine, pause and resume and are saved with the rest; the batch's
//! own status and progress are worked out from theirs.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::{Entry, MediaOptions, Status};

/// What a batch was made from, kept with its record.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct BatchMeta {
    /// The links, in the order they were found.
    pub urls: Vec<String>,
    /// The quality asked for its videos.
    #[serde(default)]
    pub media: MediaOptions,
}

/// How a batch's downloads stand, for display.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct BatchInfo {
    /// Downloads in the batch.
    pub count: u64,
    pub completed: u64,
    /// Connecting or downloading now.
    pub downloading: u64,
    /// Waiting in the queue.
    pub queued: u64,
    pub paused: u64,
    /// Failed, cancelled ones not counted.
    pub failed: u64,
    pub cancelled: u64,
    /// How far the batch is as a whole, 0 to 1: each download counts the
    /// same, a finished one fully and a running one by its own progress.
    /// Failed and cancelled ones are left out.
    pub progress: f64,
    /// A download running now, by its file name.
    pub current: Option<String>,
}

/// What a batch's downloads add up to.
#[derive(Clone, Debug, Default)]
pub(crate) struct Rollup {
    pub info: BatchInfo,
    /// Bytes in, of the downloads not failed or cancelled.
    pub downloaded: u64,
    /// Their sizes together, while every one of them knows its own.
    pub total: Option<u64>,
    pub speed: f64,
    pub conns: usize,
}

/// The status a batch shows for how its downloads stand, and the error it
/// carries: it runs while any of them does, and has ended once all have.
pub(crate) fn status_of(info: &BatchInfo) -> (Status, Option<String>) {
    let ended_badly = info.failed + info.cancelled;
    if info.downloading > 0 {
        (Status::Downloading, None)
    } else if info.queued > 0 {
        (Status::Queued, None)
    } else if info.paused > 0 {
        (Status::Paused, None)
    } else if ended_badly == 0 {
        (Status::Completed, None)
    } else if info.completed > 0 {
        // Completed with errors: what did come is there.
        (Status::Completed, Some(format!("{ended_badly} of {} links did not download", info.count)))
    } else if info.failed == 0 {
        (Status::Failed, Some("Cancelled".into()))
    } else {
        (Status::Failed, Some(format!("None of the {} links could be downloaded", info.count)))
    }
}

/// Works out every batch's status and progress from its downloads, writes
/// it into the batch's record, and returns what each batch adds up to.
pub(crate) fn roll_up(entries: &mut [Entry], now: u64) -> HashMap<u64, Rollup> {
    let mut sums: HashMap<u64, Rollup> = HashMap::new();
    let mut parts: HashMap<u64, f64> = HashMap::new();
    let mut sized: HashMap<u64, bool> = HashMap::new();
    for e in entries.iter() {
        let Some(parent) = e.rec.parent else { continue };
        let r = &e.rec;
        let sum = sums.entry(parent).or_default();
        let i = &mut sum.info;
        i.count += 1;
        let gone = r.cancelled || r.status == Status::Failed;
        match r.status {
            _ if r.cancelled => i.cancelled += 1,
            Status::Completed => i.completed += 1,
            Status::Connecting | Status::Downloading => {
                i.downloading += 1;
                if i.current.is_none() {
                    i.current = Some(r.filename.clone());
                }
            }
            Status::Queued => i.queued += 1,
            Status::Paused => i.paused += 1,
            Status::Failed => i.failed += 1,
        }
        if gone {
            continue;
        }
        let total = r.total.filter(|t| *t > 0);
        let downloaded = if r.status == Status::Completed { total.unwrap_or(r.downloaded) } else { r.downloaded };
        sum.downloaded += downloaded;
        let known = sized.entry(parent).or_insert(true);
        match total {
            Some(t) => sum.total = Some(sum.total.unwrap_or(0) + t.max(downloaded)),
            None if r.status == Status::Completed => sum.total = Some(sum.total.unwrap_or(0) + downloaded),
            None => *known = false,
        }
        *parts.entry(parent).or_insert(0.0) += if r.status == Status::Completed {
            1.0
        } else {
            total.map_or(0.0, |t| (downloaded as f64 / t as f64).min(1.0))
        };
        sum.speed += e.speed;
        sum.conns += e.live.as_ref().map_or(0, |l| l.conns.load(std::sync::atomic::Ordering::Relaxed));
    }
    for (parent, sum) in sums.iter_mut() {
        if !sized.get(parent).copied().unwrap_or(true) {
            sum.total = None;
        }
        let counted = sum.info.count - sum.info.failed - sum.info.cancelled;
        sum.info.progress = if counted == 0 { 0.0 } else { parts.get(parent).copied().unwrap_or(0.0) / counted as f64 };
    }
    for e in entries.iter_mut() {
        if e.rec.batch.is_none() {
            continue;
        }
        let sum = sums.entry(e.rec.id).or_default();
        let (status, error) = status_of(&sum.info);
        let r = &mut e.rec;
        r.status = status;
        r.error = error;
        r.downloaded = sum.downloaded;
        r.total = sum.total;
        r.items = sum.info.completed;
        r.part_started = true;
        let ended = matches!(status, Status::Completed | Status::Failed);
        if ended && r.finished_at.is_none() {
            r.finished_at = Some(now);
        } else if !ended {
            r.finished_at = None;
        }
    }
    sums
}

/// The links of a pasted text or file list, each once, in order: blanks
/// and anything that is not a web link left out.
pub(crate) fn clean_links(urls: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    urls.into_iter()
        .map(|u| u.trim().to_string())
        .filter(|u| {
            let lower = u.to_ascii_lowercase();
            (lower.starts_with("http://") || lower.starts_with("https://")) && reqwest::Url::parse(u).is_ok()
        })
        .filter(|u| seen.insert(u.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(completed: u64, downloading: u64, queued: u64, paused: u64, failed: u64, cancelled: u64) -> BatchInfo {
        BatchInfo {
            count: completed + downloading + queued + paused + failed + cancelled,
            completed,
            downloading,
            queued,
            paused,
            failed,
            cancelled,
            ..Default::default()
        }
    }

    #[test]
    fn a_batch_runs_while_any_download_does_and_ends_once_all_have() {
        assert_eq!(status_of(&info(5, 2, 5, 0, 0, 0)).0, Status::Downloading);
        assert_eq!(status_of(&info(5, 0, 5, 1, 1, 0)).0, Status::Queued);
        assert_eq!(status_of(&info(5, 0, 0, 2, 1, 0)).0, Status::Paused);
        assert_eq!(status_of(&info(12, 0, 0, 0, 0, 0)), (Status::Completed, None));
        // completed with errors: one failure does not fail the batch
        let (status, error) = status_of(&info(10, 0, 0, 0, 1, 1));
        assert_eq!(status, Status::Completed);
        assert_eq!(error.as_deref(), Some("2 of 12 links did not download"));
        assert_eq!(status_of(&info(0, 0, 0, 0, 3, 0)).0, Status::Failed);
        assert_eq!(status_of(&info(0, 0, 0, 0, 0, 3)), (Status::Failed, Some("Cancelled".into())));
    }

    #[test]
    fn links_are_kept_once_in_order() {
        let links = clean_links(vec![
            " https://youtu.be/a?si=1 ".into(),
            "ftp://example.com/x".into(),
            "".into(),
            "https://example.com/b.zip".into(),
            "https://youtu.be/a?si=1".into(),
            "not a link".into(),
        ]);
        assert_eq!(links, vec!["https://youtu.be/a?si=1", "https://example.com/b.zip"]);
    }
}
