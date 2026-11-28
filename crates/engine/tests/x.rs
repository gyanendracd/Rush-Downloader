//! Real posts on X, looked up and downloaded with no sign-in: a video, a
//! photo, and several photos in one post. Needs the internet (and FFmpeg for
//! a video's sound alone), so it only runs when asked for:
//!
//! RUSHDOWNLOAD_TEST_X_POSTS="<link> <link> ..."
//! cargo test -p rushdownload-engine --test x -- --ignored --nocapture
//!
//! With RUSHDOWNLOAD_X_NO_YTDLP=1 every post is read from X's own service
//! for embedded posts (x.rs); without it yt-dlp reads a post first, as the
//! app does.

use std::sync::Arc;
use std::time::Duration;

use rushdownload_engine::{Engine, MediaOptions, MemoryStore, NewDownload, Settings, Status};

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn posts_are_looked_up_and_saved_with_no_sign_in() {
    let links = std::env::var("RUSHDOWNLOAD_TEST_X_POSTS").expect("RUSHDOWNLOAD_TEST_X_POSTS");
    let dir = std::env::temp_dir().join(format!("rushdownload-x-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut settings = Settings::new(dir.join("out"));
    settings.organize.enabled = false;
    settings.auto_update_tools = false;
    let engine = Engine::with_secrets(dir.join("state"), settings, Arc::new(MemoryStore::default())).unwrap();

    for link in links.split_whitespace() {
        let info = engine.media_info(link).await.expect("looked up");
        println!(
            "info: {} | {:?}s | {:?}x{:?} | sizes {:?} | by {:?} {:?} | likes {:?} | thumbnail {}",
            info.title, info.duration, info.width, info.height, info.heights, info.uploader, info.uploader_url, info.likes, info.thumbnail.is_some()
        );
        assert_eq!(info.site.as_deref(), Some("Twitter"));
        let video = info.duration.is_some();
        for audio_only in [false, true] {
            if audio_only && !video {
                continue;
            }
            let id = engine
                .add(NewDownload { url: link.to_string(), via_site: true, media: MediaOptions { max_height: None, audio_only }, ..Default::default() })
                .unwrap();
            let mut done = None;
            for _ in 0..720 {
                let t = engine.list().into_iter().find(|t| t.id == id).unwrap();
                if matches!(t.status, Status::Completed | Status::Failed) {
                    done = Some(t);
                    break;
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
            let t = done.expect("finished in time");
            assert_eq!(t.status, Status::Completed, "{:?}", t.error);
            println!("  saved: {} ({} bytes)", t.path.display(), t.total.unwrap_or(0));
            if t.path.is_dir() {
                let inside: Vec<_> = std::fs::read_dir(&t.path).unwrap().flatten().map(|e| (e.file_name().to_string_lossy().to_string(), e.metadata().map(|m| m.len()).unwrap_or(0))).collect();
                println!("  holds: {inside:?}");
                assert!(inside.len() > 1 && inside.iter().all(|(_, size)| *size > 1000), "{inside:?}");
            } else {
                assert!(t.path.is_file() && t.total.unwrap_or(0) > 1000);
                if let Ok(out) = std::process::Command::new("ffprobe")
                    .args(["-v", "error", "-show_entries", "stream=codec_type,codec_name,width,height", "-of", "csv=p=0"])
                    .arg(&t.path)
                    .output()
                {
                    println!("  streams: {}", String::from_utf8_lossy(&out.stdout).trim().replace(['\r', '\n'], " "));
                }
            }
            // nothing of the download's own is left behind
            let left: Vec<_> = std::fs::read_dir(dir.join("out")).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_string()).filter(|n| n.starts_with(".rush-")).collect();
            assert!(left.is_empty(), "{left:?}");
        }
    }

    // what X does not give is told in plain words
    let told = |e: Result<rushdownload_engine::MediaInfo, String>| e.err().unwrap_or_default();
    let words_only = told(engine.media_info("https://x.com/jack/status/20").await);
    println!("words only: {words_only}");
    let gone = told(engine.media_info("https://x.com/someone/status/2053798185756422399").await);
    println!("no such post: {gone}");
    assert!(!words_only.is_empty() && !gone.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}
