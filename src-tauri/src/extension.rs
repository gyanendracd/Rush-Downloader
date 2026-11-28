//! Where the browser extension hands over links and the downloads it
//! catches.
//!
//! The app listens on this computer only (127.0.0.1), and lets nothing in
//! on trust. An extension pairs once: it asks, the app shows who is asking
//! and the user allows it or not; an allowed extension gets a random key
//! that every later request must carry (`Authorization: Bearer <key>`). The
//! app keeps only a hash of each key, in `paired-extensions.json`, and a
//! pairing can be revoked under Settings. A request to add links must also
//! come from the same extension (the `Origin` browsers set) and be
//! addressed to this computer by name, and every link is checked to be a
//! plain web address before the engine sees it.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rushdownload_engine::{Engine, NewDownload, RequestOptions};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

/// The port the extension looks for the app on.
pub const PORT: u16 = 38951;

const MAX_HEAD: usize = 16 * 1024;
const MAX_BODY: usize = 1024 * 1024;
const MAX_ITEMS: usize = 100;
/// How long a request to pair waits for the user.
const PAIR_WAIT: Duration = Duration::from_secs(180);
/// Requests to pair waiting at once, at most.
const MAX_PENDING: usize = 3;

#[derive(Debug, PartialEq)]
struct Request {
    method: String,
    path: String,
    origin: String,
    host: String,
    /// The key from `Authorization: Bearer <key>`, if sent.
    key: Option<String>,
    body: Vec<u8>,
}

/// Reads one HTTP request. Anything oversized or malformed is dropped.
fn read_request(stream: &mut impl Read) -> Option<Request> {
    let mut data = Vec::new();
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        if let Some(at) = data.windows(4).position(|w| w == b"\r\n\r\n") {
            break at;
        }
        if data.len() > MAX_HEAD {
            return None;
        }
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            return None;
        }
        data.extend_from_slice(&chunk[..n]);
    };
    let head = std::str::from_utf8(&data[..head_end]).ok()?;
    let mut lines = head.split("\r\n");
    let mut first = lines.next()?.split(' ');
    let method = first.next()?.to_string();
    let path = first.next()?.split('?').next()?.to_string();
    let (mut origin, mut host, mut length, mut key) = (String::new(), String::new(), 0usize, None);
    for line in lines {
        let Some((name, value)) = line.split_once(':') else { continue };
        let value = value.trim();
        match name.trim().to_ascii_lowercase().as_str() {
            "origin" => origin = value.to_string(),
            "host" => host = value.to_string(),
            "content-length" => length = value.parse().ok()?,
            "authorization" => {
                key = value.strip_prefix("Bearer ").map(|k| k.trim().to_string()).filter(|k| !k.is_empty());
            }
            _ => {}
        }
    }
    if length > MAX_BODY {
        return None;
    }
    let mut body = data[head_end + 4..].to_vec();
    while body.len() < length {
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            return None;
        }
        body.extend_from_slice(&chunk[..n]);
    }
    body.truncate(length);
    Some(Request { method, path, origin, host, key, body })
}

/// Whether a request was made by a browser extension rather than a web page.
fn from_extension(origin: &str) -> bool {
    ["chrome-extension://", "moz-extension://", "safari-web-extension://"]
        .iter()
        .any(|scheme| origin.len() > scheme.len() && origin.starts_with(scheme))
}

/// Whether the request was addressed to this computer by its own name. A
/// page that points a site name of its own at 127.0.0.1 fails this.
fn addressed_here(host: &str, port: u16) -> bool {
    host == format!("127.0.0.1:{port}") || host == format!("localhost:{port}")
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn random_hex(len: usize) -> String {
    use ring::rand::SecureRandom;
    let mut bytes = vec![0u8; len];
    ring::rand::SystemRandom::new().fill(&mut bytes).expect("the system's random numbers");
    hex(&bytes)
}

fn hash(key: &str) -> String {
    hex(ring::digest::digest(&ring::digest::SHA256, key.as_bytes()).as_ref())
}

/// Compares without stopping at the first difference.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// An extension allowed to send downloads.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Paired {
    id: String,
    origin: String,
    /// What the extension says it runs in ("Chrome 131", "Firefox 133").
    browser: String,
    created_at: u64,
    #[serde(default)]
    last_seen: Option<u64>,
    /// SHA-256 of its key; the key itself is not kept.
    key_sha256: String,
}

/// A paired extension, for display.
#[derive(Clone, Debug, Serialize)]
pub struct PairedInfo {
    pub id: String,
    pub origin: String,
    pub browser: String,
    pub created_at: u64,
    pub last_seen: Option<u64>,
}

/// A request to pair, shown to the user.
#[derive(Clone, Debug, Serialize)]
pub struct PairRequest {
    pub id: String,
    pub origin: String,
    pub browser: String,
}

#[derive(Debug)]
enum Answer {
    Waiting,
    Allowed(String),
    Denied,
}

struct Pending {
    origin: String,
    browser: String,
    asked: Instant,
    answer: Answer,
}

/// The paired extensions and the requests to pair. Managed by Tauri.
pub struct Bridge {
    file: PathBuf,
    paired: Mutex<Vec<Paired>>,
    pending: Mutex<HashMap<String, Pending>>,
}

impl Bridge {
    pub fn load(state_dir: &Path) -> Bridge {
        let file = state_dir.join("paired-extensions.json");
        let paired = std::fs::read(&file).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        Bridge { file, paired: Mutex::new(paired), pending: Mutex::new(HashMap::new()) }
    }

    fn save(&self) {
        let Ok(bytes) = serde_json::to_vec_pretty(&*self.paired.lock().unwrap()) else { return };
        let tmp = self.file.with_extension("json.tmp");
        if std::fs::write(&tmp, bytes).is_ok() {
            let _ = std::fs::rename(&tmp, &self.file);
        }
    }

    pub fn list(&self) -> Vec<PairedInfo> {
        self.paired
            .lock()
            .unwrap()
            .iter()
            .map(|p| PairedInfo { id: p.id.clone(), origin: p.origin.clone(), browser: p.browser.clone(), created_at: p.created_at, last_seen: p.last_seen })
            .collect()
    }

    pub fn revoke(&self, id: &str) {
        self.paired.lock().unwrap().retain(|p| p.id != id);
        self.save();
        tracing::info!(id, "browser extension pairing revoked");
    }

    /// Whether `key` belongs to the extension at `origin`; notes it was seen.
    fn check(&self, origin: &str, key: Option<&str>) -> bool {
        let Some(key) = key else { return false };
        let wanted = hash(key);
        let mut paired = self.paired.lock().unwrap();
        let Some(p) = paired.iter_mut().find(|p| same(&p.key_sha256, &wanted) && p.origin == origin) else {
            return false;
        };
        // written down at most once a minute
        let stamp = now();
        if p.last_seen.is_none_or(|seen| stamp.saturating_sub(seen) >= 60) {
            p.last_seen = Some(stamp);
            drop(paired);
            self.save();
        }
        true
    }

    fn forget_stale(&self) {
        self.pending.lock().unwrap().retain(|_, p| p.asked.elapsed() < PAIR_WAIT);
    }

    /// Starts a request to pair; `None` while too many are waiting.
    fn ask(&self, origin: &str, browser: &str) -> Option<PairRequest> {
        self.forget_stale();
        let mut pending = self.pending.lock().unwrap();
        if pending.values().filter(|p| matches!(p.answer, Answer::Waiting)).count() >= MAX_PENDING {
            return None;
        }
        let id = random_hex(12);
        pending.insert(id.clone(), Pending { origin: origin.into(), browser: browser.into(), asked: Instant::now(), answer: Answer::Waiting });
        Some(PairRequest { id, origin: origin.into(), browser: browser.into() })
    }

    /// The user's answer to a request to pair.
    pub fn answer(&self, id: &str, allow: bool) -> Result<(), String> {
        self.forget_stale();
        let mut pending = self.pending.lock().unwrap();
        let p = pending.get_mut(id).ok_or("That request to connect has expired; connect from the extension again")?;
        if !matches!(p.answer, Answer::Waiting) {
            return Ok(());
        }
        if !allow {
            p.answer = Answer::Denied;
            tracing::info!(origin = %p.origin, "browser extension pairing denied");
            return Ok(());
        }
        let key = random_hex(32);
        {
            let mut paired = self.paired.lock().unwrap();
            // the same extension again replaces its old pairing
            paired.retain(|x| x.origin != p.origin);
            paired.push(Paired {
                id: random_hex(8),
                origin: p.origin.clone(),
                browser: p.browser.clone(),
                created_at: now(),
                last_seen: None,
                key_sha256: hash(&key),
            });
        }
        self.save();
        tracing::info!(origin = %p.origin, browser = %p.browser, "browser extension paired");
        p.answer = Answer::Allowed(key);
        Ok(())
    }

    /// Where a request to pair stands, for the extension that made it. An
    /// answer is given once, then the request is gone.
    fn poll(&self, id: &str, origin: &str) -> serde_json::Value {
        self.forget_stale();
        let mut pending = self.pending.lock().unwrap();
        let Some(p) = pending.get(id).filter(|p| p.origin == origin) else {
            return serde_json::json!({ "state": "expired" });
        };
        let answer = match &p.answer {
            Answer::Waiting => return serde_json::json!({ "state": "pending" }),
            Answer::Denied => serde_json::json!({ "state": "denied" }),
            Answer::Allowed(key) => serde_json::json!({ "state": "approved", "key": key }),
        };
        pending.remove(id);
        answer
    }
}

/// One thing the extension sent.
#[derive(Debug)]
enum Sent {
    /// A page for the app to look up itself; it lands in the Home box.
    Page(String),
    /// A video or audio file, a stream, or a download the browser started.
    File(NewDownload),
}

fn web_address(value: &serde_json::Value, key: &str) -> Option<String> {
    let text = value.get(key)?.as_str()?.trim();
    let url = tauri::Url::parse(text).ok()?;
    (matches!(url.scheme(), "http" | "https") && url.host_str().is_some() && text.len() <= 4000).then(|| text.to_string())
}

fn plain_text(value: &serde_json::Value, key: &str, max: usize) -> Option<String> {
    value
        .get(key)
        .and_then(|v| v.as_str())
        .map(|t| t.chars().filter(|c| !c.is_control()).take(max).collect::<String>())
        .filter(|t| !t.trim().is_empty())
}

/// Reads what the extension sent. Only plain web addresses get through;
/// anything else in the list is left out.
fn sent_items(body: &[u8]) -> Result<Vec<Sent>, String> {
    let value: serde_json::Value = serde_json::from_slice(body).map_err(|_| "That request could not be read".to_string())?;
    let list = value.get("items").and_then(|i| i.as_array()).ok_or("That request holds no links")?;
    let agent = value
        .get("agent")
        .and_then(|a| a.as_str())
        .filter(|a| !a.is_empty() && a.len() < 400 && !a.chars().any(char::is_control));
    let mut items = Vec::new();
    for entry in list.iter().take(MAX_ITEMS) {
        let Some(url) = web_address(entry, "url") else { continue };
        let kind = entry.get("kind").and_then(|k| k.as_str()).unwrap_or("video");
        if kind == "page" {
            items.push(Sent::Page(url));
            continue;
        }
        // Streams and embedded players go to the site downloader.
        let stream = matches!(kind, "stream" | "embed");
        let mut headers = Vec::new();
        if let Some(referer) = web_address(entry, "referer") {
            headers.push(("Referer".to_string(), referer));
        }
        if let Some(agent) = agent {
            headers.push(("User-Agent".to_string(), agent.to_string()));
        }
        // A stream has no file name of its own, so it is named after its page.
        let title = plain_text(entry, "title", 150).filter(|_| stream);
        // The site's cookies for it, which a download behind a sign-in needs:
        // kept encrypted with the download, never sent to another site.
        let request = plain_text(entry, "cookies", 64 * 1024)
            .filter(|c| !c.contains(['\r', '\n']))
            .map(|cookies| RequestOptions { cookies: Some(cookies), ..Default::default() });
        items.push(Sent::File(NewDownload { url, headers, filename: title, via_site: stream, request, ..Default::default() }));
    }
    if items.is_empty() {
        return Err("None of those are links the app can download".into());
    }
    Ok(items)
}

fn respond(stream: &mut TcpStream, status: &str, origin: Option<&str>, body: &serde_json::Value) {
    let body = body.to_string();
    let mut head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n",
        body.len()
    );
    // Only an extension is told it may read the answer.
    if let Some(origin) = origin {
        head.push_str(&format!(
            "Access-Control-Allow-Origin: {origin}\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type, Authorization\r\nVary: Origin\r\n"
        ));
    }
    head.push_str("\r\n");
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body.as_bytes());
    let _ = stream.flush();
}

fn serve(mut stream: TcpStream, app: &AppHandle, engine: &Engine, port: u16) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
    let Some(request) = read_request(&mut stream) else { return };
    let no = |stream: &mut TcpStream, status: &str, why: &str| respond(stream, status, None, &serde_json::json!({ "error": why }));
    if !addressed_here(&request.host, port) {
        return no(&mut stream, "403 Forbidden", "not for this app");
    }
    let bridge = app.state::<Bridge>();
    let extension = from_extension(&request.origin);
    let origin = extension.then_some(request.origin.as_str());
    let paired = extension && bridge.check(&request.origin, request.key.as_deref());
    match (request.method.as_str(), request.path.as_str()) {
        // Says the app is running, and whether the extension asking is paired.
        ("GET", "/ping") => respond(
            &mut stream,
            "200 OK",
            origin,
            &serde_json::json!({ "app": "RushDownload", "version": env!("CARGO_PKG_VERSION"), "paired": paired }),
        ),
        ("OPTIONS", _) if extension => respond(&mut stream, "200 OK", origin, &serde_json::json!({})),
        ("POST", "/pair") if extension => {
            let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap_or_default();
            let browser = plain_text(&body, "browser", 100).unwrap_or_else(|| "a browser".into());
            match bridge.ask(&request.origin, &browser) {
                Some(ask) => {
                    let _ = app.emit("extension-pair", ask.clone());
                    if let Some(window) = app.get_window("main") {
                        let _ = window.show();
                        let _ = window.unminimize();
                        let _ = window.set_focus();
                    }
                    respond(&mut stream, "200 OK", origin, &serde_json::json!({ "request": ask.id }))
                }
                None => respond(&mut stream, "429 Too Many Requests", origin, &serde_json::json!({ "error": "Answer the requests already shown in RushDownload first" })),
            }
        }
        ("GET", path) if extension && path.starts_with("/pair/") => {
            let id = &path["/pair/".len()..];
            respond(&mut stream, "200 OK", origin, &bridge.poll(id, &request.origin))
        }
        ("POST", "/add") if paired => match sent_items(&request.body) {
            Err(why) => respond(&mut stream, "400 Bad Request", origin, &serde_json::json!({ "error": why })),
            Ok(items) => {
                let (mut added, mut pages, mut failed) = (0usize, 0usize, Vec::new());
                for item in items {
                    match item {
                        Sent::Page(link) => {
                            pages += 1;
                            let _ = app.emit("extension-link", link);
                        }
                        Sent::File(new) => match engine.add(new) {
                            Ok(_) => added += 1,
                            Err(why) => failed.push(why),
                        },
                    }
                }
                if added > 0 {
                    let _ = app.emit("extension-added", added);
                }
                // A page link waits in the Home box for the user to choose a
                // quality, so the window is brought forward for it.
                if pages > 0 {
                    if let Some(window) = app.get_window("main") {
                        let _ = window.unminimize();
                        let _ = window.set_focus();
                    }
                }
                respond(&mut stream, "200 OK", origin, &serde_json::json!({ "added": added, "pages": pages, "failed": failed }))
            }
        },
        ("POST", "/add") if extension => respond(
            &mut stream,
            "401 Unauthorized",
            origin,
            &serde_json::json!({ "error": "This browser is not connected to RushDownload yet", "pair": true }),
        ),
        ("POST", "/add") => no(&mut stream, "403 Forbidden", "only a connected browser extension may add links"),
        _ => no(&mut stream, "404 Not Found", "nothing here"),
    }
}

/// Starts listening for the extension. When the port is taken (another copy
/// of the app is open) this copy simply does not listen.
pub fn listen(app: AppHandle, engine: Engine, port: u16) {
    let Ok(listener) = TcpListener::bind((Ipv4Addr::LOCALHOST, port)) else {
        tracing::warn!(port, "the browser extension's port is taken; not listening");
        return;
    };
    tracing::info!(port, "listening for the browser extension");
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let (app, engine) = (app.clone(), engine.clone());
            std::thread::spawn(move || serve(stream, &app, &engine, port));
        }
    });
}

/* ---------- commands ---------- */

#[tauri::command]
pub fn paired_extensions(bridge: tauri::State<Bridge>) -> Vec<PairedInfo> {
    bridge.list()
}

#[tauri::command]
pub fn revoke_extension(bridge: tauri::State<Bridge>, id: String) -> Vec<PairedInfo> {
    bridge.revoke(&id);
    bridge.list()
}

#[tauri::command]
pub fn answer_extension_pair(bridge: tauri::State<Bridge>, id: String, allow: bool) -> Result<Vec<PairedInfo>, String> {
    bridge.answer(&id, allow)?;
    Ok(bridge.list())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_is_read_whole_and_oversized_ones_are_dropped() {
        let text = "POST /add?x=1 HTTP/1.1\r\nHost: 127.0.0.1:38951\r\nOrigin: chrome-extension://abc\r\nAuthorization: Bearer k3y\r\nContent-Length: 5\r\n\r\nhello extra";
        let request = read_request(&mut text.as_bytes()).unwrap();
        assert_eq!(request.method, "POST");
        assert_eq!(request.path, "/add");
        assert_eq!(request.origin, "chrome-extension://abc");
        assert_eq!(request.host, "127.0.0.1:38951");
        assert_eq!(request.key.as_deref(), Some("k3y"));
        assert_eq!(request.body, b"hello");

        let huge = format!("POST /add HTTP/1.1\r\nContent-Length: {}\r\n\r\n", MAX_BODY + 1);
        assert!(read_request(&mut huge.as_bytes()).is_none());
        let cut_short = "POST /add HTTP/1.1\r\nContent-Length: 50\r\n\r\nhello";
        assert!(read_request(&mut cut_short.as_bytes()).is_none());
        assert!(read_request(&mut "GET /ping HTTP/1.1\r\n".as_bytes()).is_none());
        let basic = read_request(&mut "GET /ping HTTP/1.1\r\nAuthorization: Basic x\r\n\r\n".as_bytes()).unwrap();
        assert_eq!(basic.key, None);
    }

    #[test]
    fn only_extensions_addressing_this_computer_are_let_in() {
        assert!(from_extension("chrome-extension://abcdefghijklmnop"));
        assert!(from_extension("moz-extension://1b2c-3d"));
        for page in ["", "null", "https://evil.test", "http://127.0.0.1:38951", "chrome-extension://", "https://chrome-extension://x"] {
            assert!(!from_extension(page), "{page:?}");
        }
        assert!(addressed_here("127.0.0.1:38951", 38951));
        assert!(addressed_here("localhost:38951", 38951));
        assert!(!addressed_here("evil.test:38951", 38951));
        assert!(!addressed_here("127.0.0.1:1", 38951));
        assert!(!addressed_here("", 38951));
    }

    fn bridge(name: &str) -> (Bridge, PathBuf) {
        let dir = std::env::temp_dir().join(format!("rd-bridge-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        (Bridge::load(&dir), dir)
    }

    #[test]
    fn pairing_needs_the_users_yes_and_gives_a_key_once() {
        let (b, dir) = bridge("pair");
        let me = "chrome-extension://me";
        let ask = b.ask(me, "Chrome 131").unwrap();
        assert_eq!(b.poll(&ask.id, me)["state"], "pending");
        assert_eq!(b.poll(&ask.id, "chrome-extension://other")["state"], "expired", "another extension cannot pick it up");
        b.answer(&ask.id, true).unwrap();
        let answer = b.poll(&ask.id, me);
        assert_eq!(answer["state"], "approved");
        let key = answer["key"].as_str().unwrap().to_string();
        assert_eq!(key.len(), 64);
        assert_eq!(b.poll(&ask.id, me)["state"], "expired", "the key is handed out once");

        assert!(b.check(me, Some(&key)));
        assert!(!b.check(me, Some("wrong")));
        assert!(!b.check(me, None));
        assert!(!b.check("chrome-extension://other", Some(&key)), "the key works for its own extension only");
        assert!(b.list()[0].last_seen.is_some());

        // only a hash is kept, and it survives a restart
        let saved = std::fs::read_to_string(dir.join("paired-extensions.json")).unwrap();
        assert!(!saved.contains(&key));
        let again = Bridge::load(&dir);
        assert!(again.check(me, Some(&key)));

        // revoked: the key no longer works
        let id = again.list()[0].id.clone();
        again.revoke(&id);
        assert!(!again.check(me, Some(&key)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_denied_or_flooding_extension_gets_nothing() {
        let (b, dir) = bridge("deny");
        let me = "moz-extension://me";
        let ask = b.ask(me, "Firefox").unwrap();
        b.answer(&ask.id, false).unwrap();
        assert_eq!(b.poll(&ask.id, me)["state"], "denied");
        assert!(b.list().is_empty());
        for _ in 0..MAX_PENDING {
            assert!(b.ask(me, "Firefox").is_some());
        }
        assert!(b.ask(me, "Firefox").is_none(), "no more than a few requests wait at once");
        assert!(b.answer("unknown", true).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_plain_web_addresses_are_taken_from_the_extension() {
        let body = br#"{"agent":"Mozilla/5.0 Test","items":[
            {"url":"https://cdn.test/v/clip.mp4","kind":"video","referer":"https://site.test/watch","title":"Holiday"},
            {"url":"https://cdn.test/hls/master.m3u8","kind":"stream","referer":"https://site.test/watch","title":"Holi\nday"},
            {"url":"https://site.test/watch","kind":"page"},
            {"url":"https://files.test/report.pdf","kind":"download","cookies":"session=abc"},
            {"url":"magnet:?xt=urn:btih:ABC","kind":"video"},
            {"url":"file:///C:/secret.txt","kind":"video"},
            {"url":"C:\\Users\\me\\a.torrent","kind":"video"},
            {"kind":"video"}
        ]}"#;
        let items = sent_items(body).unwrap();
        assert_eq!(items.len(), 4);
        let Sent::File(file) = &items[0] else { panic!("a file") };
        assert_eq!(file.url, "https://cdn.test/v/clip.mp4");
        assert!(!file.via_site);
        assert_eq!(file.filename, None, "a file keeps its own name");
        assert_eq!(
            file.headers,
            vec![
                ("Referer".to_string(), "https://site.test/watch".to_string()),
                ("User-Agent".to_string(), "Mozilla/5.0 Test".to_string())
            ]
        );
        assert!(file.request.is_none());
        let Sent::File(stream) = &items[1] else { panic!("a stream") };
        assert!(stream.via_site);
        assert_eq!(stream.filename.as_deref(), Some("Holiday"), "named after its page, control characters removed");
        assert!(matches!(&items[2], Sent::Page(link) if link == "https://site.test/watch"));
        let Sent::File(caught) = &items[3] else { panic!("a caught download") };
        assert_eq!(caught.request.as_ref().and_then(|r| r.cookies.as_deref()), Some("session=abc"), "its cookies go with it, to be kept encrypted");

        // a referer that is not a web address is left out, the link still taken
        let odd = sent_items(br#"{"items":[{"url":"https://c.test/a.mp4","referer":"javascript:alert(1)"}]}"#).unwrap();
        let Sent::File(file) = &odd[0] else { panic!("a file") };
        assert!(file.headers.is_empty());

        assert!(sent_items(b"not json").is_err());
        assert!(sent_items(br#"{"items":[]}"#).is_err());
        assert!(sent_items(br#"{"items":[{"url":"ftp://x.test/a.mp4"}]}"#).is_err());
    }
}
