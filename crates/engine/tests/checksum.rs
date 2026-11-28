//! Checksums of finished downloads, end to end against a local server.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use rushdownload_engine::{CheckState, Engine, HashAlgo, MediaOptions, MemoryStore, NewDownload, Settings, Status, TaskInfo};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Serves `body` at any path, whole (no ranges), as `name`.
async fn serve(body: &'static [u8]) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else { return };
            tokio::spawn(async move {
                let mut head = Vec::new();
                let mut chunk = [0u8; 1024];
                while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                    match sock.read(&mut chunk).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => head.extend_from_slice(&chunk[..n]),
                    }
                }
                let answer = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                let _ = sock.write_all(answer.as_bytes()).await;
                let _ = sock.write_all(body).await;
            });
        }
    });
    base
}

const BODY: &[u8] = b"abc";
const SHA256: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
const MD5: &str = "900150983cd24fb0d6963f7d28e17f72";

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rushdownload-checksum-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn engine(dir: &PathBuf, auto: Option<HashAlgo>) -> Engine {
    let mut settings = Settings::new(dir.join("out"));
    settings.organize.enabled = false;
    settings.auto_update_tools = false;
    settings.secure_dns = false;
    settings.auto_checksum = auto;
    Engine::with_secrets(dir.join("state"), settings, Arc::new(MemoryStore::default())).unwrap()
}

/// Waits until the download has finished and its checksum, if any, settled.
async fn settled(engine: &Engine, id: u64) -> TaskInfo {
    for _ in 0..400 {
        if let Some(t) = engine.list().into_iter().find(|t| t.id == id) {
            let check_done = t.checksum.as_ref().is_none_or(|c| c.state != CheckState::Calculating);
            if t.status == Status::Failed || (t.status == Status::Completed && check_done) {
                return t;
            }
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("timed out: {:?}", engine.list());
}

fn add(engine: &Engine, url: &str, checksum: Option<&str>) -> Result<u64, String> {
    engine.add(NewDownload { url: url.into(), checksum: checksum.map(String::from), ..Default::default() })
}

#[tokio::test(flavor = "multi_thread")]
async fn a_given_checksum_is_checked_once_the_file_is_in() {
    let base = serve(BODY).await;
    let dir = temp_dir("given");
    let engine = engine(&dir, None);

    let ok = settled(&engine, add(&engine, &format!("{base}/good.bin"), Some(&format!("SHA-256 {SHA256}"))).unwrap()).await;
    let c = ok.checksum.unwrap();
    assert_eq!((c.state, c.algorithm), (CheckState::Verified, HashAlgo::Sha256));
    assert_eq!(c.actual.as_deref(), Some(SHA256));

    // a wrong one: the file stays, the check says it failed, with both sums
    let wrong = "0".repeat(64);
    let bad = settled(&engine, add(&engine, &format!("{base}/bad.bin"), Some(&wrong)).unwrap()).await;
    assert_eq!(bad.status, Status::Completed, "the file did download");
    let c = bad.checksum.unwrap();
    assert_eq!(c.state, CheckState::Failed);
    assert_eq!((c.expected.as_deref(), c.actual.as_deref()), (Some(wrong.as_str()), Some(SHA256)));

    // nonsense is refused when the download is added
    assert!(add(&engine, &format!("{base}/x.bin"), Some("not a sum")).is_err());

    // without one, nothing is worked out (Settings ask for none)
    let plain = settled(&engine, add(&engine, &format!("{base}/plain.bin"), None).unwrap()).await;
    assert!(plain.checksum.is_none());

    // checked later, with a sum given afterwards; then just worked out to see
    engine.verify_checksum(plain.id, Some(MD5.into()), None).unwrap();
    let later = settled(&engine, plain.id).await.checksum.unwrap();
    assert_eq!((later.state, later.algorithm), (CheckState::Verified, HashAlgo::Md5));
    engine.verify_checksum(plain.id, None, Some(HashAlgo::Sha256)).unwrap();
    let seen = settled(&engine, plain.id).await.checksum.unwrap();
    assert_eq!((seen.state, seen.actual.as_deref()), (CheckState::NotVerified, Some(SHA256)));

    // a file deleted since cannot be checked, and says so
    std::fs::remove_file(&plain.path).unwrap();
    engine.verify_checksum(plain.id, Some(SHA256.into()), None).unwrap();
    let gone = settled(&engine, plain.id).await.checksum.unwrap();
    assert_eq!(gone.state, CheckState::Error);
    assert!(gone.error.unwrap().contains("no longer"));
}

#[tokio::test(flavor = "multi_thread")]
async fn settings_can_ask_for_every_files_checksum() {
    let base = serve(BODY).await;
    let dir = temp_dir("auto");
    let engine = engine(&dir, Some(HashAlgo::Sha256));
    let t = settled(&engine, add(&engine, &format!("{base}/a.bin"), None).unwrap()).await;
    let c = t.checksum.unwrap();
    assert_eq!((c.state, c.actual.as_deref()), (CheckState::NotVerified, Some(SHA256)), "worked out, nothing to compare with");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_batch_is_checked_against_a_sums_file() {
    let base = serve(BODY).await;
    let dir = temp_dir("batch");
    let engine = engine(&dir, None);
    let links = vec![format!("{base}/one.bin"), format!("{base}/two.bin"), format!("{base}/three.bin")];
    let batch = engine.add_batch("Sums", links, None, MediaOptions::default()).unwrap();
    for _ in 0..400 {
        let list = engine.list();
        if list.iter().filter(|t| t.parent == Some(batch)).all(|t| t.status == Status::Completed) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let sums = format!("{SHA256}  one.bin\n{}  two.bin\n", "f".repeat(64));
    assert_eq!(engine.verify_with_sums(batch, &sums).unwrap(), 2, "three.bin has no sum");
    let children: Vec<TaskInfo> = engine.list().into_iter().filter(|t| t.parent == Some(batch)).collect();
    for child in children {
        let t = settled(&engine, child.id).await;
        let state = t.checksum.map(|c| c.state);
        match t.filename.as_str() {
            "one.bin" => assert_eq!(state, Some(CheckState::Verified)),
            "two.bin" => assert_eq!(state, Some(CheckState::Failed)),
            _ => assert_eq!(state, None),
        }
    }
    assert!(engine.verify_with_sums(batch, "nothing here").is_err());
}
