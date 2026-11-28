//! Request options, end to end: headers, cookies and sign-ins reach the
//! server; a redirect to another site takes only harmless headers along;
//! a Digest challenge is answered; private values stay out of the list.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rushdownload_engine::{AuthKind, Engine, MemoryStore, NewDownload, PasswordChange, RequestOptions, Settings, Status, TaskInfo};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const BODY: &[u8] = b"the file's contents";

/// One request as the server saw it: the host asked for, the path, headers.
#[derive(Clone, Debug)]
struct Seen {
    host: String,
    path: String,
    headers: HashMap<String, String>,
}

fn md5_hex(text: &str) -> String {
    use md5::Digest;
    md5::Md5::digest(text.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

/// Whether a Digest answer is right for user "me", password "pw".
fn digest_ok(header: &str, method: &str) -> bool {
    let field = |name: &str| -> Option<String> {
        let at = header.find(&format!("{name}="))? + name.len() + 1;
        let rest = &header[at..];
        Some(if let Some(quoted) = rest.strip_prefix('"') {
            quoted[..quoted.find('"')?].to_string()
        } else {
            rest.split(',').next()?.trim().to_string()
        })
    };
    let (Some(user), Some(realm), Some(nonce), Some(uri), Some(response)) =
        (field("username"), field("realm"), field("nonce"), field("uri"), field("response"))
    else {
        return false;
    };
    let ha1 = md5_hex(&format!("{user}:{realm}:pw"));
    let ha2 = md5_hex(&format!("{method}:{uri}"));
    let expected = match (field("qop"), field("nc"), field("cnonce")) {
        (Some(qop), Some(nc), Some(cnonce)) => md5_hex(&format!("{ha1}:{nonce}:{nc}:{cnonce}:{qop}:{ha2}")),
        _ => md5_hex(&format!("{ha1}:{nonce}:{ha2}")),
    };
    user == "me" && nonce == "n0nce" && response == expected
}

/// Serves `/plain/…` (the file), `/away/…` (sent on to the same server under
/// the name "localhost", another site to the client) and `/digest/…` (the
/// file, behind a Digest sign-in). Every request is written down.
async fn server() -> (u16, Arc<Mutex<Vec<Seen>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let log = Arc::new(Mutex::new(Vec::new()));
    let seen = log.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else { return };
            let seen = seen.clone();
            tokio::spawn(async move {
                let mut head = Vec::new();
                let mut chunk = [0u8; 2048];
                while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                    match sock.read(&mut chunk).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => head.extend_from_slice(&chunk[..n]),
                    }
                }
                let text = String::from_utf8_lossy(&head).to_string();
                let mut lines = text.split("\r\n");
                let first = lines.next().unwrap_or_default().to_string();
                let path = first.split_whitespace().nth(1).unwrap_or("/").to_string();
                let headers: HashMap<String, String> = lines
                    .filter_map(|l| l.split_once(':'))
                    .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
                    .collect();
                let host = headers.get("host").cloned().unwrap_or_default();
                seen.lock().unwrap().push(Seen { host, path: path.clone(), headers: headers.clone() });
                let answer = if path.starts_with("/away/") {
                    format!("HTTP/1.1 302 Found\r\nLocation: http://localhost:{port}/plain/elsewhere.bin\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                } else if path.starts_with("/digest/")
                    && !headers.get("authorization").is_some_and(|a| a.starts_with("Digest ") && digest_ok(a, "GET"))
                {
                    "HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Digest realm=\"files\", qop=\"auth\", nonce=\"n0nce\", opaque=\"0paque\"\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string()
                } else {
                    format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", BODY.len())
                };
                let _ = sock.write_all(answer.as_bytes()).await;
                if answer.starts_with("HTTP/1.1 200") {
                    let _ = sock.write_all(BODY).await;
                }
            });
        }
    });
    (port, log)
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rushdownload-request-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn engine(dir: &PathBuf) -> Engine {
    let mut settings = Settings::new(dir.join("out"));
    settings.organize.enabled = false;
    settings.auto_update_tools = false;
    settings.secure_dns = false;
    Engine::with_secrets(dir.join("state"), settings, Arc::new(MemoryStore::default())).unwrap()
}

async fn done(engine: &Engine, id: u64) -> TaskInfo {
    for _ in 0..400 {
        if let Some(t) = engine.list().into_iter().find(|t| t.id == id) {
            if matches!(t.status, Status::Completed | Status::Failed) {
                return t;
            }
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("timed out: {:?}", engine.list());
}

fn options() -> RequestOptions {
    RequestOptions {
        user_agent: Some("TestAgent/1.0".into()),
        referer: Some("https://page.test/where".into()),
        headers: vec![("X-Plain".into(), "visible".into()), ("X-Api-Key".into(), "s3cret-key".into())],
        cookies: Some("session=c00kie".into()),
        auth: AuthKind::Basic,
        username: "me".into(),
        password: Some("pw".into()),
        ..Default::default()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_users_headers_cookies_and_sign_in_reach_the_server_and_stay_private() {
    let (port, log) = server().await;
    let dir = temp_dir("own");
    let engine = engine(&dir);
    let id = engine
        .add(NewDownload { url: format!("http://127.0.0.1:{port}/plain/a.bin"), request: Some(options()), ..Default::default() })
        .unwrap();
    let t = done(&engine, id).await;
    assert_eq!(t.status, Status::Completed, "{:?}", t.error);
    assert_eq!(std::fs::read(&t.path).unwrap(), BODY);
    let seen = log.lock().unwrap().clone();
    assert!(!seen.is_empty());
    for s in &seen {
        assert_eq!(s.headers.get("user-agent").map(String::as_str), Some("TestAgent/1.0"));
        assert_eq!(s.headers.get("referer").map(String::as_str), Some("https://page.test/where"));
        assert_eq!(s.headers.get("cookie").map(String::as_str), Some("session=c00kie"));
        assert_eq!(s.headers.get("x-api-key").map(String::as_str), Some("s3cret-key"));
        assert_eq!(s.headers.get("authorization").map(String::as_str), Some("Basic bWU6cHc="));
    }

    // what is shown and saved has no secret in it
    let shown = t.request.unwrap();
    assert!(shown.has_password && shown.has_cookies && shown.private_headers == vec!["X-Api-Key"]);
    let saved = std::fs::read_to_string(dir.join("state").join("downloads.json")).unwrap();
    for secret in ["c00kie", "s3cret-key", "bWU6cHc=", "\"pw\""] {
        assert!(!saved.contains(secret), "{secret} leaked into downloads.json");
    }
    assert!(dir.join("state").join("secrets.vault").exists());

    // a header the app sets itself cannot be given
    let bad = RequestOptions { headers: vec![("Range".into(), "bytes=0-".into())], ..Default::default() };
    assert!(engine.add(NewDownload { url: format!("http://127.0.0.1:{port}/plain/b.bin"), request: Some(bad), ..Default::default() }).is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_redirect_to_another_site_takes_only_harmless_headers() {
    let (port, log) = server().await;
    let dir = temp_dir("away");
    let engine = engine(&dir);
    let id = engine
        .add(NewDownload { url: format!("http://127.0.0.1:{port}/away/a.bin"), request: Some(options()), ..Default::default() })
        .unwrap();
    let t = done(&engine, id).await;
    assert_eq!(t.status, Status::Completed, "{:?}", t.error);
    let seen = log.lock().unwrap().clone();
    let home: Vec<&Seen> = seen.iter().filter(|s| s.host.starts_with("127.0.0.1")).collect();
    let away: Vec<&Seen> = seen.iter().filter(|s| s.host.starts_with("localhost")).collect();
    assert!(!home.is_empty() && !away.is_empty(), "{seen:?}");
    assert!(home.iter().all(|s| s.headers.contains_key("cookie")), "the first site gets everything");
    for s in away {
        assert_eq!(s.path, "/plain/elsewhere.bin");
        assert_eq!(s.headers.get("user-agent").map(String::as_str), Some("TestAgent/1.0"), "{s:?}");
        for private in ["cookie", "authorization", "x-api-key", "x-plain"] {
            assert!(!s.headers.contains_key(private), "{private} followed to another site: {s:?}");
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_digest_challenge_is_answered() {
    let (port, log) = server().await;
    let dir = temp_dir("digest");
    let engine = engine(&dir);
    let digest = RequestOptions { auth: AuthKind::Digest, username: "me".into(), password: Some("pw".into()), ..Default::default() };
    let id = engine
        .add(NewDownload { url: format!("http://127.0.0.1:{port}/digest/a.bin"), request: Some(digest), ..Default::default() })
        .unwrap();
    let t = done(&engine, id).await;
    assert_eq!(t.status, Status::Completed, "{:?}", t.error);
    assert_eq!(std::fs::read(&t.path).unwrap(), BODY);
    assert!(log.lock().unwrap().iter().any(|s| s.headers.get("authorization").is_some_and(|a| a.starts_with("Digest "))));

    // the wrong password is refused, and the download says so
    let wrong = RequestOptions { auth: AuthKind::Digest, username: "me".into(), password: Some("nope".into()), ..Default::default() };
    let id = engine
        .add(NewDownload { url: format!("http://127.0.0.1:{port}/digest/b.bin"), request: Some(wrong), ..Default::default() })
        .unwrap();
    let t = done(&engine, id).await;
    assert_eq!(t.status, Status::Failed);
    assert!(t.error.unwrap().contains("401"));
}

#[tokio::test(flavor = "multi_thread")]
async fn profiles_and_the_default_profile_apply_and_keep_their_secrets() {
    let (port, log) = server().await;
    let dir = temp_dir("profiles");
    let engine = engine(&dir);
    let token = RequestOptions { auth: AuthKind::Bearer, user_agent: Some("ProfileAgent".into()), ..Default::default() };
    let pid = engine.save_request_profile(0, "API", token, PasswordChange::Set("t0ken".into()), PasswordChange::Keep).unwrap();
    assert!(engine.request_profiles().items[0].options.has_password);

    // chosen for one download
    let id = engine
        .add(NewDownload { url: format!("http://127.0.0.1:{port}/plain/p.bin"), request_profile: Some(pid), ..Default::default() })
        .unwrap();
    assert_eq!(done(&engine, id).await.status, Status::Completed);
    assert!(log.lock().unwrap().iter().all(|s| s.headers.get("authorization").map(String::as_str) == Some("Bearer t0ken")));

    // renamed without retyping the token: it is kept
    let again = engine.request_profiles().items[0].options.clone();
    engine.save_request_profile(pid, "API (renamed)", again, PasswordChange::Keep, PasswordChange::Keep).unwrap();

    // as the default, for a download with nothing of its own
    engine.set_default_request_profile(Some(pid)).unwrap();
    log.lock().unwrap().clear();
    let id = engine.add(NewDownload { url: format!("http://127.0.0.1:{port}/plain/d.bin"), ..Default::default() }).unwrap();
    assert_eq!(done(&engine, id).await.status, Status::Completed);
    assert!(log.lock().unwrap().iter().all(|s| s.headers.get("user-agent").map(String::as_str) == Some("ProfileAgent")
        && s.headers.get("authorization").map(String::as_str) == Some("Bearer t0ken")));

    // a profile with no token is refused; one in use cannot be deleted
    let empty = RequestOptions { auth: AuthKind::Bearer, ..Default::default() };
    assert!(engine.save_request_profile(0, "Empty", empty, PasswordChange::Keep, PasswordChange::Keep).is_err());
    let busy = engine
        .add(NewDownload { url: format!("http://127.0.0.1:{port}/away/never.bin"), request_profile: Some(pid), ..Default::default() })
        .unwrap();
    engine.pause(busy);
    assert!(engine.delete_request_profile(pid).is_err());
    engine.remove(busy, false);
    engine.delete_request_profile(pid).unwrap();
    assert_eq!(engine.request_profiles().default, None);
    let saved = std::fs::read_to_string(dir.join("state").join("downloads.json")).unwrap();
    assert!(!saved.contains("t0ken"));
}
