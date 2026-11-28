//! A real Instagram reel, looked up and downloaded (original, and sound
//! alone). Needs the internet, yt-dlp and FFmpeg (and gallery-dl with a
//! sign-in for posts Instagram shows only to signed-in users), so it only
//! runs when asked for:
//!
//! RUSHDOWNLOAD_TEST_INSTAGRAM=<reel link> [RUSHDOWNLOAD_TEST_COOKIES=<cookies.txt>]
//! cargo test -p rushdownload-engine --test instagram -- --ignored --nocapture

use std::sync::Arc;
use std::time::Duration;

use rushdownload_engine::{Engine, MediaOptions, MemoryStore, NewDownload, Settings, Status};

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn a_reel_downloads_in_its_original_quality_and_as_sound_alone() {
    // A sign-in is optional: without RUSHDOWNLOAD_TEST_COOKIES the post is
    // fetched as any visitor sees it, which is what public posts need.
    let cookies = std::env::var("RUSHDOWNLOAD_TEST_COOKIES").ok();
    let link = std::env::var("RUSHDOWNLOAD_TEST_INSTAGRAM").expect("RUSHDOWNLOAD_TEST_INSTAGRAM");
    let dir = std::env::temp_dir().join(format!("rushdownload-instagram-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("state")).unwrap();
    if let Some(cookies) = &cookies {
        std::fs::copy(cookies, dir.join("state").join("cookies.txt")).unwrap();
    }
    let mut settings = Settings::new(dir.join("out"));
    settings.organize.enabled = false;
    settings.auto_update_tools = false;
    let engine = Engine::with_secrets(dir.join("state"), settings, Arc::new(MemoryStore::default())).unwrap();

    let info = engine.media_info(&link).await.expect("looked up");
    println!("info: {} | {:?} | {:?}s | {:?}x{:?} | thumbnail {}", info.title, info.site, info.duration, info.width, info.height, info.thumbnail.is_some());
    assert_eq!(info.site.as_deref(), Some("Instagram"));
    assert!(info.heights.is_empty(), "one quality: the original");

    for audio_only in [false, true] {
        let id = engine
            .add(NewDownload { url: link.clone(), via_site: true, media: MediaOptions { max_height: None, audio_only }, ..Default::default() })
            .unwrap();
        let mut done = None;
        for _ in 0..600 {
            let t = engine.list().into_iter().find(|t| t.id == id).unwrap();
            if matches!(t.status, Status::Completed | Status::Failed) {
                done = Some(t);
                break;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        let t = done.expect("finished in time");
        assert_eq!(t.status, Status::Completed, "{:?}", t.error);
        println!("{}: {} ({} bytes)", if audio_only { "audio" } else { "original" }, t.path.display(), t.total.unwrap_or(0));
        assert!(t.path.is_file());
        assert!(t.filename.ends_with(if audio_only { ".mp3" } else { ".mp4" }), "{}", t.filename);
        // what the file holds: picture size and sound, where ffprobe is at hand
        if let Ok(out) = std::process::Command::new("ffprobe")
            .args(["-v", "error", "-show_entries", "stream=codec_type,codec_name,width,height", "-of", "csv=p=0"])
            .arg(&t.path)
            .output()
        {
            println!("streams: {}", String::from_utf8_lossy(&out.stdout).trim().replace(['\r', '\n'], " "));
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// What the Home card asks about an Instagram profile, with a sign-in:
/// RUSHDOWNLOAD_TEST_INSTAGRAM_PROFILE=<profile link> as well as the cookies.
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn a_profile_is_described_and_counted() {
    let cookies = std::env::var("RUSHDOWNLOAD_TEST_COOKIES").expect("RUSHDOWNLOAD_TEST_COOKIES");
    let link = std::env::var("RUSHDOWNLOAD_TEST_INSTAGRAM_PROFILE").expect("RUSHDOWNLOAD_TEST_INSTAGRAM_PROFILE");
    let dir = std::env::temp_dir().join(format!("rushdownload-instagram-profile-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("state")).unwrap();
    std::fs::copy(&cookies, dir.join("state").join("cookies.txt")).unwrap();
    let mut settings = Settings::new(dir.join("out"));
    settings.auto_update_tools = false;
    // RUSHDOWNLOAD_TEST_SECURE_DNS=off tries it without the secure-DNS relay.
    settings.secure_dns = std::env::var("RUSHDOWNLOAD_TEST_SECURE_DNS").map_or(true, |v| v != "off");
    let engine = Engine::with_secrets(dir.join("state"), settings, Arc::new(MemoryStore::default())).unwrap();
    let who = engine.profile_of(&link);
    println!("profile_of: {who:?}");
    let face = engine.profile_face(&link).await;
    println!("face: name {:?} picture {}", face.name, face.picture.is_some());
    let stats = engine.profile_stats(&link).await;
    println!("stats: posts {:?} followers {:?}", stats.posts, stats.followers);
    match engine.playlist_info(&link).await {
        Ok(info) => println!("playlist_info: {} | count {:?}", info.title, info.count),
        Err(e) => println!("playlist_info error: {e}"),
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// The plain-word refusals, against Instagram itself, with no sign-in:
/// RUSHDOWNLOAD_TEST_INSTAGRAM_RESTRICTED=<a post Instagram shows only to
/// signed-in users>.
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn refusals_are_told_in_the_words_promised() {
    let dir = std::env::temp_dir().join(format!("rushdownload-instagram-refusals-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut settings = Settings::new(dir.join("out"));
    settings.auto_update_tools = false;
    let engine = Engine::with_secrets(dir.join("state"), settings, Arc::new(MemoryStore::default())).unwrap();
    let told = |e: Result<rushdownload_engine::MediaInfo, String>| e.err().unwrap_or_default();
    let invalid = told(engine.media_info("https://www.instagram.com/explore/").await);
    println!("not a post: {invalid}");
    assert_eq!(invalid, "Please enter a valid Instagram URL.");
    let gone = told(engine.media_info("https://www.instagram.com/reel/Dzzzzzzzzzz/").await);
    println!("no such reel: {gone}");
    assert!(gone == "This Instagram video is currently unavailable." || gone.contains("without authentication"), "{gone}");
    if let Ok(restricted) = std::env::var("RUSHDOWNLOAD_TEST_INSTAGRAM_RESTRICTED") {
        let shut = told(engine.media_info(&restricted).await);
        println!("restricted: {shut}");
        assert!(shut.contains("without authentication"), "{shut}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Public posts read the way Instagram's own page reads them for a visitor
/// (instagram_web.rs), with no sign-in and without yt-dlp: a reel, a photo,
/// and several things in one post.
///
/// RUSHDOWNLOAD_INSTAGRAM_NO_YTDLP=1 RUSHDOWNLOAD_TEST_INSTAGRAM_POSTS="<link> <link> ..."
/// cargo test -p rushdownload-engine --test instagram as_a_visitors_page -- --ignored --nocapture
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn posts_are_read_and_saved_as_a_visitors_page_reads_them() {
    let links = std::env::var("RUSHDOWNLOAD_TEST_INSTAGRAM_POSTS").expect("RUSHDOWNLOAD_TEST_INSTAGRAM_POSTS");
    let dir = std::env::temp_dir().join(format!("rushdownload-instagram-page-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut settings = Settings::new(dir.join("out"));
    settings.organize.enabled = false;
    settings.auto_update_tools = false;
    let engine = Engine::with_secrets(dir.join("state"), settings, Arc::new(MemoryStore::default())).unwrap();

    for link in links.split_whitespace() {
        let info = engine.media_info(link).await.expect("looked up");
        println!("info: {} | {:?}s | {:?}x{:?} | by {:?} | likes {:?} | thumbnail {}", info.title, info.duration, info.width, info.height, info.uploader, info.likes, info.thumbnail.is_some());
        assert_eq!(info.site.as_deref(), Some("Instagram"));
        let video = info.title.starts_with("Video by");
        for audio_only in [false, true] {
            if audio_only && !video {
                continue;
            }
            let id = engine
                .add(NewDownload { url: link.to_string(), via_site: true, media: MediaOptions { max_height: None, audio_only }, ..Default::default() })
                .unwrap();
            let mut done = None;
            for _ in 0..600 {
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
    let _ = std::fs::remove_dir_all(&dir);
}
