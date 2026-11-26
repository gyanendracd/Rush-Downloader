//! Deep scan against a small local website.

use std::time::Duration;

use rushdownload_engine::{Engine, FoundKind, NewDownload, Settings, Status};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const CLIP: &[u8] = b"not really a video, but served as one";

/// `/page` embeds a video, links a second one, mentions a stream inside a
/// script and frames `/frame`, which holds one more video.
async fn serve_site() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let root = origin.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else { return };
            let root = root.clone();
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut tmp = [0u8; 2048];
                while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    match sock.read(&mut tmp).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => buf.extend_from_slice(&tmp[..n]),
                    }
                }
                let head = String::from_utf8_lossy(&buf).to_string();
                let path = head.split_whitespace().nth(1).unwrap_or("/").to_string();
                let range = head
                    .to_ascii_lowercase()
                    .lines()
                    .find_map(|l| l.strip_prefix("range: bytes=").map(str::to_string))
                    .and_then(|r| {
                        let (a, b) = r.trim().split_once('-')?;
                        let a: usize = a.parse().ok()?;
                        let b: usize = b.parse().unwrap_or(CLIP.len() - 1);
                        Some((a, b.min(CLIP.len() - 1)))
                    });
                let escaped = root.replace('/', "\\/");
                let (status, ctype, extra, body): (&str, &str, String, Vec<u8>) = match path.as_str() {
                    "/page" => (
                        "200 OK",
                        "text/html; charset=utf-8",
                        String::new(),
                        format!(
                            r#"<html><body>
                            <video src="/media/clip.mp4"></video>
                            <a href="media/second%20part.mkv">part two</a>
                            <script>player.setup({{"file":"{escaped}\/hls\/master.m3u8"}});</script>
                            <iframe src="/frame"></iframe>
                            <a href="/other.html">another page</a>
                            </body></html>"#
                        )
                        .into_bytes(),
                    ),
                    "/frame" => (
                        "200 OK",
                        "text/html",
                        String::new(),
                        br#"<source src="/media/inner.webm">"#.to_vec(),
                    ),
                    p if p.starts_with("/media/") => match range {
                        Some((a, b)) => (
                            "206 Partial Content",
                            "video/mp4",
                            format!("Content-Range: bytes {a}-{b}/{}\r\n", CLIP.len()),
                            CLIP[a..=b].to_vec(),
                        ),
                        None => ("200 OK", "video/mp4", String::new(), CLIP.to_vec()),
                    },
                    _ => ("404 Not Found", "text/plain", String::new(), b"nope".to_vec()),
                };
                let header = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\n{extra}Connection: close\r\n\r\n",
                    body.len()
                );
                let _ = sock.write_all(header.as_bytes()).await;
                let _ = sock.write_all(&body).await;
                let _ = sock.shutdown().await;
            });
        }
    });
    origin
}

#[tokio::test(flavor = "multi_thread")]
async fn scan_lists_media_and_found_items_download() {
    let origin = serve_site().await;
    let dir = std::env::temp_dir().join(format!("rushdownload-scan-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let engine = Engine::new(dir.join("state"), Settings::new(dir.join("out"))).unwrap();

    let found = engine.scan_page(&format!("{origin}/page")).await.unwrap();
    let summary: Vec<(FoundKind, &str, Option<u64>)> = found
        .iter()
        .map(|f| (f.kind, f.name.as_str(), f.size))
        .collect();
    let size = Some(CLIP.len() as u64);
    assert!(summary.contains(&(FoundKind::Video, "clip.mp4", size)), "{summary:?}");
    assert!(summary.contains(&(FoundKind::Video, "second part.mkv", size)), "{summary:?}");
    assert!(summary.contains(&(FoundKind::Video, "inner.webm", size)), "{summary:?}");
    assert!(summary.contains(&(FoundKind::Stream, "master.m3u8", None)), "{summary:?}");
    assert_eq!(
        found.iter().filter(|f| f.kind == FoundKind::Embed).count(),
        1,
        "{summary:?}"
    );
    assert_eq!(found.len(), 5, "{summary:?}");

    // The video inside the frame names the frame as the page it came from.
    let inner = found.iter().find(|f| f.name == "inner.webm").unwrap();
    assert_eq!(inner.referer, format!("{origin}/frame"));

    // A found item downloads like any other link.
    let clip = found.iter().find(|f| f.name == "clip.mp4").unwrap();
    let id = engine
        .add(NewDownload {
            url: clip.url.clone(),
            headers: vec![("Referer".into(), clip.referer.clone())],
            ..Default::default()
        })
        .unwrap();
    for _ in 0..200 {
        let t = engine.list().into_iter().find(|t| t.id == id).unwrap();
        if t.status == Status::Completed {
            assert_eq!(std::fs::read(&t.path).unwrap(), CLIP);
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        assert_ne!(t.status, Status::Failed, "{:?}", t.error);
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("download did not finish");
}

#[tokio::test(flavor = "multi_thread")]
async fn scan_reports_pages_that_cannot_be_opened() {
    let origin = serve_site().await;
    let dir = std::env::temp_dir().join(format!("rushdownload-scan-err-{}", std::process::id()));
    let engine = Engine::new(dir.join("state"), Settings::new(dir.join("out"))).unwrap();
    let err = engine.scan_page(&format!("{origin}/missing")).await.unwrap_err();
    assert!(err.contains("404"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}
