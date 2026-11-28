//! End-to-end tests against a small local HTTP server.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::sync::Arc;
use std::time::Duration;

use rushdownload_engine::{Engine, Fetcher, MediaOptions, NewDownload, Settings, Status, TaskInfo};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

struct Server {
    url: String,
    /// Number of ranged requests served.
    ranged: Arc<AtomicUsize>,
}

fn payload(len: usize) -> Arc<Vec<u8>> {
    let mut x = 0x2545F4914F6CDD1Du64;
    Arc::new(
        (0..len)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                x as u8
            })
            .collect(),
    )
}

/// Serves `data` at any path. `ranges` toggles Range support and `delay`
/// throttles the body (per 32 KiB) so tests can pause mid-download.
async fn serve(data: Arc<Vec<u8>>, ranges: bool, delay: Duration) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/files/test%20file.bin", listener.local_addr().unwrap());
    let ranged = Arc::new(AtomicUsize::new(0));
    let counter = ranged.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else { return };
            let data = data.clone();
            let counter = counter.clone();
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut tmp = [0u8; 2048];
                while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    match sock.read(&mut tmp).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => buf.extend_from_slice(&tmp[..n]),
                    }
                }
                let head = String::from_utf8_lossy(&buf).to_ascii_lowercase();
                let range = head
                    .lines()
                    .find_map(|l| l.strip_prefix("range: bytes="))
                    .filter(|_| ranges)
                    .map(|r| {
                        let (a, b) = r.trim().split_once('-').unwrap();
                        let a: usize = a.parse().unwrap();
                        let b: usize = b.parse().unwrap_or(data.len() - 1);
                        (a, b.min(data.len() - 1))
                    });
                let (header, body) = match range {
                    Some((a, b)) => {
                        counter.fetch_add(1, Relaxed);
                        (
                            format!(
                                "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {}-{}/{}\r\nETag: \"v1\"\r\nConnection: close\r\n\r\n",
                                b - a + 1, a, b, data.len()
                            ),
                            &data[a..=b],
                        )
                    }
                    None => (
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Disposition: attachment; filename=\"plain.bin\"\r\nConnection: close\r\n\r\n",
                            data.len()
                        ),
                        &data[..],
                    ),
                };
                if sock.write_all(header.as_bytes()).await.is_err() {
                    return;
                }
                for piece in body.chunks(32 * 1024) {
                    if sock.write_all(piece).await.is_err() {
                        return;
                    }
                    if !delay.is_zero() {
                        tokio::time::sleep(delay).await;
                    }
                }
                let _ = sock.shutdown().await;
            });
        }
    });
    Server { url, ranged }
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rushdownload-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// An engine that leaves finished files where they were downloaded: these
/// tests look at the download folder itself. Sorting has its own test.
fn engine(dir: &PathBuf) -> Engine {
    let mut settings = Settings::new(dir.join("out"));
    settings.organize.enabled = false;
    // never download helper programs during tests
    settings.auto_update_tools = false;
    Engine::new(dir.join("state"), settings).unwrap()
}

async fn wait_for(engine: &Engine, id: u64, pred: impl Fn(&TaskInfo) -> bool) -> TaskInfo {
    for _ in 0..600 {
        if let Some(t) = engine.list().into_iter().find(|t| t.id == id) {
            if pred(&t) {
                return t;
            }
            assert_ne!(t.status, Status::Failed, "download failed: {:?}", t.error);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("timed out: {:?}", engine.list());
}

fn new(url: &str) -> NewDownload {
    NewDownload {
        url: url.into(),
        ..Default::default()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn multi_connection_download_is_byte_exact() {
    let data = payload(24 * 1024 * 1024 + 12345);
    let server = serve(data.clone(), true, Duration::ZERO).await;
    let dir = temp_dir("multi");
    let engine = engine(&dir);

    let id = engine.add(new(&server.url)).unwrap();
    let done = wait_for(&engine, id, |t| t.status == Status::Completed).await;

    assert_eq!(done.filename, "test file.bin");
    assert_eq!(done.downloaded, data.len() as u64);
    assert_eq!(std::fs::read(&done.path).unwrap(), *data);
    assert!(!dir.join("out/test file.bin.part").exists());
    // 1 probe + at least one request per connection.
    assert!(server.ranged.load(Relaxed) >= 9, "expected parallel range requests");

    // A second download of the same URL must not overwrite the first.
    let id2 = engine.add(new(&server.url)).unwrap();
    let done2 = wait_for(&engine, id2, |t| t.status == Status::Completed).await;
    assert_eq!(done2.filename, "test file (1).bin");
    assert_eq!(std::fs::read(&done2.path).unwrap(), *data);
}

#[tokio::test(flavor = "multi_thread")]
async fn pause_and_resume_continue_where_they_stopped() {
    let data = payload(8 * 1024 * 1024);
    let server = serve(data.clone(), true, Duration::from_millis(15)).await;
    let dir = temp_dir("resume");
    let engine = engine(&dir);

    let id = engine.add(new(&server.url)).unwrap();
    wait_for(&engine, id, |t| t.downloaded > 1024 * 1024).await;
    engine.pause(id);
    let paused = wait_for(&engine, id, |t| {
        t.status == Status::Paused && t.active_connections == 0
    })
    .await;
    assert!(paused.downloaded < data.len() as u64);
    assert!(dir.join("out/test file.bin.part").exists());

    // A fresh engine (app restart) must pick up the saved progress.
    tokio::time::sleep(Duration::from_millis(200)).await;
    drop(engine);
    let engine = self::engine(&dir);
    let restored = engine.list().into_iter().find(|t| t.id == id).unwrap();
    assert_eq!(restored.status, Status::Paused);
    assert!(restored.downloaded >= paused.downloaded);

    let before = server.ranged.load(Relaxed);
    engine.resume(id);
    let done = wait_for(&engine, id, |t| t.status == Status::Completed).await;
    assert_eq!(std::fs::read(&done.path).unwrap(), *data);
    assert!(server.ranged.load(Relaxed) > before);
}

#[tokio::test(flavor = "multi_thread")]
async fn falls_back_to_single_stream_without_range_support() {
    let data = payload(3 * 1024 * 1024 + 7);
    let server = serve(data.clone(), false, Duration::ZERO).await;
    let dir = temp_dir("single");
    let engine = engine(&dir);

    let id = engine.add(new(&server.url)).unwrap();
    let done = wait_for(&engine, id, |t| t.status == Status::Completed).await;
    assert_eq!(done.filename, "plain.bin");
    assert!(!done.resumable);
    assert_eq!(std::fs::read(&done.path).unwrap(), *data);
}

#[tokio::test(flavor = "multi_thread")]
async fn remove_deletes_partial_data_and_speed_limit_applies() {
    let data = payload(16 * 1024 * 1024);
    let server = serve(data.clone(), true, Duration::ZERO).await;
    let dir = temp_dir("remove");
    let engine = engine(&dir);
    let mut settings = engine.settings();
    settings.speed_limit = 1024 * 1024;
    engine.set_settings(settings);

    let id = engine.add(new(&server.url)).unwrap();
    wait_for(&engine, id, |t| t.downloaded > 256 * 1024).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let t = engine.list().into_iter().find(|t| t.id == id).unwrap();
    // ~1 MiB/s for a bit over two seconds; unlimited would have finished.
    assert!(t.downloaded < 6 * 1024 * 1024, "limit ignored: {}", t.downloaded);
    assert_eq!(t.status, Status::Downloading);

    engine.remove(id, false);
    for _ in 0..100 {
        if engine.list().is_empty() && !dir.join("out/test file.bin.part").exists() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("partial file was not cleaned up");
}

/// Needs internet, yt-dlp and ffmpeg: `cargo test -p rushdownload-engine -- --ignored`
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn site_download_can_be_paused_and_removed() {
    let dir = temp_dir("site");
    let engine = engine(&dir);
    let mut settings = engine.settings();
    settings.speed_limit = 300 * 1024;
    engine.set_settings(settings);

    let id = engine
        .add(new("https://www.youtube.com/watch?v=QtYaN5FlaOI"))
        .unwrap();
    let t = wait_for(&engine, id, |t| t.downloaded > 200 * 1024).await;
    assert!(t.media);
    assert!(t.filename.contains("[QtYaN5FlaOI]"), "{}", t.filename);

    engine.pause(id);
    let paused = wait_for(&engine, id, |t| {
        t.status == Status::Paused && t.active_connections == 0
    })
    .await;
    // Nothing may keep downloading in the background after a pause.
    tokio::time::sleep(Duration::from_secs(2)).await;
    let size = |d: &PathBuf| -> u64 {
        std::fs::read_dir(d.join("out"))
            .unwrap()
            .flatten()
            .map(|e| e.metadata().map_or(0, |m| m.len()))
            .sum()
    };
    let before = size(&dir);
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(before, size(&dir), "files still growing after pause");
    assert!(paused.downloaded > 0);

    engine.remove(id, false);
    tokio::time::sleep(Duration::from_millis(500)).await;
    let left: Vec<_> = std::fs::read_dir(dir.join("out"))
        .unwrap()
        .flatten()
        .map(|e| e.file_name())
        .collect();
    assert!(left.is_empty(), "leftover files: {left:?}");
}

/// Needs internet, yt-dlp and ffmpeg: `cargo test -p rushdownload-engine -- --ignored`
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn site_download_honours_resolution_and_audio_only() {
    let dir = temp_dir("quality");
    let engine = engine(&dir);
    let url = "https://www.youtube.com/watch?v=QtYaN5FlaOI";

    let video = engine
        .add(NewDownload {
            url: url.into(),
            media: MediaOptions {
                max_height: Some(360),
                audio_only: false,
            },
            ..Default::default()
        })
        .unwrap();
    let audio = engine
        .add(NewDownload {
            url: url.into(),
            media: MediaOptions {
                max_height: None,
                audio_only: true,
            },
            ..Default::default()
        })
        .unwrap();

    let v = wait_for(&engine, video, |t| t.status == Status::Completed).await;
    let a = wait_for(&engine, audio, |t| t.status == Status::Completed).await;
    println!("video: {} ({} bytes)", v.filename, v.downloaded);
    println!("audio: {} ({} bytes)", a.filename, a.downloaded);
    assert!(v.filename.contains(" 360p ["), "{}", v.filename);
    assert!(v.path.is_file() && v.downloaded > 0);
    assert!(a.filename.ends_with(".mp3"), "{}", a.filename);
    assert!(a.path.is_file() && a.downloaded > 0);
    println!("OUT={}", dir.join("out").display());
}

/// Needs internet: `cargo test -p rushdownload-engine -- --ignored torrent`
/// Uses Ubuntu's official torrent, which is free to share.
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn torrent_can_be_paused_resumed_and_removed() {
    let dir = temp_dir("torrent");
    let engine = engine(&dir);
    let id = engine
        .add(new("https://releases.ubuntu.com/24.04/ubuntu-24.04.3-live-server-amd64.iso.torrent"))
        .unwrap();

    let slow = |pred: fn(&TaskInfo) -> bool, what: &'static str| {
        let engine = engine.clone();
        async move {
            for _ in 0..1200 {
                let t = engine.list().into_iter().find(|t| t.id == id).expect("torrent in list");
                assert_ne!(t.status, Status::Failed, "torrent failed: {:?}", t.error);
                if pred(&t) {
                    return t;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            panic!("timed out waiting for {what}");
        }
    };

    let running = slow(|t| t.downloaded > 3 * 1024 * 1024, "the first few megabytes").await;
    assert!(running.torrent);
    assert_eq!(running.filename, "ubuntu-24.04.3-live-server-amd64.iso");
    assert!(running.total.unwrap_or(0) > 1_000_000_000, "total: {:?}", running.total);
    assert!(running.path.is_dir(), "the torrent's own folder exists");
    println!("running: {} of {:?} bytes, {} B/s", running.downloaded, running.total, running.speed);

    engine.pause(id);
    let paused = slow(|t| t.status == Status::Paused && t.active_connections == 0, "pause").await;
    tokio::time::sleep(Duration::from_secs(3)).await;
    let later = engine.list().into_iter().find(|t| t.id == id).unwrap();
    assert_eq!(later.downloaded, paused.downloaded, "nothing downloads while paused");
    println!("paused at {} bytes", paused.downloaded);

    engine.resume(id);
    let floor = paused.downloaded;
    for _ in 0..1200 {
        let t = engine.list().into_iter().find(|t| t.id == id).unwrap();
        assert_ne!(t.status, Status::Failed, "resume failed: {:?}", t.error);
        if t.status == Status::Downloading && t.downloaded > floor + 2 * 1024 * 1024 {
            println!("resumed: kept {} bytes, now {}", floor, t.downloaded);
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let resumed = engine.list().into_iter().find(|t| t.id == id).unwrap();
    assert!(resumed.downloaded > floor, "resume continued past the paused point");

    let folder = resumed.path.clone();
    engine.remove(id, false);
    for _ in 0..100 {
        if engine.list().is_empty() && !folder.exists() {
            println!("removed: folder is gone");
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the torrent's folder was not removed: {}", folder.display());
}

/// Needs internet and a .torrent file: set RUSH_TEST_TORRENT to its path, then
/// `cargo test --release -p rushdownload-engine -- --ignored local_torrent`
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn local_torrent_file_previews_and_downloads() {
    let source = std::env::var("RUSH_TEST_TORRENT").expect("set RUSH_TEST_TORRENT");
    std::env::set_var("RUSHDOWNLOAD_NO_DHT", "1");
    let dir = temp_dir("localtorrent");
    let engine = engine(&dir);

    // Quoted paths (as copied from Explorer) are accepted too.
    let preview = engine.torrent_preview(&format!("\"{source}\"")).await.unwrap();
    println!("preview: {} | {} bytes | {} file(s)", preview.name, preview.total, preview.files.len());
    for f in preview.files.iter().take(5) {
        println!("  [{}] {} ({} bytes)", f.index, f.path.join("/"), f.size);
    }
    assert!(!preview.files.is_empty() && preview.total > 0);

    let custom = dir.join("chosen folder");
    let id = engine
        .add(NewDownload {
            url: source.clone(),
            dir: Some(custom.clone()),
            ..Default::default()
        })
        .unwrap();
    let mut seen = None;
    for _ in 0..1200 {
        let t = engine.list().into_iter().find(|t| t.id == id).unwrap();
        assert_ne!(t.status, Status::Failed, "failed: {:?}", t.error);
        if t.downloaded > 3 * 1024 * 1024 {
            seen = Some(t);
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let t = seen.expect("the torrent file started downloading");
    println!("running: {} of {:?} bytes into {}", t.downloaded, t.total, t.path.display());
    assert!(t.torrent && t.path.starts_with(&custom) && t.path.is_dir());
    let copy = dir.join("state").join("torrents").join(format!("{id}.torrent"));
    assert!(copy.is_file(), "the app keeps its own copy of the .torrent file");

    engine.remove(id, false);
    for _ in 0..100 {
        if engine.list().is_empty() && !t.path.exists() && !copy.exists() {
            println!("removed: data folder and saved copy are gone");
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("leftovers after removing the torrent");
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_one_of_two_identical_downloads_keeps_the_other() {
    let data = payload(6 * 1024 * 1024);
    let server = serve(data.clone(), true, Duration::from_millis(12)).await;
    let dir = temp_dir("twins");
    let engine = engine(&dir);

    // The very same link, twice, at the same moment.
    let first = engine.add(new(&server.url)).unwrap();
    let second = engine.add(new(&server.url)).unwrap();
    let a = wait_for(&engine, first, |t| t.downloaded > 512 * 1024).await;
    let b = wait_for(&engine, second, |t| t.downloaded > 512 * 1024).await;
    assert_ne!(a.filename, b.filename, "each copy has its own file name");

    // Cancel only the second one.
    engine.remove(second, false);
    for _ in 0..100 {
        if engine.list().iter().all(|t| t.id != second) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let left: Vec<u64> = engine.list().iter().map(|t| t.id).collect();
    assert_eq!(left, vec![first], "only the cancelled copy leaves the list");

    // The first must carry on and finish intact.
    let done = wait_for(&engine, first, |t| t.status == Status::Completed).await;
    assert_eq!(std::fs::read(&done.path).unwrap(), *data);
    let names: Vec<String> = std::fs::read_dir(dir.join("out"))
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    assert_eq!(names, vec![done.filename.clone()], "the cancelled copy left nothing behind");
}

/// Serves `data` as a video file at any path, with HEAD support, the way a
/// plain media host does.
async fn serve_video(data: Arc<Vec<u8>>, delay: Duration) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/media/clip.mp4", listener.local_addr().unwrap());
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else { return };
            let data = data.clone();
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut tmp = [0u8; 2048];
                while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    match sock.read(&mut tmp).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => buf.extend_from_slice(&tmp[..n]),
                    }
                }
                let head = String::from_utf8_lossy(&buf).to_ascii_lowercase();
                let is_head = head.starts_with("head ");
                let from = head
                    .lines()
                    .find_map(|l| l.strip_prefix("range: bytes="))
                    .and_then(|r| r.trim().split_once('-'))
                    .and_then(|(a, _)| a.parse::<usize>().ok())
                    .filter(|a| *a < data.len());
                let header = match from {
                    Some(a) => format!(
                        "HTTP/1.1 206 Partial Content\r\nContent-Type: video/mp4\r\nAccept-Ranges: bytes\r\nContent-Length: {}\r\nContent-Range: bytes {}-{}/{}\r\nConnection: close\r\n\r\n",
                        data.len() - a, a, data.len() - 1, data.len()
                    ),
                    None => format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: video/mp4\r\nAccept-Ranges: bytes\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        data.len()
                    ),
                };
                if sock.write_all(header.as_bytes()).await.is_err() || is_head {
                    let _ = sock.shutdown().await;
                    return;
                }
                for piece in data[from.unwrap_or(0)..].chunks(32 * 1024) {
                    if sock.write_all(piece).await.is_err() {
                        return;
                    }
                    tokio::time::sleep(delay).await;
                }
                let _ = sock.shutdown().await;
            });
        }
    });
    url
}

fn files_in(dir: &PathBuf) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    names.sort();
    names
}

/// Needs yt-dlp, but no internet:
/// `cargo test --release -p rushdownload-engine -- --ignored twins`
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn site_download_twins_do_not_touch_each_other() {
    let data = payload(8 * 1024 * 1024);
    let url = serve_video(data.clone(), Duration::from_millis(25)).await;
    let dir = temp_dir("sitetwins");
    let engine = engine(&dir);
    let site = || NewDownload {
        url: url.clone(),
        via_site: true,
        ..Default::default()
    };

    let first = engine.add(site()).unwrap();
    let second = engine.add(site()).unwrap();
    let a = wait_for(&engine, first, |t| t.downloaded > 256 * 1024).await;
    wait_for(&engine, second, |t| t.downloaded > 256 * 1024).await;
    assert!(a.media, "the test went through the site downloader");
    // A plain video file is fetched by several connections at once.
    println!("connections: {} | parts: {}", a.active_connections, a.segments.len());
    if std::env::var_os("RUSHDOWNLOAD_NO_FAST_SITE").is_none() {
        assert!(a.segments.len() > 1, "the video is split between connections");
        assert!(
            a.segments.iter().filter(|[start, pos, _]| pos > start).count() > 1,
            "more than one part is filling at the same time"
        );
    }
    println!("both running: {:?}", files_in(&dir.join("out")));

    // Cancel only the second one.
    engine.remove(second, false);
    for _ in 0..100 {
        if engine.list().iter().all(|t| t.id != second) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let done = wait_for(&engine, first, |t| t.status == Status::Completed).await;
    assert_eq!(std::fs::read(&done.path).unwrap(), *data, "the kept copy is intact");
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(
        files_in(&dir.join("out")),
        vec![done.filename.clone()],
        "the cancelled copy left nothing behind"
    );

    // A later copy of the same video gets a file of its own.
    let third = engine.add(site()).unwrap();
    let again = wait_for(&engine, third, |t| t.status == Status::Completed).await;
    println!("first: {} | again: {}", done.filename, again.filename);
    assert_ne!(again.filename, done.filename);
    assert_eq!(std::fs::read(&done.path).unwrap(), *data, "the first copy was not overwritten");
    assert_eq!(std::fs::read(&again.path).unwrap(), *data);
}

/// Needs internet and a .torrent file: set RUSH_TEST_TORRENT to its path, then
/// `cargo test --release -p rushdownload-engine -- --ignored torrent_twin`
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn torrent_twin_downloads_as_its_own_copy() {
    let source = std::env::var("RUSH_TEST_TORRENT").expect("set RUSH_TEST_TORRENT");
    std::env::set_var("RUSHDOWNLOAD_NO_DHT", "1");
    let dir = temp_dir("torrenttwin");
    let engine = engine(&dir);
    let get = |id: u64| engine.list().into_iter().find(|t| t.id == id);
    let until_past = |id: u64, bytes: u64| {
        let engine = engine.clone();
        async move {
            for _ in 0..1200 {
                let t = engine.list().into_iter().find(|t| t.id == id).expect("in the list");
                assert_ne!(t.status, Status::Failed, "failed: {:?}", t.error);
                if t.downloaded > bytes {
                    return t;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            panic!("download {id} did not get past {bytes} bytes");
        }
    };

    // The very same torrent, twice, at the same time.
    let first = engine.add(new(&source)).unwrap();
    let a = until_past(first, 2 * 1024 * 1024).await;
    let second = engine.add(new(&source)).unwrap();
    let b = until_past(second, 2 * 1024 * 1024).await;
    println!("first: {} | copy: {}", a.path.display(), b.path.display());
    let filling: Vec<String> = b
        .segments
        .iter()
        .map(|[start, pos, end]| format!("{}/{}", pos - start, end - start))
        .collect();
    println!("parts of the copy: {}", filling.join(" "));
    assert!(b.segments.len() > 1, "a torrent reports its parts");
    // The pieces come in order from the start of the file, so the parts
    // fill one after another; what matters is that real progress is reported.
    assert!(
        b.segments.iter().any(|[start, pos, _]| pos > start),
        "the parts show the pieces that are done"
    );
    assert!(b.segments.iter().all(|[start, pos, end]| start <= pos && pos <= end));
    assert_ne!(a.path, b.path, "each copy has its own folder");
    assert!(a.path.is_dir() && b.path.is_dir());

    // Pausing the copy leaves the first running.
    engine.pause(second);
    for _ in 0..100 {
        if get(second).unwrap().status == Status::Paused {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(get(second).unwrap().status, Status::Paused);
    let now = get(first).unwrap();
    assert_eq!(now.status, Status::Downloading);
    until_past(first, now.downloaded + 1024 * 1024).await;

    // The copy resumes, still as its own download.
    engine.resume(second);
    let paused_at = get(second).unwrap().downloaded;
    until_past(second, paused_at + 1024 * 1024).await;

    // Cancelling the copy removes only the copy.
    engine.remove(second, false);
    for _ in 0..100 {
        if get(second).is_none() && !b.path.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(get(second).is_none() && !b.path.exists(), "the copy and its folder are gone");
    let before = get(first).expect("the first torrent is still listed");
    assert!(before.path.is_dir(), "its folder is still there");
    until_past(first, before.downloaded + 1024 * 1024).await;
    println!("first kept going past {} bytes", before.downloaded);

    engine.remove(first, false);
    for _ in 0..100 {
        if engine.list().is_empty() && !before.path.exists() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("leftovers after removing the torrent");
}

#[tokio::test(flavor = "multi_thread")]
async fn failed_connection_attempts_are_counted_while_connecting() {
    // A port nothing listens on: every attempt is refused.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/file.bin", listener.local_addr().unwrap());
    drop(listener);
    let dir = temp_dir("failing");
    let engine = engine(&dir);
    let id = engine.add(new(&url)).unwrap();

    let mut seen = 0;
    for _ in 0..400 {
        let t = engine.list().into_iter().find(|t| t.id == id).unwrap();
        if t.status == Status::Failed {
            break;
        }
        if t.connect_failures > seen {
            assert_eq!(t.status, Status::Connecting, "it is still trying");
            seen = t.connect_failures;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(seen >= 2, "saw {seen} failed attempts before it gave up");
    let t = engine.list().into_iter().find(|t| t.id == id).unwrap();
    assert_eq!(t.status, Status::Failed);
    assert_eq!(t.connect_failures, 0, "nothing is counted once it has stopped");
}

/// Needs yt-dlp, but no internet:
/// `cargo test --release -p rushdownload-engine -- --ignored site_download_resumes`
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn site_download_resumes_its_connections_after_a_pause() {
    let data = payload(12 * 1024 * 1024);
    let url = serve_video(data.clone(), Duration::from_millis(40)).await;
    let dir = temp_dir("siteresume");
    let engine = engine(&dir);
    let id = engine
        .add(NewDownload {
            url: url.clone(),
            via_site: true,
            ..Default::default()
        })
        .unwrap();
    wait_for(&engine, id, |t| t.downloaded > 2 * 1024 * 1024).await;

    engine.pause(id);
    let paused = wait_for(&engine, id, |t| t.status == Status::Paused && t.active_connections == 0).await;
    println!("paused at {} bytes in {} parts", paused.downloaded, paused.segments.len());
    assert!(paused.downloaded > 2 * 1024 * 1024 && paused.segments.len() > 1);

    engine.resume(id);
    let back = wait_for(&engine, id, |t| t.status == Status::Downloading && t.active_connections > 0).await;
    println!("resumed at {} bytes", back.downloaded);
    assert!(
        back.downloaded >= paused.downloaded,
        "what was downloaded before the pause is kept"
    );
    let done = wait_for(&engine, id, |t| t.status == Status::Completed).await;
    assert_eq!(std::fs::read(&done.path).unwrap(), *data, "the file is intact");
    assert_eq!(files_in(&dir.join("out")), vec![done.filename.clone()]);
}

/// Needs internet, yt-dlp and ffmpeg. Set RUSH_TEST_SITE_URL to a video page:
/// `cargo test --release -p rushdownload-engine -- --ignored site_speed --nocapture`
/// Prints how the video was fetched and how fast, then removes it.
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn site_speed_with_many_connections() {
    let url = std::env::var("RUSH_TEST_SITE_URL").expect("set RUSH_TEST_SITE_URL");
    std::env::set_var("RUSHDOWNLOAD_DEBUG", "1");
    let dir = temp_dir("sitespeed");
    let engine = engine(&dir);
    let id = engine
        .add(NewDownload {
            url,
            media: MediaOptions {
                max_height: Some(720),
                audio_only: false,
            },
            ..Default::default()
        })
        .unwrap();
    let started = std::time::Instant::now();
    let mut last = 0u64;
    for tick in 0..120 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        let Some(t) = engine.list().into_iter().find(|t| t.id == id) else { break };
        if tick % 2 == 1 {
            let filling = t.segments.iter().filter(|[s, p, e]| p > s && p < e).count();
            println!(
                "{:>4.1}s {:?} {} of {:?} bytes, {} KB/s, {} connections, {} parts ({} filling)",
                started.elapsed().as_secs_f32(),
                t.status,
                t.downloaded,
                t.total,
                (t.downloaded.saturating_sub(last)) / 1024,
                t.active_connections,
                t.segments.len(),
                filling
            );
            last = t.downloaded;
        }
        if t.status == Status::Failed {
            println!("failed: {:?}", t.error);
            break;
        }
        // RUSH_TEST_SITE_FULL=1 lets it finish, to check the joined file.
        let full = std::env::var_os("RUSH_TEST_SITE_FULL").is_some();
        if t.status == Status::Completed {
            println!("finished in {:.1}s: {} ({} bytes)", started.elapsed().as_secs_f32(), t.filename, t.downloaded);
            let report = std::process::Command::new("ffmpeg")
                .args(["-hide_banner", "-i"])
                .arg(&t.path)
                .output()
                .map(|o| String::from_utf8_lossy(&o.stderr).to_string())
                .unwrap_or_default();
            for line in report.lines().filter(|l| l.contains("Duration:") || l.contains("Stream #")) {
                println!("  {}", line.trim());
            }
            assert!(report.contains("Video:") && report.contains("Audio:"), "the file has picture and sound");
            break;
        }
        if !full && t.downloaded > 40 * 1024 * 1024 {
            break;
        }
    }
    engine.remove(id, true);
    tokio::time::sleep(Duration::from_secs(1)).await;
}

/// Needs internet and gallery-dl:
/// `cargo test --release -p rushdownload-engine -- --ignored profile_link --nocapture`
/// Fetches the first few files of Bluesky's own public account.
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn profile_link_downloads_into_images_and_videos() {
    std::env::set_var("RUSHDOWNLOAD_PROFILE_LIMIT", "6");
    let dir = temp_dir("profile");
    let engine = engine(&dir);
    let id = engine.add(new("https://bsky.app/profile/bsky.app")).unwrap();
    let first = engine.list().into_iter().find(|t| t.id == id).unwrap();
    assert!(first.profile);
    assert_eq!(first.filename, "bsky.app");

    let mut done = None;
    for _ in 0..1200 {
        let t = engine.list().into_iter().find(|t| t.id == id).unwrap();
        assert_ne!(t.status, Status::Failed, "failed: {:?}", t.error);
        if t.status == Status::Completed {
            done = Some(t);
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let done = done.expect("the profile finished");
    let images = files_in(&done.path.join("Images"));
    let videos = files_in(&done.path.join("Videos"));
    println!("{} files, {} bytes", done.items, done.downloaded);
    println!("Images: {images:?}");
    println!("Videos: {videos:?}");
    println!("folder: {:?}", files_in(&done.path));
    assert!(done.path.is_dir() && done.path.ends_with("bsky.app"));
    assert_eq!(done.items as usize, images.len() + videos.len());
    assert!(!images.is_empty() || !videos.is_empty());
    assert!(images.iter().all(|n| !n.ends_with(".mp4")) && videos.iter().all(|n| !n.ends_with(".jpg")));
    assert_eq!(files_in(&done.path), vec!["Images".to_string(), "Videos".to_string()], "no leftovers");

    engine.remove(id, true);
    for _ in 0..50 {
        if !done.path.exists() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the profile folder was not deleted");
}

/// A site that answers 403 to anything that is not a browser. `video` decides
/// whether a browser gets a video or a plain page.
async fn serve_browsers_only(data: Arc<Vec<u8>>, video: bool, path: &str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}{path}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else { return };
            let data = data.clone();
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut tmp = [0u8; 2048];
                while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    match sock.read(&mut tmp).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => buf.extend_from_slice(&tmp[..n]),
                    }
                }
                let head = String::from_utf8_lossy(&buf).to_ascii_lowercase();
                let is_head = head.starts_with("head ");
                let browser = head.lines().any(|l| l.starts_with("user-agent:") && l.contains("mozilla"));
                let page = b"<html><head><title>Nothing here</title></head><body><p>Just text.</p></body></html>";
                let (header, body): (String, &[u8]) = if !browser {
                    ("HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(), &[])
                } else if video {
                    let from = head
                        .lines()
                        .find_map(|l| l.strip_prefix("range: bytes="))
                        .and_then(|r| r.trim().split_once('-'))
                        .and_then(|(a, _)| a.parse::<usize>().ok())
                        .filter(|a| *a < data.len());
                    match from {
                        Some(a) => (
                            format!(
                                "HTTP/1.1 206 Partial Content\r\nContent-Type: video/mp4\r\nAccept-Ranges: bytes\r\nContent-Length: {}\r\nContent-Range: bytes {}-{}/{}\r\nConnection: close\r\n\r\n",
                                data.len() - a, a, data.len() - 1, data.len()
                            ),
                            &data[a..],
                        ),
                        None => (
                            format!(
                                "HTTP/1.1 200 OK\r\nContent-Type: video/mp4\r\nAccept-Ranges: bytes\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                data.len()
                            ),
                            &data[..],
                        ),
                    }
                } else {
                    (
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            page.len()
                        ),
                        &page[..],
                    )
                };
                if sock.write_all(header.as_bytes()).await.is_err() || is_head {
                    let _ = sock.shutdown().await;
                    return;
                }
                let _ = sock.write_all(body).await;
                let _ = sock.shutdown().await;
            });
        }
    });
    url
}

/// Needs yt-dlp, but no internet:
/// `cargo test --release -p rushdownload-engine -- --ignored refuses`
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn page_that_refuses_the_app_still_goes_to_the_site_downloader() {
    let data = payload(2 * 1024 * 1024);
    let dir = temp_dir("refused");
    let engine = engine(&dir);

    // A page link the site only serves to browsers: the video still arrives.
    let url = serve_browsers_only(data.clone(), true, "/watch/clip").await;
    let id = engine.add(new(&url)).unwrap();
    let done = wait_for(&engine, id, |t| t.status == Status::Completed).await;
    println!("refused link downloaded as {}", done.filename);
    assert!(done.media);
    assert_eq!(std::fs::read(&done.path).unwrap(), *data);

    // A refused page with no video on it says both things.
    let url = serve_browsers_only(data.clone(), false, "/posts/some-article").await;
    let id = engine.add(new(&url)).unwrap();
    let mut error = None;
    for _ in 0..600 {
        let t = engine.list().into_iter().find(|t| t.id == id).unwrap();
        if t.status == Status::Failed {
            error = t.error;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let error = error.expect("it fails with a message");
    println!("no video: {error}");
    assert!(error.contains("403") && error.contains("no video"), "{error}");

    // A refused link to a file is not a page: it fails as before.
    let url = serve_browsers_only(data.clone(), true, "/files/setup.zip").await;
    let id = engine.add(new(&url)).unwrap();
    let mut error = None;
    for _ in 0..200 {
        let t = engine.list().into_iter().find(|t| t.id == id).unwrap();
        if t.status == Status::Failed {
            error = t.error;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(error.as_deref(), Some("HTTP 403 Forbidden"));
}

/// A site that hangs up on anything that is not a browser, without a word,
/// and serves browsers a video.
async fn serve_hangs_up_on_strangers(data: Arc<Vec<u8>>, path: &str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}{path}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else { return };
            let data = data.clone();
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut tmp = [0u8; 2048];
                while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    match sock.read(&mut tmp).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => buf.extend_from_slice(&tmp[..n]),
                    }
                }
                let head = String::from_utf8_lossy(&buf).to_ascii_lowercase();
                if !head.lines().any(|l| l.starts_with("user-agent:") && l.contains("mozilla")) {
                    return; // the connection just closes
                }
                let from = head
                    .lines()
                    .find_map(|l| l.strip_prefix("range: bytes="))
                    .and_then(|r| r.trim().split_once('-'))
                    .and_then(|(a, _)| a.parse::<usize>().ok())
                    .filter(|a| *a < data.len());
                let header = match from {
                    Some(a) => format!(
                        "HTTP/1.1 206 Partial Content\r\nContent-Type: video/mp4\r\nAccept-Ranges: bytes\r\nContent-Length: {}\r\nContent-Range: bytes {}-{}/{}\r\nConnection: close\r\n\r\n",
                        data.len() - a, a, data.len() - 1, data.len()
                    ),
                    None => format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: video/mp4\r\nAccept-Ranges: bytes\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        data.len()
                    ),
                };
                if sock.write_all(header.as_bytes()).await.is_err() || head.starts_with("head ") {
                    let _ = sock.shutdown().await;
                    return;
                }
                let _ = sock.write_all(&data[from.unwrap_or(0)..]).await;
                let _ = sock.shutdown().await;
            });
        }
    });
    url
}

/// Needs yt-dlp, but no internet:
/// `cargo test --release -p rushdownload-engine -- --ignored unreachable`
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn unreachable_page_link_still_goes_to_the_site_downloader() {
    let data = payload(2 * 1024 * 1024);
    let dir = temp_dir("unreachable");
    let engine = engine(&dir);

    // A page link whose site will not talk to the app itself.
    let url = serve_hangs_up_on_strangers(data.clone(), "/view_video.php").await;
    let started = std::time::Instant::now();
    let id = engine.add(new(&format!("{url}?viewkey=abc123"))).unwrap();
    let done = wait_for(&engine, id, |t| t.status == Status::Completed).await;
    println!("downloaded as {} after {:.1}s", done.filename, started.elapsed().as_secs_f32());
    assert!(done.media);
    assert_eq!(std::fs::read(&done.path).unwrap(), *data);

    // A link to a file that cannot be reached keeps retrying and fails as before.
    let url = serve_hangs_up_on_strangers(data.clone(), "/files/setup.zip").await;
    let id = engine.add(new(&url)).unwrap();
    let t = wait_for(&engine, id, |t| t.connect_failures >= 2).await;
    assert!(!t.media, "a file link is not handed to the site downloader");
    engine.remove(id, false);
}

/// Needs internet and yt-dlp:
/// `cargo test --release -p rushdownload-engine -- --ignored secure_dns --nocapture`
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn secure_dns_serves_downloads_and_the_site_downloader() {
    let dir = temp_dir("securedns");
    let engine = engine(&dir);
    let mut settings = engine.settings();
    settings.secure_dns = true;
    engine.set_settings(settings);
    assert!(engine.settings().secure_dns);

    // A file on the internet: its name is looked up at Cloudflare.
    let id = engine.add(new("https://www.cloudflare.com/robots.txt")).unwrap();
    let done = wait_for(&engine, id, |t| t.status == Status::Completed).await;
    println!("with secure DNS: {} ({} bytes)", done.filename, done.downloaded);
    assert!(done.downloaded > 0);

    // A name that does not exist fails with the secure lookup's own message.
    let id = engine.add(new("https://no-such-host-rushdownload-test.invalid/file.zip")).unwrap();
    let mut error = None;
    for _ in 0..600 {
        let t = engine.list().into_iter().find(|t| t.id == id).unwrap();
        if t.status == Status::Failed {
            error = t.error;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let error = error.expect("an unknown name fails");
    println!("unknown name: {error}");
    assert!(error.contains("secure DNS"), "{error}");

    // The site downloader runs through the local proxy and still works.
    let data = payload(2 * 1024 * 1024);
    let url = serve_video(data.clone(), Duration::ZERO).await;
    let id = engine
        .add(NewDownload {
            url,
            via_site: true,
            ..Default::default()
        })
        .unwrap();
    let done = wait_for(&engine, id, |t| t.status == Status::Completed).await;
    println!("site download through the proxy: {}", done.filename);
    assert_eq!(std::fs::read(&done.path).unwrap(), *data);
}

/// Needs internet and yt-dlp. Set RUSH_TEST_SITE_URL to a video page:
/// `cargo test --release -p rushdownload-engine -- --ignored site_lookup --nocapture`
/// Looks the page up the way the app does (secure DNS on) and prints what
/// resolutions it offers, or the error. Nothing is downloaded.
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn site_lookup_reports_resolutions_or_the_error() {
    let url = std::env::var("RUSH_TEST_SITE_URL").expect("set RUSH_TEST_SITE_URL");
    let dir = temp_dir("lookup");
    let engine = engine(&dir);
    assert!(engine.settings().secure_dns, "secure DNS is on by default");
    let started = std::time::Instant::now();
    match engine.media_info(&url).await {
        Ok(info) => println!(
            "found after {:.1}s: title of {} characters, resolutions {:?}, site {:?}",
            started.elapsed().as_secs_f32(),
            info.title.chars().count(),
            info.heights,
            info.site
        ),
        Err(e) => println!("lookup failed after {:.1}s: {e}", started.elapsed().as_secs_f32()),
    }
}

/// Needs yt-dlp, but no internet:
/// `cargo test --release -p rushdownload-engine -- --ignored named_site --nocapture`
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn named_site_downloads_in_other_qualities_get_files_of_their_own() {
    let data = payload(2 * 1024 * 1024);
    let url = serve_video(data.clone(), Duration::ZERO).await;
    let dir = temp_dir("namedsite");
    let engine = engine(&dir);
    // The Browser page names a download after its page and asks for a quality.
    let pick = |height: Option<u32>| NewDownload {
        url: url.clone(),
        via_site: true,
        filename: Some("Holiday clip".into()),
        media: MediaOptions {
            max_height: height,
            audio_only: false,
        },
        ..Default::default()
    };
    let mut names = Vec::new();
    for height in [Some(1080), Some(240), Some(240), None] {
        let id = engine.add(pick(height)).unwrap();
        let done = wait_for(&engine, id, |t| t.status == Status::Completed).await;
        assert_eq!(std::fs::read(&done.path).unwrap(), *data);
        names.push(done.filename);
    }
    println!("{names:?}");
    assert_eq!(
        names,
        vec![
            "Holiday clip 1080p.mp4".to_string(),
            "Holiday clip 240p.mp4".to_string(),
            "Holiday clip 240p (2).mp4".to_string(),
            "Holiday clip.mp4".to_string(),
        ],
        "each pick has a file of its own, named after its quality"
    );
    assert_eq!(files_in(&dir.join("out")).len(), 4);

    // A file that is already in the folder is never taken for the download.
    std::fs::write(dir.join("out").join("Other clip 720p.mp4"), b"an older file").unwrap();
    let id = engine
        .add(NewDownload {
            filename: Some("Other clip".into()),
            ..pick(Some(720))
        })
        .unwrap();
    let done = wait_for(&engine, id, |t| t.status == Status::Completed).await;
    assert_eq!(done.filename, "Other clip 720p (2).mp4");
    assert_eq!(std::fs::read(&done.path).unwrap(), *data);
    assert_eq!(std::fs::read(dir.join("out").join("Other clip 720p.mp4")).unwrap(), b"an older file");
}

/// Needs internet, yt-dlp and ffmpeg. Set RUSH_TEST_PLAYLIST to a playlist link:
/// `cargo test --release -p rushdownload-engine -- --ignored playlist_link --nocapture`
/// Fetches the first two videos only, at the smallest picture size.
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn playlist_link_downloads_into_a_folder_named_after_it() {
    let url = std::env::var("RUSH_TEST_PLAYLIST").expect("set RUSH_TEST_PLAYLIST");
    std::env::set_var("RUSHDOWNLOAD_PROFILE_LIMIT", "2");
    let dir = temp_dir("playlist");
    let engine = engine(&dir);
    let id = engine
        .add(NewDownload {
            url,
            media: MediaOptions {
                max_height: Some(144),
                audio_only: false,
            },
            ..Default::default()
        })
        .unwrap();
    let first = engine.list().into_iter().find(|t| t.id == id).unwrap();
    assert!(first.profile, "a playlist link is fetched whole");

    let mut done = None;
    for _ in 0..3000 {
        let t = engine.list().into_iter().find(|t| t.id == id).unwrap();
        assert_ne!(t.status, Status::Failed, "failed: {:?}", t.error);
        if t.status == Status::Completed {
            done = Some(t);
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let done = done.expect("the playlist finished");
    let files = files_in(&done.path);
    println!("folder name is {} characters long and holds {} file(s), {} bytes", done.filename.chars().count(), files.len(), done.downloaded);
    for name in &files {
        println!("  starts {:?}, ends {:?}", name.chars().take(6).collect::<String>(), name.rsplit('.').next().unwrap_or(""));
    }
    assert!(done.path.is_dir());
    assert!(!done.filename.contains("YouTube playlist"), "the folder is named after the playlist itself");
    assert_eq!(done.items, 2, "the two videos asked for");
    assert_eq!(files.len(), 2, "videos only, no leftovers: {files:?}");
    assert!(files[0].starts_with("001 - ") && files[1].starts_with("002 - "), "in playlist order: {files:?}");
    engine.remove(id, true);
}

/// Needs internet and yt-dlp. Set RUSH_TEST_PLAYLIST to a playlist link:
/// `cargo test --release -p rushdownload-engine -- --ignored playlist_lookup --nocapture`
/// Looks the playlist up the way the Home card does. Nothing is downloaded.
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn playlist_lookup_gives_its_name_picture_and_size() {
    let url = std::env::var("RUSH_TEST_PLAYLIST").expect("set RUSH_TEST_PLAYLIST");
    let dir = temp_dir("playlistinfo");
    let engine = engine(&dir);
    let started = std::time::Instant::now();
    let info = engine.playlist_info(&url).await.expect("the playlist can be read");
    println!(
        "after {:.1}s: title of {} characters, {} videos, {} photos, more {}, {} seconds in all, picture {}, channel {}",
        started.elapsed().as_secs_f32(),
        info.title.chars().count(),
        info.count,
        info.photos,
        info.more,
        info.duration.map_or("unknown".to_string(), |d| d.to_string()),
        if info.thumbnail.as_deref().is_some_and(|t| t.starts_with("https://")) { "yes" } else { "no" },
        if info.channel.is_some() { "yes" } else { "no" },
    );
    println!("videos to check for resolutions: {}", info.samples.len());
    assert!(!info.title.is_empty() && info.count + info.photos > 0);
}

/// Needs internet, yt-dlp and gallery-dl:
/// `cargo test --release -p rushdownload-engine -- --ignored profile_card --nocapture`
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn profile_card_has_the_profiles_name_and_picture() {
    let dir = temp_dir("profilecard");
    let engine = engine(&dir);
    // A photo-and-video profile: counted by gallery-dl, which also says whose it is.
    let found = engine.playlist_info("https://bsky.app/profile/bsky.app").await.unwrap();
    println!("profile: {found:?}");
    assert_eq!(found.title, "Bluesky");
    assert!(found.thumbnail.is_some_and(|p| p.contains("avatar")));
    assert!(found.count + found.photos > 0);
    // A channel: its listing has its name and its own picture.
    let found = engine.playlist_info("https://www.youtube.com/@GyanPlayZZ").await.unwrap();
    println!("channel: {} | {:?} | {} videos", found.title, found.thumbnail, found.count);
    assert!(!found.title.is_empty());
    assert!(found.thumbnail.is_some());
}

/// Needs internet, yt-dlp and gallery-dl. Downloads two files from each of
/// two collections:
/// `RUSHDOWNLOAD_PROFILE_LIMIT=2 cargo test --release -p rushdownload-engine -- --ignored any_site --nocapture`
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn a_collection_on_any_site_is_found_and_downloaded() {
    let dir = temp_dir("anysite");
    let engine = engine(&dir);
    // What the programs say each link holds.
    let cases = [
        ("https://www.dailymotion.com/france24", Some(Fetcher::Ytdlp)),
        ("https://soundcloud.com/forss", Some(Fetcher::Ytdlp)),
        ("https://www.artstation.com/sungchoi", Some(Fetcher::Gallerydl)),
        // a channel VK would not list without a sign-in: still a collection, so the user is told
        ("https://vkvideo.ru/@shishonin_official", Some(Fetcher::Ytdlp)),
        // one video, also opened from inside a playlist, stays one video
        ("https://www.youtube.com/watch?v=mbM45GyHhhk", None),
        ("https://www.youtube.com/watch?v=mbM45GyHhhk&list=PLSqWva9Yx97U", None),
        // a page that is nobody's collection, and a plain file
        ("https://www.rust-lang.org/learn", None),
        ("https://www.cloudflare.com/robots.txt", None),
    ];
    for (link, expected) in cases {
        let began = std::time::Instant::now();
        let found = engine.collection_of(link).await;
        println!("{link}: {found:?} in {:.1}s", began.elapsed().as_secs_f64());
        assert_eq!(found.as_ref().and_then(|f| f.by), expected, "{link}");
        // a found collection gets the profile card's details too
        if found.is_some() {
            let face = engine.profile_face(link).await;
            println!("  card: {:?} | picture {}", face.name, face.picture.is_some());
            assert!(face.name.is_some() || face.picture.is_some(), "{link}: nothing for the card");
        }
        // asked once: the second time is answered from what was said
        let again = std::time::Instant::now();
        assert_eq!(engine.collection_of(link).await, found);
        assert!(again.elapsed() < Duration::from_millis(50));
    }
    // A sound collection and a picture collection, each downloaded into a folder of its own.
    for link in ["https://soundcloud.com/forss", "https://www.artstation.com/sungchoi"] {
        let id = engine.add(new(link)).unwrap();
        let began = std::time::Instant::now();
        let done = loop {
            let t = engine.list().into_iter().find(|t| t.id == id).unwrap();
            if matches!(t.status, Status::Completed | Status::Failed) || began.elapsed() > Duration::from_secs(240) {
                break t;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        };
        println!(
            "{link}: {:?} {:?} | folder {} | {} files, {} bytes | picture {:?}",
            done.status, done.error, done.filename, done.items, done.downloaded, done.thumbnail.as_deref().map(|p| &p[..p.len().min(40)])
        );
        assert_eq!(done.status, Status::Completed, "{:?}", done.error);
        assert!(done.profile && done.items >= 1);
        assert!(done.path.is_dir());
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Minutes after midnight, `offset` minutes from the time now, on the clock.
fn clock_in(offset: i64) -> u16 {
    use chrono::Timelike;
    let now = chrono::Local::now();
    let minutes = i64::from(now.hour() * 60 + now.minute()) + offset;
    minutes.rem_euclid(24 * 60) as u16
}

#[tokio::test(flavor = "multi_thread")]
async fn a_schedule_holds_its_download_and_survives_a_restart() {
    use rushdownload_engine::{Repeat, ScheduleSpec, ScheduleStatus};
    let data = payload(8 * 1024 * 1024);
    let server = serve(data.clone(), true, Duration::from_millis(15)).await;
    let dir = temp_dir("schedule");
    let engine = engine(&dir);

    let id = engine.add(new(&server.url)).unwrap();
    wait_for(&engine, id, |t| t.status == Status::Downloading).await;

    // Every day, starting two hours from now: the download waits until then.
    let later = ScheduleSpec {
        folder: None,
        name: "Tonight".into(),
        repeat: Repeat::Weekly { days: 0x7f },
        start: clock_in(120),
        stop: None,
        downloads: vec![id, 999],
    };
    let sid = engine.save_schedule(None, later).unwrap();
    wait_for(&engine, id, |t| t.status == Status::Paused).await;
    let info = engine.schedules().into_iter().find(|s| s.id == sid).unwrap();
    assert_eq!(info.status, ScheduleStatus::Waiting);
    // a download that does not exist is left out
    assert_eq!(info.spec.downloads, vec![id]);
    let next = info.next_start.unwrap();
    let now = chrono::Local::now().timestamp();
    assert!(next > now + 100 * 60 && next <= now + 121 * 60, "next start {next}, now {now}");

    // A restart keeps the schedule, and does not start the held download.
    tokio::time::sleep(Duration::from_millis(200)).await;
    drop(engine);
    let engine = self::engine(&dir);
    let kept = engine.schedules();
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].spec.name, "Tonight");
    tokio::time::sleep(Duration::from_millis(500)).await;
    let t = engine.list().into_iter().find(|t| t.id == id).unwrap();
    assert_eq!(t.status, Status::Paused);

    // Changed to a window that is open now: it starts at once.
    let open = ScheduleSpec {
        folder: None,
        name: "Now".into(),
        repeat: Repeat::Weekly { days: 0x7f },
        start: clock_in(-1),
        stop: Some(clock_in(30)),
        downloads: vec![id],
    };
    engine.save_schedule(Some(sid), open).unwrap();
    let done = wait_for(&engine, id, |t| t.status == Status::Completed).await;
    assert_eq!(std::fs::read(&done.path).unwrap(), *data);
    assert_eq!(engine.schedules()[0].status, ScheduleStatus::Active);

    // Paused, it is shown so; deleted, it is gone.
    engine.set_schedule_enabled(sid, false);
    assert_eq!(engine.schedules()[0].status, ScheduleStatus::Paused);
    engine.delete_schedule(sid);
    assert!(engine.schedules().is_empty());
    drop(engine);
    assert!(self::engine(&dir).schedules().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_download_keeps_to_its_own_limit_and_takes_a_new_one_while_running() {
    let data = payload(6 * 1024 * 1024);
    let server = serve(data.clone(), true, Duration::ZERO).await;
    let dir = temp_dir("own-limit");
    let engine = engine(&dir);

    // 512 KB/s of its own, with no global limit
    let id = engine
        .add(NewDownload { speed_limit: Some(512 * 1024), ..new(&server.url) })
        .unwrap();
    let started = wait_for(&engine, id, |t| t.status == Status::Downloading).await;
    assert_eq!(started.speed_limit, 512 * 1024);
    tokio::time::sleep(Duration::from_secs(2)).await;
    let t = engine.list().into_iter().find(|t| t.id == id).unwrap();
    // two seconds at 512 KB/s, and half a second of starting credit per connection pool
    assert!(t.downloaded < 1700 * 1024, "{} bytes in 2 s", t.downloaded);
    assert!(t.downloaded > 300 * 1024, "{} bytes in 2 s", t.downloaded);

    // lifted while it runs: the rest comes at once
    engine.set_speed_limit(id, 0);
    let done = wait_for(&engine, id, |t| t.status == Status::Completed).await;
    assert_eq!(done.speed_limit, 0);
    assert_eq!(std::fs::read(&done.path).unwrap(), *data);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_global_limit_still_caps_a_download_with_a_higher_own_limit() {
    let data = payload(6 * 1024 * 1024);
    let server = serve(data, true, Duration::ZERO).await;
    let dir = temp_dir("global-cap");
    let engine = engine(&dir);
    let mut settings = engine.settings();
    settings.speed_limit = 512 * 1024;
    engine.set_settings(settings);

    let id = engine
        .add(NewDownload { speed_limit: Some(50 * 1024 * 1024), ..new(&server.url) })
        .unwrap();
    wait_for(&engine, id, |t| t.status == Status::Downloading).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let t = engine.list().into_iter().find(|t| t.id == id).unwrap();
    assert!(t.downloaded < 1700 * 1024, "{} bytes in 2 s under a 512 KB/s global limit", t.downloaded);
    engine.pause(id);
}

/// Needs internet and a .torrent file: set RUSH_TEST_TORRENT to its path, then
/// `cargo test --release -p rushdownload-engine --test download -- --ignored --nocapture torrent_takes_a_new_limit`
/// A limit set while the torrent runs holds its peers and web seeds together.
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn torrent_takes_a_new_limit_while_running() {
    let source = std::env::var("RUSH_TEST_TORRENT").expect("set RUSH_TEST_TORRENT");
    let dir = temp_dir("torrent-limit");
    let engine = engine(&dir);
    let id = engine.add(new(&source)).unwrap();

    let speed = |engine: &Engine| engine.list().into_iter().find(|t| t.id == id).map_or(0, |t| t.speed);
    let average = |engine: Engine, secs: u64| async move {
        let mut sum = 0u64;
        let n = secs * 2;
        for _ in 0..n {
            tokio::time::sleep(Duration::from_millis(500)).await;
            sum += speed(&engine);
        }
        sum / n
    };

    // running, unlimited
    for _ in 0..1200 {
        if speed(&engine) > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let free = average(engine.clone(), 8).await;
    println!("unlimited: {:.2} MB/s", free as f64 / 1048576.0);

    engine.set_speed_limit(id, 1024 * 1024);
    // requests already sent to peers still arrive; the limit holds after those
    for n in 0..6 {
        let a = average(engine.clone(), 5).await;
        println!("{}-{} s after the limit: {:.2} MB/s", n * 5, n * 5 + 5, a as f64 / 1048576.0);
    }
    let held = average(engine.clone(), 8).await;
    println!("limited to 1 MB/s: {:.2} MB/s", held as f64 / 1048576.0);
    engine.remove(id, true);
    assert!(held < 1024 * 1024 * 13 / 10, "{held} bytes/s under a 1 MB/s limit");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_link_keeps_the_partial_file_only_for_the_same_file() {
    use rushdownload_engine::LinkProblem;
    let data = payload(8 * 1024 * 1024);
    let first = serve(data.clone(), true, Duration::from_millis(15)).await;
    let dir = temp_dir("new-link");
    let engine = engine(&dir);

    // part downloaded from the first link, then paused (the link "expired")
    let id = engine.add(new(&first.url)).unwrap();
    wait_for(&engine, id, |t| t.downloaded > 1024 * 1024).await;
    engine.pause(id);
    let paused = wait_for(&engine, id, |t| t.status == Status::Paused && t.active_connections == 0).await;
    tokio::time::sleep(Duration::from_millis(300)).await;

    // the same file elsewhere: it carries on
    let same = serve(data.clone(), true, Duration::ZERO).await;
    let check = engine.check_new_link(id, &same.url, None).await.unwrap();
    assert!(check.resume, "{check:?}");
    assert!(check.downloaded >= paused.downloaded / 2, "{check:?}");

    // another file of the same size: its bytes differ from what is on disk
    let other: Arc<Vec<u8>> = Arc::new(data.iter().map(|b| b ^ 0x5a).collect());
    let different = serve(other.clone(), true, Duration::ZERO).await;
    let check = engine.check_new_link(id, &different.url, None).await.unwrap();
    assert_eq!((check.resume, check.problem), (false, Some(LinkProblem::ContentDiffers)));
    // and it is not taken without starting over
    assert!(engine.use_new_link(id, &different.url, None, false).await.is_err());

    // another size
    let shorter = serve(payload(5 * 1024 * 1024), true, Duration::ZERO).await;
    let check = engine.check_new_link(id, &shorter.url, None).await.unwrap();
    assert_eq!(check.problem, Some(LinkProblem::SizeDiffers));

    // no byte ranges
    let whole = serve(data.clone(), false, Duration::ZERO).await;
    let check = engine.check_new_link(id, &whole.url, None).await.unwrap();
    assert_eq!(check.problem, Some(LinkProblem::NoRanges));

    // not a link at all
    assert!(engine.check_new_link(id, "ftp://example.com/x", None).await.is_err());

    // the same file: taken, and finished from where it was, correctly
    let before = same.ranged.load(Relaxed);
    engine.use_new_link(id, &same.url, None, false).await.unwrap();
    let done = wait_for(&engine, id, |t| t.status == Status::Completed).await;
    assert_eq!(done.url, same.url);
    assert_eq!(std::fs::read(&done.path).unwrap(), *data);
    assert!(same.ranged.load(Relaxed) > before);
    assert!(done.path.ends_with("test file.bin"), "the name stays: {:?}", done.path);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_link_to_another_file_starts_over() {
    let data = payload(6 * 1024 * 1024);
    let first = serve(data.clone(), true, Duration::from_millis(15)).await;
    let dir = temp_dir("new-link-restart");
    let engine = engine(&dir);
    let id = engine.add(new(&first.url)).unwrap();
    wait_for(&engine, id, |t| t.downloaded > 1024 * 1024).await;
    engine.pause(id);
    wait_for(&engine, id, |t| t.status == Status::Paused && t.active_connections == 0).await;
    tokio::time::sleep(Duration::from_millis(300)).await;

    let other: Arc<Vec<u8>> = Arc::new(data.iter().map(|b| b ^ 0x33).collect());
    let different = serve(other.clone(), true, Duration::ZERO).await;
    let check = engine.use_new_link(id, &different.url, None, true).await.unwrap();
    assert!(!check.resume);
    let done = wait_for(&engine, id, |t| t.status == Status::Completed).await;
    // all of it from the new link: nothing of the old file is left in it
    assert_eq!(std::fs::read(&done.path).unwrap(), *other);
}

/// Waits until `pred` holds for the whole list (a batch and its downloads).
async fn wait_list(engine: &Engine, pred: impl Fn(&[TaskInfo]) -> bool) -> Vec<TaskInfo> {
    for _ in 0..900 {
        let list = engine.list();
        if pred(&list) {
            return list;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("timed out: {:#?}", engine.list());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_batch_downloads_every_link_into_its_own_folder_as_one_download() {
    let data = payload(3 * 1024 * 1024);
    let server = serve(data.clone(), true, Duration::from_millis(4)).await;
    let base = server.url.trim_end_matches("test%20file.bin").to_string();
    let dir = temp_dir("batch");
    let engine = engine(&dir);

    // a name with characters no folder may have; a link twice; one that cannot be reached
    let links = vec![
        format!("{base}one.bin"),
        format!("{base}two.bin?token=1"),
        format!("{base}one.bin"),
        "http://127.0.0.1:1/gone.zip".to_string(),
    ];
    let id = engine.add_batch("My: Batch?", links, None, MediaOptions::default()).unwrap();
    let folder = dir.join("out").join("My_ Batch_");
    assert!(folder.is_dir(), "the folder is made before anything downloads");

    let list = engine.list();
    let parent = list.iter().find(|t| t.id == id).unwrap();
    assert_eq!(parent.batch.as_ref().unwrap().count, 3, "the repeated link is downloaded once");
    assert_eq!(parent.path, folder);
    let children: Vec<&TaskInfo> = list.iter().filter(|t| t.parent == Some(id)).collect();
    assert_eq!(children.len(), 3);
    assert!(children.iter().all(|c| c.dir == folder));

    // the batch ends once every download has: the unreachable one fails alone
    let list = wait_list(&engine, |l| {
        l.iter().find(|t| t.id == id).is_some_and(|p| matches!(p.status, Status::Completed | Status::Failed))
    })
    .await;
    let parent = list.iter().find(|t| t.id == id).unwrap();
    let info = parent.batch.clone().unwrap();
    assert_eq!((info.completed, info.failed), (2, 1), "{parent:?}");
    assert_eq!(parent.status, Status::Completed, "completed with errors");
    assert_eq!(parent.error.as_deref(), Some("1 of 3 links did not download"));
    assert_eq!(parent.downloaded, 2 * data.len() as u64);
    assert_eq!(parent.total, Some(2 * data.len() as u64));
    assert!(parent.finished_at.is_some());
    assert_eq!(std::fs::read(folder.join("one.bin")).unwrap(), *data);
    assert_eq!(std::fs::read(folder.join("two.bin")).unwrap(), *data);

    // a restart finds the batch and its downloads as they were, and makes no second one
    tokio::time::sleep(Duration::from_millis(200)).await;
    drop(engine);
    let engine = self::engine(&dir);
    let list = engine.list();
    assert_eq!(list.iter().filter(|t| t.batch.is_some()).count(), 1);
    assert_eq!(list.iter().filter(|t| t.parent == Some(id)).count(), 3);
    assert_eq!(list.iter().find(|t| t.id == id).unwrap().batch.as_ref().unwrap().completed, 2);

    // removing the batch takes its downloads along and leaves the files that finished
    engine.remove(id, false);
    assert!(engine.list().is_empty());
    assert!(folder.join("one.bin").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_batch_pauses_resumes_and_cancels_as_one() {
    let data = payload(6 * 1024 * 1024);
    let server = serve(data.clone(), true, Duration::from_millis(20)).await;
    let base = server.url.trim_end_matches("test%20file.bin").to_string();
    let dir = temp_dir("batch-pause");
    let engine = engine(&dir);
    let links = (1..=4).map(|n| format!("{base}file{n}.bin")).collect();
    let id = engine.add_batch("Pausing", links, None, MediaOptions::default()).unwrap();

    wait_list(&engine, |l| l.iter().any(|t| t.parent == Some(id) && t.downloaded > 512 * 1024)).await;
    engine.pause(id);
    let list = wait_list(&engine, |l| {
        l.iter().filter(|t| t.parent == Some(id)).all(|t| t.status == Status::Paused && t.active_connections == 0)
    })
    .await;
    assert_eq!(list.iter().find(|t| t.id == id).unwrap().status, Status::Paused);
    // one download cancelled on its own: the rest go on
    let first = list.iter().find(|t| t.parent == Some(id)).unwrap().id;
    engine.cancel(first);
    engine.resume(id);
    let list = wait_list(&engine, |l| l.iter().find(|t| t.id == id).is_some_and(|p| p.status == Status::Completed)).await;
    let info = list.iter().find(|t| t.id == id).unwrap().batch.clone().unwrap();
    assert_eq!((info.completed, info.cancelled), (3, 1));
    let cancelled = list.iter().find(|t| t.id == first).unwrap();
    assert!(cancelled.cancelled && cancelled.status == Status::Failed && cancelled.downloaded == 0);

    // and it can be fetched after all
    engine.retry_failed(id);
    let list = wait_list(&engine, |l| {
        l.iter().find(|t| t.id == id).is_some_and(|p| p.batch.as_ref().unwrap().completed == 4)
    })
    .await;
    let parent = list.iter().find(|t| t.id == id).unwrap();
    assert_eq!((parent.status, parent.error.as_deref()), (Status::Completed, None));
    for n in 1..=4 {
        assert_eq!(std::fs::read(dir.join("out/Pausing").join(format!("file{n}.bin"))).unwrap(), *data);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn finished_files_are_sorted_by_kind_unless_given_a_folder() {
    let data = payload(512 * 1024);
    let server = serve(data.clone(), true, Duration::ZERO).await;
    let base = server.url.trim_end_matches("test%20file.bin").to_string();
    let dir = temp_dir("organize");
    // sorting is on for a new install
    let engine = Engine::new(dir.join("state"), Settings::new(dir.join("out"))).unwrap();
    assert!(engine.settings().organize.enabled);
    let out = dir.join("out");

    // a document goes to Documents; a file already there is not written over
    std::fs::create_dir_all(out.join("Documents")).unwrap();
    std::fs::write(out.join("Documents/report.pdf"), b"old").unwrap();
    let id = engine.add(new(&format!("{base}report.pdf"))).unwrap();
    let done = wait_for(&engine, id, |t| t.status == Status::Completed).await;
    assert_eq!(done.path, out.join("Documents/report (1).pdf"));
    assert_eq!(std::fs::read(&done.path).unwrap(), *data);
    assert_eq!(std::fs::read(out.join("Documents/report.pdf")).unwrap(), b"old");
    // an unknown kind goes to Other
    let id = engine.add(new(&format!("{base}blob.bin"))).unwrap();
    assert_eq!(wait_for(&engine, id, |t| t.status == Status::Completed).await.path, out.join("Other/blob.bin"));

    // a folder chosen for the download wins
    let mine = dir.join("mine");
    let id = engine.add(NewDownload { dir: Some(mine.clone()), ..new(&format!("{base}clip.mp4")) }).unwrap();
    assert_eq!(wait_for(&engine, id, |t| t.status == Status::Completed).await.path, mine.join("clip.mp4"));

    // custom category, and moving a finished file
    let mut settings = engine.settings();
    settings.organize.categories.insert(
        0,
        rushdownload_engine::FileCategory { name: "Books".into(), folder: "Books".into(), extensions: vec!["EPUB".into()] },
    );
    engine.set_settings(settings);
    let id = engine.add(new(&format!("{base}novel.epub"))).unwrap();
    assert_eq!(wait_for(&engine, id, |t| t.status == Status::Completed).await.path, out.join("Books/novel.epub"));
    let moved = engine.move_to(id, mine.clone()).await.unwrap();
    assert_eq!(moved, mine.join("novel.epub"));
    assert_eq!(engine.list().into_iter().find(|t| t.id == id).unwrap().path, moved);

    // sorting off: it stays in the download folder
    let mut settings = engine.settings();
    settings.organize.enabled = false;
    // never download helper programs during tests
    settings.auto_update_tools = false;
    engine.set_settings(settings);
    let id = engine.add(new(&format!("{base}song.mp3"))).unwrap();
    assert_eq!(wait_for(&engine, id, |t| t.status == Status::Completed).await.path, out.join("song.mp3"));
}
