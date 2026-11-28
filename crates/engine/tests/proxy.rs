//! Proxies, end to end: downloads, the app's own look-ups and the gateway
//! the helper programs use all go through the proxy chosen, sign in to it,
//! and fail rather than connect directly when it cannot be used.
//!
//! The proxies here are small SOCKS5 and HTTP proxies written for the test.
//! They connect every request to the local file server, whatever site it
//! names, so the downloads use made-up names (files.test) that only a proxy
//! can reach: one that slipped past the proxy would fail.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine as _;
use rushdownload_engine::{
    Engine, MemoryStore, NewDownload, PasswordChange, ProxyChoice, ProxyKind, ProxyProfile, Settings, Status, TaskInfo,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const BODY_LEN: usize = 300_000;

fn body() -> Vec<u8> {
    (0..BODY_LEN).map(|i| (i % 251) as u8).collect()
}

/// A file server answering every request with the whole body.
async fn file_server() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let data = Arc::new(body());
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else { return };
            let data = data.clone();
            tokio::spawn(async move {
                let mut head = Vec::new();
                let mut chunk = [0u8; 2048];
                while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                    match sock.read(&mut chunk).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => head.extend_from_slice(&chunk[..n]),
                    }
                }
                let answer = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/octet-stream\r\nConnection: close\r\n\r\n",
                    data.len()
                );
                let _ = sock.write_all(answer.as_bytes()).await;
                let _ = sock.write_all(&data).await;
            });
        }
    });
    port
}

/// What a test proxy saw.
#[derive(Clone, Default)]
struct Seen {
    /// Connections it carried, after signing in.
    carried: Arc<AtomicUsize>,
    /// Sign-ins it turned down.
    refused: Arc<AtomicUsize>,
    /// The sites asked for.
    hosts: Arc<Mutex<Vec<String>>>,
}

/// A SOCKS5 proxy asking for `user` and `pass`, taking site names (as
/// socks5h sends them), connecting everything to `target_port` here.
async fn socks5_proxy(user: &'static str, pass: &'static str, target_port: u16) -> (u16, Seen) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Seen::default();
    let out = seen.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else { return };
            let seen = seen.clone();
            tokio::spawn(async move {
                let mut b = [0u8; 2];
                sock.read_exact(&mut b).await.ok()?;
                let mut methods = vec![0u8; b[1] as usize];
                sock.read_exact(&mut methods).await.ok()?;
                if !methods.contains(&2) {
                    sock.write_all(&[5, 0xff]).await.ok()?;
                    return None;
                }
                sock.write_all(&[5, 2]).await.ok()?;
                let mut v = [0u8; 2];
                sock.read_exact(&mut v).await.ok()?;
                let mut u = vec![0u8; v[1] as usize];
                sock.read_exact(&mut u).await.ok()?;
                let mut l = [0u8; 1];
                sock.read_exact(&mut l).await.ok()?;
                let mut p = vec![0u8; l[0] as usize];
                sock.read_exact(&mut p).await.ok()?;
                if u != user.as_bytes() || p != pass.as_bytes() {
                    seen.refused.fetch_add(1, Relaxed);
                    sock.write_all(&[1, 1]).await.ok()?;
                    return None;
                }
                sock.write_all(&[1, 0]).await.ok()?;
                let mut req = [0u8; 4];
                sock.read_exact(&mut req).await.ok()?;
                let host = match req[3] {
                    1 => {
                        let mut ip = [0u8; 4];
                        sock.read_exact(&mut ip).await.ok()?;
                        std::net::Ipv4Addr::from(ip).to_string()
                    }
                    3 => {
                        let mut n = [0u8; 1];
                        sock.read_exact(&mut n).await.ok()?;
                        let mut name = vec![0u8; n[0] as usize];
                        sock.read_exact(&mut name).await.ok()?;
                        String::from_utf8(name).ok()?
                    }
                    _ => {
                        let mut ip = [0u8; 16];
                        sock.read_exact(&mut ip).await.ok()?;
                        std::net::Ipv6Addr::from(ip).to_string()
                    }
                };
                let mut port = [0u8; 2];
                sock.read_exact(&mut port).await.ok()?;
                seen.hosts.lock().unwrap().push(host);
                let mut server = TcpStream::connect(("127.0.0.1", target_port)).await.ok()?;
                sock.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]).await.ok()?;
                seen.carried.fetch_add(1, Relaxed);
                tokio::io::copy_bidirectional(&mut sock, &mut server).await.ok()?;
                Some(())
            });
        }
    });
    (port, out)
}

/// An HTTP proxy asking for `user` and `pass` (Basic), connecting
/// everything to `target_port` here. Takes plain requests and tunnels.
async fn http_proxy(user: &'static str, pass: &'static str, target_port: u16) -> (u16, Seen) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Seen::default();
    let out = seen.clone();
    let expected = format!("basic {}", base64::engine::general_purpose::STANDARD.encode(format!("{user}:{pass}"))).to_ascii_lowercase();
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else { return };
            let (seen, expected) = (seen.clone(), expected.clone());
            tokio::spawn(async move {
                let mut head = Vec::new();
                let mut chunk = [0u8; 2048];
                let end = loop {
                    if let Some(at) = head.windows(4).position(|w| w == b"\r\n\r\n") {
                        break at;
                    }
                    let n = sock.read(&mut chunk).await.ok()?;
                    if n == 0 {
                        return None;
                    }
                    head.extend_from_slice(&chunk[..n]);
                };
                let text = String::from_utf8_lossy(&head[..end]).to_string();
                let auth_ok = text
                    .lines()
                    .any(|l| l.to_ascii_lowercase().strip_prefix("proxy-authorization:").is_some_and(|v| v.trim() == expected));
                if !auth_ok {
                    seen.refused.fetch_add(1, Relaxed);
                    sock.write_all(b"HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm=\"t\"\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.ok()?;
                    return None;
                }
                let first = text.lines().next()?.to_string();
                let mut parts = first.split_whitespace();
                let method = parts.next()?.to_string();
                let to = parts.next()?.to_string();
                let mut server = TcpStream::connect(("127.0.0.1", target_port)).await.ok()?;
                if method == "CONNECT" {
                    seen.hosts.lock().unwrap().push(to.rsplit_once(':')?.0.to_string());
                    sock.write_all(b"HTTP/1.1 200 Connection established\r\n\r\n").await.ok()?;
                } else {
                    let url = reqwest::Url::parse(&to).ok()?;
                    seen.hosts.lock().unwrap().push(url.host_str()?.to_string());
                    server.write_all(format!("{method} {} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n", url.path()).as_bytes()).await.ok()?;
                }
                seen.carried.fetch_add(1, Relaxed);
                tokio::io::copy_bidirectional(&mut sock, &mut server).await.ok()?;
                Some(())
            });
        }
    });
    (port, out)
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rushdownload-proxy-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn engine(dir: &PathBuf) -> Engine {
    let mut settings = Settings::new(dir.join("out"));
    settings.organize.enabled = false;
    settings.auto_update_tools = false;
    // Names are looked up the system's way: no requests to Cloudflare.
    settings.secure_dns = false;
    Engine::with_secrets(dir.join("state"), settings, Arc::new(MemoryStore::default())).unwrap()
}

fn profile(kind: ProxyKind, port: u16, user: &str) -> ProxyProfile {
    ProxyProfile {
        id: 0,
        name: "Test proxy".into(),
        kind,
        host: "127.0.0.1".into(),
        port,
        username: user.into(),
        has_password: false,
        bypass: vec![],
    }
}

fn set(pass: &str) -> PasswordChange {
    PasswordChange::Set(pass.into())
}

async fn settle(engine: &Engine, id: u64) -> TaskInfo {
    for _ in 0..400 {
        if let Some(t) = engine.list().into_iter().find(|t| t.id == id) {
            if matches!(t.status, Status::Completed | Status::Failed) {
                return t;
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("timed out: {:?}", engine.list());
}

fn add(engine: &Engine, url: &str, proxy: ProxyChoice) -> u64 {
    engine.add(NewDownload { url: url.into(), proxy, ..Default::default() }).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn downloads_go_through_the_default_socks5_proxy_and_sign_in() {
    let files = file_server().await;
    let (proxy_port, seen) = socks5_proxy("me", "s3cret:/@", files).await;
    let dir = temp_dir("socks");
    let engine = engine(&dir);
    let id = engine.save_proxy(profile(ProxyKind::Socks5h, proxy_port, "me"), set("s3cret:/@")).unwrap();
    engine.set_default_proxy(Some(id)).unwrap();

    let task = settle(&engine, add(&engine, &format!("http://files.test:{files}/a.bin"), ProxyChoice::Default)).await;
    assert_eq!(task.status, Status::Completed, "{:?}", task.error);
    assert_eq!(std::fs::read(&task.path).unwrap(), body());
    assert!(seen.carried.load(Relaxed) >= 1);
    assert!(seen.hosts.lock().unwrap().iter().all(|h| h == "files.test"), "the proxy looked the name up");

    // the password is in the secret store, never in the app's files
    let saved = std::fs::read_to_string(dir.join("state").join("downloads.json")).unwrap();
    assert!(saved.contains("\"has_password\":true"));
    assert!(!saved.contains("s3cret"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_refused_sign_in_fails_the_download_instead_of_going_direct() {
    let files = file_server().await;
    let (proxy_port, seen) = socks5_proxy("me", "right", files).await;
    let dir = temp_dir("refused");
    let engine = engine(&dir);
    let id = engine.save_proxy(profile(ProxyKind::Socks5h, proxy_port, "me"), set("wrong")).unwrap();
    // 127.0.0.1 is always reached directly; a made-up name needs the proxy
    let task = settle(&engine, add(&engine, &format!("http://files.test:{files}/a.bin"), ProxyChoice::Profile(id))).await;
    assert_eq!(task.status, Status::Failed);
    assert!(seen.refused.load(Relaxed) >= 1);
    assert_eq!(seen.carried.load(Relaxed), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_direct_download_skips_the_default_proxy() {
    let files = file_server().await;
    let (proxy_port, seen) = socks5_proxy("me", "pw", files).await;
    let dir = temp_dir("direct");
    let engine = engine(&dir);
    let id = engine.save_proxy(profile(ProxyKind::Socks5h, proxy_port, "me"), set("pw")).unwrap();
    engine.set_default_proxy(Some(id)).unwrap();
    // without the proxy the made-up name cannot be found
    let task = settle(&engine, add(&engine, &format!("http://files.test:{files}/a.bin"), ProxyChoice::Direct)).await;
    assert_eq!(task.status, Status::Failed);
    assert_eq!(seen.carried.load(Relaxed) + seen.refused.load(Relaxed), 0, "the proxy was never asked");
    // and a real direct download works as before
    let task = settle(&engine, add(&engine, &format!("http://127.0.0.1:{files}/b.bin"), ProxyChoice::Direct)).await;
    assert_eq!(task.status, Status::Completed, "{:?}", task.error);
}

#[tokio::test(flavor = "multi_thread")]
async fn http_proxies_sign_in_with_basic_auth() {
    let files = file_server().await;
    let (proxy_port, seen) = http_proxy("me", "pw", files).await;
    let dir = temp_dir("http");
    let engine = engine(&dir);
    let id = engine.save_proxy(profile(ProxyKind::Http, proxy_port, "me"), set("pw")).unwrap();
    let task = settle(&engine, add(&engine, &format!("http://files.test:{files}/a.bin"), ProxyChoice::Profile(id))).await;
    assert_eq!(task.status, Status::Completed, "{:?}", task.error);
    assert_eq!(std::fs::read(&task.path).unwrap(), body());
    assert!(seen.carried.load(Relaxed) >= 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn helper_programs_reach_the_proxy_through_a_gateway_that_wants_its_key() {
    let files = file_server().await;
    let (socks_port, socks_seen) = socks5_proxy("me", "pw", files).await;
    let (http_port, http_seen) = http_proxy("me", "pw", files).await;
    let dir = temp_dir("gateway");
    let engine = engine(&dir);
    for (kind, port, seen) in [(ProxyKind::Socks5h, socks_port, &socks_seen), (ProxyKind::Http, http_port, &http_seen)] {
        let id = engine.save_proxy(profile(kind, port, "me"), set("pw")).unwrap();
        let gateway = engine.helper_proxy(&ProxyChoice::Profile(id)).unwrap().expect("a gateway");
        assert!(gateway.starts_with("http://rush:"), "{gateway}");
        assert!(!gateway.contains("pw@"), "the proxy's password is not on the command line");
        // the same proxy gets the same gateway
        assert_eq!(engine.helper_proxy(&ProxyChoice::Profile(id)).unwrap().unwrap(), gateway);

        let through = reqwest::Client::builder().proxy(reqwest::Proxy::all(&gateway).unwrap()).build().unwrap();
        let got = through.get(format!("http://files.test:{files}/x.bin")).send().await.unwrap();
        assert_eq!(got.status(), 200, "{kind:?}");
        assert_eq!(got.bytes().await.unwrap().len(), BODY_LEN);
        assert!(seen.carried.load(Relaxed) >= 1, "{kind:?}");

        // without the key the gateway lets nothing through
        let url = reqwest::Url::parse(&gateway).unwrap();
        let keyless = format!("http://{}:{}", url.host_str().unwrap(), url.port().unwrap());
        let stranger = reqwest::Client::builder().proxy(reqwest::Proxy::all(&keyless).unwrap()).build().unwrap();
        let refused = stranger.get(format!("http://files.test:{files}/x.bin")).send().await.unwrap();
        assert_eq!(refused.status(), 407);
    }
    // a direct choice gives helpers no proxy (secure DNS is off here)
    assert_eq!(engine.helper_proxy(&ProxyChoice::Direct).unwrap(), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn testing_a_proxy_says_whether_it_works_and_why_not() {
    let files = file_server().await;
    let (proxy_port, _) = socks5_proxy("me", "pw", files).await;
    let dir = temp_dir("test");
    let engine = engine(&dir);
    let url = Some(format!("http://files.test:{files}/check"));
    let ok = engine.test_proxy(profile(ProxyKind::Socks5h, proxy_port, "me"), set("pw"), url.clone()).await;
    assert!(ok.ok, "{:?}", ok.error);
    assert_eq!(ok.status, Some(200));

    let bad = engine.test_proxy(profile(ProxyKind::Socks5h, proxy_port, "me"), set("nope"), url.clone()).await;
    assert!(!bad.ok);
    assert!(bad.error.as_deref().unwrap().contains("user name and password"), "{:?}", bad.error);

    // a saved proxy is tested with its saved password
    let id = engine.save_proxy(profile(ProxyKind::Socks5h, proxy_port, "me"), set("pw")).unwrap();
    let mut saved = profile(ProxyKind::Socks5h, proxy_port, "me");
    saved.id = id;
    assert!(engine.test_proxy(saved, PasswordChange::Keep, url.clone()).await.ok);

    let closed = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let closed_port = closed.local_addr().unwrap().port();
    drop(closed);
    let nobody = engine.test_proxy(profile(ProxyKind::Http, closed_port, ""), PasswordChange::Keep, url).await;
    assert!(nobody.error.as_deref().unwrap().contains("Nothing answered"), "{:?}", nobody.error);

    let invalid = engine.test_proxy(profile(ProxyKind::Http, 0, ""), PasswordChange::Keep, None).await;
    assert!(!invalid.ok && invalid.error.is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_proxy_in_use_cannot_be_deleted_and_the_default_follows_deletion() {
    let dir = temp_dir("delete");
    let engine = engine(&dir);
    let id = engine.save_proxy(profile(ProxyKind::Socks5, 9, ""), PasswordChange::Keep).unwrap();
    let other = engine.save_proxy(profile(ProxyKind::Http, 9, ""), PasswordChange::Keep).unwrap();
    engine.set_default_proxy(Some(other)).unwrap();
    let task = add(&engine, "http://files.test:9/a.bin", ProxyChoice::Profile(id));
    engine.pause(task);
    assert!(engine.delete_proxy(id).unwrap_err().contains("download"));
    engine.remove(task, false);
    engine.delete_proxy(id).unwrap();
    assert!(engine.add(NewDownload { url: "http://a.test/x".into(), proxy: ProxyChoice::Profile(id), ..Default::default() }).is_err());

    engine.delete_proxy(other).unwrap();
    assert_eq!(engine.proxies().default, None, "downloads on the default now connect directly");
    assert!(engine.proxies().items.is_empty());

    // what was saved comes back after a restart
    let id = engine.save_proxy(profile(ProxyKind::Socks5h, 1080, "me"), set("pw")).unwrap();
    engine.set_default_proxy(Some(id)).unwrap();
    drop(engine);
    let again = Engine::with_secrets(dir.join("state"), Settings::new(dir.join("out")), Arc::new(MemoryStore::default())).unwrap();
    let book = again.proxies();
    assert_eq!(book.default, Some(id));
    assert_eq!(book.items[0].username, "me");
    assert!(book.items[0].has_password);
}
