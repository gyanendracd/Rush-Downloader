//! Where the browser extension hands over the links it finds on a page.
//!
//! The app listens on this computer only (127.0.0.1). Web pages can reach
//! that address too, so nothing is taken on trust: a request to add links
//! must come from a browser extension (browsers set the `Origin` of a request
//! themselves; a page cannot fake it), and every link is checked to be a
//! plain web address before the engine sees it.

use rushdownload_engine::{Engine, NewDownload};
use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

/// The port the extension looks for the app on.
pub const PORT: u16 = 38951;

const MAX_HEAD: usize = 16 * 1024;
const MAX_BODY: usize = 1024 * 1024;
const MAX_ITEMS: usize = 100;

#[derive(Debug, PartialEq)]
struct Request {
    method: String,
    path: String,
    origin: String,
    host: String,
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
    let (mut origin, mut host, mut length) = (String::new(), String::new(), 0usize);
    for line in lines {
        let Some((name, value)) = line.split_once(':') else { continue };
        let value = value.trim();
        match name.trim().to_ascii_lowercase().as_str() {
            "origin" => origin = value.to_string(),
            "host" => host = value.to_string(),
            "content-length" => length = value.parse().ok()?,
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
    Some(Request { method, path, origin, host, body })
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

/// One thing the extension sent.
#[derive(Debug)]
enum Sent {
    /// A page for the app to look up itself; it lands in the Home box.
    Page(String),
    /// A video or audio file, or a stream, to download as it is.
    File(NewDownload),
}

fn web_address(value: &serde_json::Value, key: &str) -> Option<String> {
    let text = value.get(key)?.as_str()?.trim();
    let url = tauri::Url::parse(text).ok()?;
    (matches!(url.scheme(), "http" | "https") && url.host_str().is_some() && text.len() <= 4000)
        .then(|| text.to_string())
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
        let title = entry
            .get("title")
            .and_then(|t| t.as_str())
            .map(|t| t.chars().filter(|c| !c.is_control()).take(150).collect::<String>())
            .filter(|t| stream && !t.trim().is_empty());
        items.push(Sent::File(NewDownload { url, headers, filename: title, via_site: stream, ..Default::default() }));
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
            "Access-Control-Allow-Origin: {origin}\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type\r\nVary: Origin\r\n"
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
    let extension = from_extension(&request.origin);
    let origin = extension.then_some(request.origin.as_str());
    match (request.method.as_str(), request.path.as_str()) {
        // Says the app is running, and nothing more.
        ("GET", "/ping") => respond(
            &mut stream,
            "200 OK",
            origin,
            &serde_json::json!({ "app": "RushDownload", "version": env!("CARGO_PKG_VERSION") }),
        ),
        ("OPTIONS", _) if extension => respond(&mut stream, "200 OK", origin, &serde_json::json!({})),
        ("POST", "/add") if extension => match sent_items(&request.body) {
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
                respond(
                    &mut stream,
                    "200 OK",
                    origin,
                    &serde_json::json!({ "added": added, "pages": pages, "failed": failed }),
                );
            }
        },
        ("POST", "/add") => no(&mut stream, "403 Forbidden", "only the browser extension may add links"),
        _ => no(&mut stream, "404 Not Found", "nothing here"),
    }
}

/// Starts listening for the extension. When the port is taken (another copy
/// of the app is open) this copy simply does not listen.
pub fn listen(app: AppHandle, engine: Engine, port: u16) {
    let Ok(listener) = TcpListener::bind((Ipv4Addr::LOCALHOST, port)) else { return };
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let (app, engine) = (app.clone(), engine.clone());
            std::thread::spawn(move || serve(stream, &app, &engine, port));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_is_read_whole_and_oversized_ones_are_dropped() {
        let text = "POST /add?x=1 HTTP/1.1\r\nHost: 127.0.0.1:38951\r\nOrigin: chrome-extension://abc\r\nContent-Length: 5\r\n\r\nhello extra";
        let request = read_request(&mut text.as_bytes()).unwrap();
        assert_eq!(request.method, "POST");
        assert_eq!(request.path, "/add");
        assert_eq!(request.origin, "chrome-extension://abc");
        assert_eq!(request.host, "127.0.0.1:38951");
        assert_eq!(request.body, b"hello");

        let huge = format!("POST /add HTTP/1.1\r\nContent-Length: {}\r\n\r\n", MAX_BODY + 1);
        assert!(read_request(&mut huge.as_bytes()).is_none());
        let cut_short = "POST /add HTTP/1.1\r\nContent-Length: 50\r\n\r\nhello";
        assert!(read_request(&mut cut_short.as_bytes()).is_none());
        assert!(read_request(&mut "GET /ping HTTP/1.1\r\n".as_bytes()).is_none());
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

    #[test]
    fn only_plain_web_addresses_are_taken_from_the_extension() {
        let body = br#"{"agent":"Mozilla/5.0 Test","items":[
            {"url":"https://cdn.test/v/clip.mp4","kind":"video","referer":"https://site.test/watch","title":"Holiday"},
            {"url":"https://cdn.test/hls/master.m3u8","kind":"stream","referer":"https://site.test/watch","title":"Holi\nday"},
            {"url":"https://site.test/watch","kind":"page"},
            {"url":"magnet:?xt=urn:btih:ABC","kind":"video"},
            {"url":"file:///C:/secret.txt","kind":"video"},
            {"url":"C:\\Users\\me\\a.torrent","kind":"video"},
            {"kind":"video"}
        ]}"#;
        let items = sent_items(body).unwrap();
        assert_eq!(items.len(), 3);
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
        let Sent::File(stream) = &items[1] else { panic!("a stream") };
        assert!(stream.via_site);
        assert_eq!(stream.filename.as_deref(), Some("Holiday"), "named after its page, control characters removed");
        assert!(matches!(&items[2], Sent::Page(link) if link == "https://site.test/watch"));

        // a referer that is not a web address is left out, the link still taken
        let odd = sent_items(br#"{"items":[{"url":"https://c.test/a.mp4","referer":"javascript:alert(1)"}]}"#).unwrap();
        let Sent::File(file) = &odd[0] else { panic!("a file") };
        assert!(file.headers.is_empty());

        assert!(sent_items(b"not json").is_err());
        assert!(sent_items(br#"{"items":[]}"#).is_err());
        assert!(sent_items(br#"{"items":[{"url":"ftp://x.test/a.mp4"}]}"#).is_err());
    }
}
