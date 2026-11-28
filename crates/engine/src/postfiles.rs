//! A post's photos and videos, saved as files: one file for one of them, a
//! folder named after the post for several. Shared by the readers that ask
//! a site for a post themselves (instagram_web.rs, x.rs).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering::Relaxed;
use std::sync::Arc;

use crate::instagram::place;
use crate::media::find_tool;
use crate::profile::BROWSER_AGENT;
use crate::task::{Fail, Live};
use crate::{Inner, Record};

/// One photo or video of a post, as the file it becomes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PostFile {
    /// What to fetch, as (address, extension): the file itself, or a video's
    /// picture and sound apart, which are then joined.
    pub parts: Vec<(String, String)>,
    /// The finished file's extension.
    pub ext: String,
}

impl PostFile {
    /// A file that is fetched as it is.
    pub(crate) fn whole(address: &str, ext: &str) -> Self {
        PostFile { parts: vec![(address.to_string(), ext.to_string())], ext: ext.to_string() }
    }
}

/// What kind of picture an address points at, by its ending.
pub(crate) fn picture_ext(address: &str) -> &'static str {
    let path = address.split(['?', '#']).next().unwrap_or_default().to_ascii_lowercase();
    ["jpg", "jpeg", "png", "webp", "heic"].into_iter().find(|ext| path.ends_with(&format!(".{ext}"))).unwrap_or("jpg")
}

/// Fetches one file whole into `dest`, counting it into the download.
pub(crate) async fn fetch(live: &Arc<Live>, address: &str, referer: &str, dest: &Path) -> Result<u64, Fail> {
    let ask = live.client().get(address).header(reqwest::header::USER_AGENT, BROWSER_AGENT).header(reqwest::header::REFERER, referer);
    let mut answer = tokio::select! {
        _ = live.cancel.cancelled() => return Err(Fail::Cancelled),
        a = ask.send() => a.map_err(crate::probe::net_err)?,
    };
    if !answer.status().is_success() {
        // A site's links to its files stop working after a while; the next
        // try asks for the post, and its links, again.
        return Err(Fail::Retry(format!("the site's file server answered {}", answer.status().as_u16())));
    }
    let mut file = std::fs::File::create(dest).map_err(|e| Fail::Fatal(format!("could not create the file: {e}")))?;
    let mut size = 0u64;
    loop {
        let chunk = tokio::select! {
            _ = live.cancel.cancelled() => return Err(Fail::Cancelled),
            c = answer.chunk() => c.map_err(crate::probe::net_err)?,
        };
        let Some(chunk) = chunk else { break };
        tokio::select! {
            _ = live.cancel.cancelled() => return Err(Fail::Cancelled),
            _ = live.limiter.acquire(chunk.len()) => {}
        }
        file.write_all(&chunk).map_err(|e| Fail::Fatal(format!("could not write the file: {e}")))?;
        size += chunk.len() as u64;
        live.downloaded.fetch_add(chunk.len() as u64, Relaxed);
    }
    Ok(size)
}

/// Saves a post's files under `named` ("someone CODE"): one file as
/// "someone CODE.jpg", several as a folder "someone CODE" holding
/// "someone CODE 1.jpg", "someone CODE 2.mp4"… A name given with the
/// download is used in its place.
pub(crate) async fn save(inner: &Arc<Inner>, id: u64, live: &Arc<Live>, rec: &Record, named: &str, referer: &str, files: &[PostFile]) -> Result<(), Fail> {
    let saved = save_in_work(inner, id, live, rec, named, referer, files).await;
    crate::media::remove_work_dir(&rec.dir, id);
    saved
}

async fn save_in_work(inner: &Arc<Inner>, id: u64, live: &Arc<Live>, rec: &Record, named: &str, referer: &str, files: &[PostFile]) -> Result<(), Fail> {
    let work = crate::media::work_dir(&rec.dir, id);
    // A start over begins clean.
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).map_err(|e| Fail::Fatal(format!("could not create folder: {e}")))?;
    inner.update(id, |r| {
        r.part_started = true;
        r.segments.clear();
    });
    live.downloaded.store(0, Relaxed);
    live.items.store(0, Relaxed);

    let ffmpeg = find_tool("ffmpeg");
    let mut made: Vec<PathBuf> = Vec::new();
    for (n, file) in files.iter().enumerate() {
        let mut parts = Vec::new();
        for (k, (address, ext)) in file.parts.iter().enumerate() {
            let part = work.join(format!("part-{}-{k}.{ext}", n + 1));
            fetch(live, address, referer, &part).await?;
            parts.push(part);
        }
        if parts.is_empty() {
            continue;
        }
        let whole = work.join(crate::probe::sanitize(&format!("{named} {}.{}", n + 1, file.ext)));
        // one part is the file; picture and sound apart are joined
        crate::media::assemble(live, ffmpeg.as_deref(), &parts, false, &whole).await?;
        for part in parts {
            let _ = std::fs::remove_file(part);
        }
        made.push(whole);
        live.items.fetch_add(1, Relaxed);
    }

    let given = rec.user_named.then(|| Path::new(&rec.filename).file_stem().unwrap_or_default().to_string_lossy().to_string()).filter(|g| !g.is_empty());
    let name = crate::probe::sanitize(&given.unwrap_or_else(|| named.to_string()));
    let (final_name, size) = match made.as_slice() {
        [] => return Err(Fail::Fatal("This post has nothing to download.".into())),
        [one] => {
            let ext = one.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_else(|| "jpg".into());
            let size = std::fs::metadata(one).map(|m| m.len()).unwrap_or(0);
            (place(one, &rec.dir, &format!("{name}.{ext}"))?, size)
        }
        several => {
            let folder_name = crate::torrent::claim_folder(&rec.dir, &name)?;
            let folder = rec.dir.join(&folder_name);
            let mut size = 0;
            for file in several {
                size += std::fs::metadata(file).map(|m| m.len()).unwrap_or(0);
                place(file, &folder, &file.file_name().unwrap_or_default().to_string_lossy())?;
            }
            (folder_name, size)
        }
    };
    live.downloaded.store(size, Relaxed);
    live.total_hint.store(size, Relaxed);
    inner.update(id, |r| {
        r.filename = final_name.clone();
        r.total = Some(size);
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pictures_kind_is_told_by_its_address() {
        assert_eq!(picture_ext("https://cdn.test/4.webp?x=1.jpg"), "webp");
        assert_eq!(picture_ext("https://cdn.test/a.PNG#top"), "png");
        assert_eq!(picture_ext("https://cdn.test/photo"), "jpg");
        assert_eq!(PostFile::whole("https://cdn.test/a.png", "png"), PostFile { parts: vec![("https://cdn.test/a.png".into(), "png".into())], ext: "png".into() });
    }
}
