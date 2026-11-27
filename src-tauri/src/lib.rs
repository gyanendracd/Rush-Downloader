use rushdownload_engine::{
    Engine, Found, ItemInfo, MediaInfo, NewDownload, PlaylistInfo, ProfileFace, ProfileRef, Settings, SiteFile, SiteFiles, SiteKind, Status, TaskInfo, ToolStatus,
    TorrentPreview,
};
use tauri::{AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, State, WebviewUrl};
use tauri_plugin_opener::OpenerExt;

mod extension;
mod shell;

struct AppState(Engine);

#[tauri::command]
fn list_downloads(state: State<AppState>) -> Vec<TaskInfo> {
    state.0.list()
}

/// The videos or files of a profile or playlist download.
#[tauri::command]
async fn download_items(app: AppHandle, state: State<'_, AppState>, id: u64) -> Result<Vec<ItemInfo>, String> {
    let engine = state.0.clone();
    // The folder may be read for this, which must not hold up the window.
    let items = tauri::async_runtime::spawn_blocking(move || engine.items(id))
        .await
        .map_err(|e| e.to_string())?;
    // The window shows each finished file as its own picture, which it may
    // only do for folders it has been let into: these ones, and no others.
    let mut folders: Vec<&std::path::Path> = items.iter().filter_map(|item| item.path.as_deref()?.parent()).collect();
    folders.sort();
    folders.dedup();
    for folder in folders {
        let _ = app.asset_protocol_scope().allow_directory(folder, false);
    }
    Ok(items)
}

#[tauri::command]
fn add_download(state: State<AppState>, request: NewDownload) -> Result<u64, String> {
    state.0.add(request)
}

#[tauri::command]
async fn scan_page(state: State<'_, AppState>, url: String) -> Result<Vec<Found>, String> {
    let engine = state.0.clone();
    engine.scan_page(&url).await
}

/// The size of a file a browsed page offers, asked the way the page's own
/// player would ask (from that page, as that browser).
#[tauri::command]
async fn link_size(state: State<'_, AppState>, url: String, referer: String, agent: String) -> Result<Option<u64>, String> {
    let engine = state.0.clone();
    let mut headers = vec![("Referer".to_string(), referer)];
    if !agent.is_empty() && agent.len() < 400 {
        headers.push(("User-Agent".to_string(), agent));
    }
    Ok(engine.link_size(&url, &headers).await)
}

#[tauri::command]
async fn media_info(state: State<'_, AppState>, url: String) -> Result<MediaInfo, String> {
    let engine = state.0.clone();
    engine.media_info(&url).await
}

/// Whether copied links are picked up. Off stops the clipboard being read.
static CLIPBOARD_WATCH: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// A copied text that is one link the app can download: a web address, a
/// magnet link, or the path of a .torrent file. Anything else is ignored.
fn copied_link(text: &str) -> Option<String> {
    let text = text.trim();
    if text.is_empty() || text.len() > 2000 || text.chars().any(|c| c == '\n' || c == '\r') {
        return None;
    }
    let lower = text.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        let fine = !text.chars().any(char::is_whitespace)
            && tauri::Url::parse(text).ok().is_some_and(|u| u.host_str().is_some_and(|h| h.contains('.') || h == "localhost"));
        return fine.then(|| text.to_string());
    }
    if lower.starts_with("magnet:?") {
        return (!text.chars().any(char::is_whitespace)).then(|| text.to_string());
    }
    let path = text.trim_matches('"');
    (path.to_ascii_lowercase().ends_with(".torrent") && std::path::Path::new(path).is_absolute())
        .then(|| text.to_string())
}

/// Watches the clipboard and tells the interface when a link is copied, the
/// way a download manager picks links up. Only links leave this function;
/// any other copied text is looked at and dropped.
fn watch_clipboard(app: AppHandle) {
    std::thread::spawn(move || {
        let mut board: Option<arboard::Clipboard> = None;
        let mut last: Option<String> = None;
        // What is on the clipboard when watching starts is not a new copy.
        let mut settled = false;
        loop {
            std::thread::sleep(std::time::Duration::from_millis(600));
            if !CLIPBOARD_WATCH.load(std::sync::atomic::Ordering::Relaxed) {
                settled = false;
                continue;
            }
            if board.is_none() {
                board = arboard::Clipboard::new().ok();
            }
            let Some(clipboard) = board.as_mut() else { continue };
            let text = clipboard.get_text().ok();
            if !settled {
                settled = true;
                last = text;
                continue;
            }
            // Something that is not text (a picture, a file) is not a link.
            let Some(text) = text else { continue };
            if last.as_deref() == Some(text.as_str()) {
                continue;
            }
            last = Some(text.clone());
            if let Some(link) = copied_link(&text) {
                let _ = app.emit("clipboard-link", link);
            }
        }
    });
}

/// Turns picking up copied links on or off.
#[tauri::command]
fn set_clipboard_watch(on: bool) {
    CLIPBOARD_WATCH.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// The cookie that shows a real sign-in on the sites the app knows. Visiting
/// a sign-in page leaves cookies too, and those must not count.
const LOGIN_COOKIES: &[(&str, &[&str])] = &[
    ("x.com", &["auth_token"]),
    ("twitter.com", &["auth_token"]),
    ("instagram.com", &["sessionid"]),
    ("facebook.com", &["c_user"]),
    ("tiktok.com", &["sessionid", "sid_tt"]),
    ("reddit.com", &["reddit_session", "token_v2"]),
    ("youtube.com", &["LOGIN_INFO", "SID", "__Secure-1PSID"]),
    ("google.com", &["SID", "__Secure-1PSID"]),
    ("github.com", &["user_session"]),
    ("vk.com", &["remixsid"]),
    ("twitch.tv", &["auth-token"]),
    ("bilibili.com", &["SESSDATA"]),
    ("linkedin.com", &["li_at"]),
    ("threads.com", &["sessionid"]),
    ("threads.net", &["sessionid"]),
];

/// Sites a sign-in is kept for, shortest form ("x.com", "instagram.com").
fn signed_in_sites(file: &std::path::Path) -> Vec<String> {
    let mut sites: Vec<String> = std::fs::read_to_string(file)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| {
            let (domain, _, name) = cookie_key(line)?;
            let labels: Vec<&str> = domain.trim_start_matches('.').split('.').collect();
            if labels.len() < 2 {
                return None;
            }
            let site = labels[labels.len() - 2..].join(".");
            let signed_in = match LOGIN_COOKIES.iter().find(|(known, _)| *known == site) {
                Some((_, names)) => names.contains(&name.as_str()),
                // Elsewhere, a cookie named like a session is the best sign there is.
                None => {
                    let lower = name.to_ascii_lowercase();
                    ["session", "auth", "token", "login"].iter().any(|w| lower.contains(w))
                }
            };
            signed_in.then_some(site)
        })
        .collect();
    sites.sort();
    sites.dedup();
    sites
}

const COOKIE_FILE_HEAD: &str = "# Netscape HTTP Cookie File\n# Sign-ins kept by RushDownload.\n\n";

/// One cookie line's identity: site, path and name.
fn cookie_key(line: &str) -> Option<(String, String, String)> {
    let line = line.strip_prefix("#HttpOnly_").unwrap_or(line);
    if line.starts_with('#') {
        return None;
    }
    let fields: Vec<&str> = line.split('\t').collect();
    (fields.len() == 7 && !fields[0].is_empty() && !fields[5].is_empty())
        .then(|| (fields[0].to_string(), fields[2].to_string(), fields[5].to_string()))
}

/// Adds cookie lines to the kept sign-ins; a newer cookie replaces the older
/// one for the same site, path and name. Returns how many lines were valid.
fn merge_cookies(file: &std::path::Path, incoming: &[String]) -> std::io::Result<usize> {
    let fresh: Vec<&String> = incoming.iter().filter(|l| cookie_key(l).is_some()).collect();
    let replaced: std::collections::HashSet<_> = fresh.iter().filter_map(|l| cookie_key(l)).collect();
    let mut out = String::from(COOKIE_FILE_HEAD);
    for line in std::fs::read_to_string(file).unwrap_or_default().lines() {
        if cookie_key(line).is_some_and(|key| !replaced.contains(&key)) {
            out.push_str(line);
            out.push('\n');
        }
    }
    for line in &fresh {
        out.push_str(line);
        out.push('\n');
    }
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(file, out)?;
    Ok(fresh.len())
}

/// Keeps the sign-in window's cookies, in the cookie-file format that yt-dlp
/// and gallery-dl read.
fn write_cookies(file: &std::path::Path, cookies: &[tauri::webview::Cookie<'static>]) -> std::io::Result<()> {
    let mut lines = Vec::new();
    for c in cookies {
        let Some(domain) = c.domain().filter(|d| !d.is_empty()) else { continue };
        // The app's own pages are not a sign-in.
        if domain.contains("localhost") || c.name().is_empty() || c.value().contains(['\t', '\n']) {
            continue;
        }
        let expires = c.expires_datetime().map_or(0, |t| t.unix_timestamp().max(0));
        lines.push(format!(
            "{}{}\t{}\t{}\t{}\t{}\t{}\t{}",
            if c.http_only().unwrap_or(false) { "#HttpOnly_" } else { "" },
            domain,
            if domain.starts_with('.') { "TRUE" } else { "FALSE" },
            c.path().unwrap_or("/"),
            if c.secure().unwrap_or(false) { "TRUE" } else { "FALSE" },
            expires,
            c.name(),
            c.value(),
        ));
    }
    merge_cookies(file, &lines).map(|_| ())
}

/// Adds the sign-ins from a cookies file exported from a browser (the
/// "cookies.txt" format). For sites that refuse to sign in inside other
/// apps, Google and YouTube among them.
#[tauri::command]
fn import_sign_in(state: State<AppState>, path: String) -> Result<Vec<String>, String> {
    let text = std::fs::read_to_string(&path).map_err(|e| format!("Could not read that file: {e}"))?;
    let lines: Vec<String> = text.lines().map(|l| l.trim_end_matches('\r').to_string()).collect();
    let file = state.0.cookies_file();
    let added = merge_cookies(&file, &lines).map_err(|e| format!("Could not save the sign-in: {e}"))?;
    if added == 0 {
        return Err("That file holds no sign-ins. It has to be a cookies file exported from a browser".into());
    }
    Ok(signed_in_sites(&file))
}

/// The label of the site page shown inside the main window. It is not listed
/// in the app's capabilities, so the page cannot call any command.
const SIGN_IN_VIEW: &str = "signin";
/// Windows a sign-in page opened itself (another site's sign-in).
const SIGN_IN_POPUP: &str = "signin-popup-";
static POPUPS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn close_sign_in_popups(app: &AppHandle) {
    for (label, window) in app.webview_windows() {
        if label.starts_with(SIGN_IN_POPUP) {
            let _ = window.destroy();
        }
    }
}

/// Set while the embedded page is used for browsing rather than signing in.
static BROWSING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// How the catcher script marks a find in the page title.
const MEDIA_MARK: &str = "RUSHDL::MEDIA::";

/// How the catcher script marks a note about the page itself.
const PAGE_MARK: &str = "RUSHDL::PAGE::";

/// The picture a browsed page offers for itself. Only a plain web address
/// of sane length is taken from the page.
fn page_picture(json: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let text = value.get("thumbnail")?.as_str()?;
    let url = tauri::Url::parse(text).ok()?;
    (matches!(url.scheme(), "http" | "https") && text.len() <= 2000).then(|| text.to_string())
}

/// A video or audio file a browsed page loaded.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
struct CaughtMedia {
    url: String,
    /// "video", "audio" or "stream".
    kind: String,
    /// The page (or frame) that loaded it.
    referer: String,
    /// Its resolution when the page or the address says it ("720p"), else empty.
    label: String,
    /// For a stream: the picture heights its playlist names, largest first.
    heights: Vec<u32>,
    /// For a stream: this is the playlist that lists every quality.
    master: bool,
}

/// Reads one find sent by a page. Pages are not trusted: only plain web
/// addresses of sane length get through.
fn caught_media(json: &str) -> Option<CaughtMedia> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let address = |key: &str| {
        let text = value.get(key)?.as_str()?;
        let url = tauri::Url::parse(text).ok()?;
        (matches!(url.scheme(), "http" | "https") && text.len() <= 4000).then(|| text.to_string())
    };
    let kind = match value.get("kind")?.as_str()? {
        "audio" => "audio",
        "stream" => "stream",
        _ => "video",
    };
    Some(CaughtMedia {
        url: address("url")?,
        kind: kind.to_string(),
        referer: address("referer")?,
        // Only a plain resolution is taken from the page's words.
        label: value
            .get("label")
            .and_then(|l| l.as_str())
            .filter(|l| {
                l.len() <= 6 && l.ends_with('p') && l[..l.len() - 1].chars().all(|c| c.is_ascii_digit()) && l.len() > 1
            })
            .unwrap_or_default()
            .to_string(),
        // Only sane picture heights, and only a handful of them.
        heights: value
            .get("heights")
            .and_then(|h| h.as_array())
            .map(|list| {
                list.iter()
                    .filter_map(|h| h.as_u64())
                    .filter(|h| (100..=4320).contains(h))
                    .map(|h| h as u32)
                    .take(12)
                    .collect()
            })
            .unwrap_or_default(),
        master: kind == "stream" && value.get("master").and_then(|m| m.as_bool()).unwrap_or(false),
    })
}

static BROWSER_ARGS: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();

/// Start-up switches for the embedded browser while secure DNS is on: it is
/// sent through the app's local proxy, which looks site names up at Cloudflare
/// (1.1.1.1) over HTTPS, as the downloads do.
fn browser_args(proxy: std::net::SocketAddr) -> String {
    // The first switch is what the app framework itself passes; setting any
    // switch replaces it.
    format!("--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --proxy-server=http://{proxy}")
}

/// Sign-in pages are laid out for a full browser window, about this large.
/// The page is scaled down to fit when the area it gets is smaller.
const SIGN_IN_PAGE_WIDTH: f64 = 840.0;
const SIGN_IN_PAGE_HEIGHT: f64 = 900.0;

fn sign_in_zoom(width: f64, height: f64) -> f64 {
    (width / SIGN_IN_PAGE_WIDTH).min(height / SIGN_IN_PAGE_HEIGHT).clamp(0.55, 1.0)
}

/// Shows a site's own sign-in page inside the main window, over the given
/// area of it (in the interface's own pixels). The user signs in there.
#[tauri::command]
async fn signin_open(
    app: AppHandle,
    state: State<'_, AppState>,
    url: String,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    browse: Option<bool>,
) -> Result<(), String> {
    let url = tauri::Url::parse(&url).map_err(|_| "That is not a valid address".to_string())?;
    let local = matches!(url.host_str(), Some("127.0.0.1" | "localhost"));
    if url.scheme() != "https" && !(url.scheme() == "http" && local) {
        return Err("Only https pages can be opened".into());
    }
    // Browsing shows pages at their own size, opens nothing in new windows
    // and reports the media a page loads; signing in shrinks the page to fit.
    let browse = browse.unwrap_or(false);
    BROWSING.store(browse, std::sync::atomic::Ordering::Relaxed);
    let zoom = if browse { 1.0 } else { sign_in_zoom(width, height) };
    let position = LogicalPosition::new(x, y);
    let size = LogicalSize::new(width.max(80.0), height.max(80.0));
    if let Some(view) = app.get_webview(SIGN_IN_VIEW) {
        view.navigate(url).map_err(|e| e.to_string())?;
        let _ = view.set_position(position);
        let _ = view.set_size(size);
        let _ = view.set_zoom(zoom);
        let _ = view.show();
        return Ok(());
    }
    let window = app.get_window("main").ok_or("the main window is not open")?;
    // The sites get browser data of their own, apart from the app's, so
    // forgetting the sign-ins removes every trace of them.
    let data = state.0.cookies_file().with_file_name("signin-web");
    let notify = app.clone();
    let on_title = app.clone();
    let on_move = app.clone();
    let on_file = app.clone();
    let mut page = tauri::webview::WebviewBuilder::new(SIGN_IN_VIEW, WebviewUrl::External(url)).data_directory(data);
    // The embedded browser keeps the switches it first started with until the
    // app is restarted; asking for others would fail. Without secure DNS it
    // gets none of its own.
    let args = BROWSER_ARGS.get_or_init(|| {
        // The proxy is started either way, so it is there to be used.
        let proxy = state.0.dns_proxy();
        // Switches given from outside (the test harness) are left to decide.
        if std::env::var_os("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS").is_some() {
            return None;
        }
        proxy.map(browser_args)
    });
    if let Some(args) = args {
        page = page.additional_browser_args(args);
    }
    window
        .add_child(
            page
                .initialization_script_for_all_frames(include_str!("catch_media.js"))
                // The catcher script passes what it finds through the page
                // title. Whatever a page sends is only ever a web address,
                // shown to the user to download or ignore.
                .on_document_title_changed(move |_, title| {
                    let browsing = BROWSING.load(std::sync::atomic::Ordering::Relaxed);
                    if let Some(json) = title.strip_prefix(MEDIA_MARK) {
                        if let Some(found) = caught_media(json).filter(|_| browsing) {
                            let _ = on_title.emit("browser-media", found);
                        }
                    } else if let Some(json) = title.strip_prefix(PAGE_MARK) {
                        if let Some(picture) = page_picture(json).filter(|_| browsing) {
                            let _ = on_title.emit("browser-page", picture);
                        }
                    } else {
                        let _ = on_title.emit("browser-title", title);
                    }
                })
                .on_navigation(move |target| {
                    let _ = on_move.emit("browser-nav", target.as_str().to_string());
                    true
                })
                // A file the page starts downloading (a site's own Download
                // button, often after a wait or a check made by script) goes
                // to the app instead, with the page's sign-in, like any other.
                .on_download(move |view, event| {
                    let tauri::webview::DownloadEvent::Requested { url, .. } = event else {
                        return true;
                    };
                    if !matches!(url.scheme(), "http" | "https") {
                        return true;
                    }
                    let app = on_file.clone();
                    let page = view.url().ok().map(|u| u.to_string());
                    // Asking the page for its cookies waits on the window, so
                    // it is done away from it.
                    tauri::async_runtime::spawn(async move {
                        let mut headers = Vec::new();
                        if let Ok(cookies) = view.cookies_for_url(url.clone()) {
                            let line = cookies.iter().map(|c| format!("{}={}", c.name(), c.value())).collect::<Vec<_>>().join("; ");
                            if !line.is_empty() {
                                headers.push(("Cookie".to_string(), line));
                            }
                        }
                        if let Some(page) = page.filter(|p| p.starts_with("http")) {
                            headers.push(("Referer".to_string(), page));
                        }
                        let engine = app.state::<AppState>().0.clone();
                        match engine.add(NewDownload { url: url.to_string(), headers, ..Default::default() }) {
                            Ok(_) => {
                                let _ = app.emit("browser-download", url.to_string());
                            }
                            Err(why) => {
                                let _ = app.emit("browser-download-failed", why);
                            }
                        }
                    });
                    false
                })
                // "Continue with Google" and the like open the other site's
                // sign-in in a window of their own and wait for its answer.
                // That window is the site's doing; it is opened as asked,
                // shares this page's browser data, and cannot call the app.
                .on_new_window(move |target, features| {
                    // While browsing, pages do not get to open windows (mostly
                    // advertising); the interface offers to open the address here.
                    if BROWSING.load(std::sync::atomic::Ordering::Relaxed) {
                        let _ = notify.emit("browser-popup", target.as_str().to_string());
                        return tauri::webview::NewWindowResponse::Deny;
                    }
                    let _ = notify.emit("sign-in-popup", target.host_str().unwrap_or_default().to_string());
                    let label = format!("{SIGN_IN_POPUP}{}", POPUPS.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
                    let blank = WebviewUrl::External("about:blank".parse().expect("a valid address"));
                    match tauri::WebviewWindowBuilder::new(&notify, label, blank)
                        .window_features(features)
                        .title("Sign in")
                        .build()
                    {
                        Ok(window) => tauri::webview::NewWindowResponse::Create { window },
                        Err(_) => tauri::webview::NewWindowResponse::Deny,
                    }
                }),
            position,
            size,
        )
        .map_err(|e| format!("could not open the page: {e}"))?
        .set_zoom(zoom)
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Keeps the sign-in page over its area when the window or page is resized.
#[tauri::command]
async fn signin_bounds(app: AppHandle, x: f64, y: f64, width: f64, height: f64) -> Result<(), String> {
    let browse = BROWSING.load(std::sync::atomic::Ordering::Relaxed);
    if let Some(view) = app.get_webview(SIGN_IN_VIEW) {
        let _ = view.set_position(LogicalPosition::new(x, y));
        let _ = view.set_size(LogicalSize::new(width.max(80.0), height.max(80.0)));
        let _ = view.set_zoom(if browse { 1.0 } else { sign_in_zoom(width, height) });
    }
    Ok(())
}

/// Back, forward or reload on the Browser page.
#[tauri::command]
async fn browser_go(app: AppHandle, action: String) -> Result<(), String> {
    let script = match action.as_str() {
        "back" => "history.back()",
        "forward" => "history.forward()",
        "reload" => "location.reload()",
        _ => return Err("unknown action".into()),
    };
    if let Some(view) = app.get_webview(SIGN_IN_VIEW) {
        view.eval(script).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Starts the player on the browsed page the way a visitor would (see
/// catch_media.js), for links the app fetches by itself.
#[tauri::command]
async fn browser_autoplay(app: AppHandle) -> Result<(), String> {
    if let Some(view) = app.get_webview(SIGN_IN_VIEW) {
        view.eval("window.postMessage({ __rushPlay: true }, '*')").map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Keeps the browsing session's sign-in for a download that is about to
/// start, without closing the page.
#[tauri::command]
async fn browser_keep_sign_in(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let file = state.0.cookies_file();
    if let Some(view) = app.get_webview(SIGN_IN_VIEW) {
        let _ = tauri::async_runtime::spawn_blocking(move || {
            if let Ok(cookies) = view.cookies() {
                let _ = write_cookies(&file, &cookies);
            }
        })
        .await;
    }
    Ok(())
}

/// Saves the sign-in made on the page for downloads, closes the page and
/// returns every site a sign-in is kept for.
#[tauri::command]
async fn signin_close(app: AppHandle, state: State<'_, AppState>) -> Result<Vec<String>, String> {
    let file = state.0.cookies_file();
    close_sign_in_popups(&app);
    if let Some(view) = app.get_webview(SIGN_IN_VIEW) {
        let target = file.clone();
        // Reading cookies must not happen on the main thread.
        let saved = tauri::async_runtime::spawn_blocking(move || {
            let saved = view
                .cookies()
                .map_err(|e| e.to_string())
                .and_then(|cookies| write_cookies(&target, &cookies).map_err(|e| e.to_string()));
            let _ = view.close();
            saved
        })
        .await
        .map_err(|e| e.to_string())?;
        saved.map_err(|e| format!("Could not save the sign-in: {e}"))?;
        let _ = app.emit("sign-in", signed_in_sites(&file));
    }
    Ok(signed_in_sites(&file))
}

/// Brings over the sign-ins a browser on this computer keeps (Firefox, Brave
/// and so on), so the user does not have to sign in again or export a file.
/// Every cookie the browser holds is kept, the way an imported cookies file
/// is; only real sign-ins are counted in the list that is shown.
#[tauri::command]
async fn import_browser_sign_in(state: State<'_, AppState>, browser: String) -> Result<Vec<String>, String> {
    let file = state.0.cookies_file();
    let lines = Engine::browser_cookies(&browser).await?;
    let added = merge_cookies(&file, &lines).map_err(|e| format!("Could not save the sign-in: {e}"))?;
    if added == 0 {
        return Err("That browser holds no sign-ins".into());
    }
    Ok(signed_in_sites(&file))
}

#[tauri::command]
fn sign_in_status(state: State<AppState>) -> Vec<String> {
    signed_in_sites(&state.0.cookies_file())
}

/// Forgets every sign-in: the kept cookies and the sites' browser data.
#[tauri::command]
async fn forget_sign_ins(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let file = state.0.cookies_file();
    let _ = std::fs::remove_file(&file);
    let data = file.with_file_name("signin-web");
    close_sign_in_popups(&app);
    if let Some(view) = app.get_webview(SIGN_IN_VIEW) {
        let _ = tauri::async_runtime::spawn_blocking(move || {
            for cookie in view.cookies().unwrap_or_default() {
                let _ = view.delete_cookie(cookie);
            }
            let _ = view.close();
        })
        .await;
    }
    // The browser lets go of its files a moment after the page closes.
    let _ = tauri::async_runtime::spawn_blocking(move || {
        for _ in 0..25 {
            if !data.exists() || std::fs::remove_dir_all(&data).is_ok() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
    })
    .await;
    Ok(())
}

#[tauri::command]
async fn playlist_info(state: State<'_, AppState>, url: String) -> Result<PlaylistInfo, String> {
    let engine = state.0.clone();
    engine.playlist_info(&url).await
}

#[tauri::command]
async fn profile_face(state: State<'_, AppState>, url: String) -> Result<ProfileFace, String> {
    let engine = state.0.clone();
    Ok(engine.profile_face(&url).await)
}

#[tauri::command]
async fn profile_stats(state: State<'_, AppState>, url: String) -> Result<ProfileFace, String> {
    let engine = state.0.clone();
    Ok(engine.profile_stats(&url).await)
}

#[tauri::command]
fn profile_of(state: State<AppState>, url: String) -> Option<ProfileRef> {
    state.0.profile_of(&url)
}

/// Whether a link is a whole collection on any site the downloading programs
/// know. Takes a few seconds the first time a link is asked about.
#[tauri::command]
async fn collection_of(state: State<'_, AppState>, url: String) -> Result<Option<ProfileRef>, String> {
    let engine = state.0.clone();
    Ok(engine.collection_of(&url).await)
}

#[tauri::command]
async fn torrent_preview(state: State<'_, AppState>, source: String) -> Result<TorrentPreview, String> {
    let engine = state.0.clone();
    engine.torrent_preview(&source).await
}

/// Whether a link is one of a site the app reads files from (or one it
/// cannot download from, and why). Answered from the link alone.
#[tauri::command]
fn site_kind(url: String) -> Option<SiteKind> {
    Engine::site_kind(&url)
}

/// The files a site link holds, to pick from.
#[tauri::command]
async fn site_files(state: State<'_, AppState>, url: String) -> Result<SiteFiles, String> {
    let engine = state.0.clone();
    engine.site_files(&url).await
}

/// Downloads the files picked from a site link.
#[tauri::command]
fn add_site_files(
    state: State<AppState>,
    url: String,
    title: String,
    files: Vec<SiteFile>,
    dir: Option<String>,
) -> Result<u64, String> {
    state.0.add_files(&url, &title, files, dir.filter(|d| !d.is_empty()).map(Into::into))
}

#[tauri::command]
fn pause_download(state: State<AppState>, id: u64) {
    state.0.pause(id);
}

#[tauri::command]
fn resume_download(state: State<AppState>, id: u64) {
    state.0.resume(id);
}

#[tauri::command]
fn remove_download(state: State<AppState>, id: u64, delete_file: bool) {
    state.0.remove(id, delete_file);
}

#[tauri::command]
fn get_settings(state: State<AppState>) -> Settings {
    state.0.settings()
}

#[tauri::command]
fn set_settings(state: State<AppState>, settings: Settings) {
    state.0.set_settings(settings);
}

fn completed(state: &AppState, id: u64) -> Result<TaskInfo, String> {
    state
        .0
        .list()
        .into_iter()
        .find(|t| t.id == id && t.status == Status::Completed)
        .ok_or_else(|| "That download is not finished yet".to_string())
}

#[tauri::command]
fn open_file(app: AppHandle, state: State<AppState>, id: u64) -> Result<(), String> {
    let task = completed(&state, id)?;
    app.opener()
        .open_path(task.path.to_string_lossy(), None::<&str>)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn reveal_file(app: AppHandle, state: State<AppState>, id: u64) -> Result<(), String> {
    let task = completed(&state, id)?;
    app.opener()
        .reveal_item_in_dir(&task.path)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn open_download_dir(app: AppHandle, state: State<AppState>) -> Result<(), String> {
    let dir = state.0.settings().download_dir;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    app.opener()
        .open_path(dir.to_string_lossy(), None::<&str>)
        .map_err(|e| e.to_string())
}

/// Reads a text file and returns every http(s) link in it, for batch adds.
#[tauri::command]
fn import_links(path: String) -> Result<Vec<String>, String> {
    let meta = std::fs::metadata(&path).map_err(|e| format!("Could not open the file: {e}"))?;
    if meta.len() > 5 * 1024 * 1024 {
        return Err("That file is too large to be a list of links".into());
    }
    let bytes = std::fs::read(&path).map_err(|e| format!("Could not read the file: {e}"))?;
    let text = String::from_utf8_lossy(&bytes);
    let mut links: Vec<String> = Vec::new();
    for word in text.split(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == '<' || c == '>') {
        let lower = word.to_ascii_lowercase();
        if (lower.starts_with("http://") || lower.starts_with("https://")) && !links.iter().any(|l| l == word) {
            links.push(word.to_string());
        }
    }
    Ok(links)
}

#[tauri::command]
fn tool_status() -> ToolStatus {
    Engine::tools()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![shell::AUTOSTART_ARG]),
        ))
        .on_window_event(shell::on_window_event)
        .setup(|app| {
            // RUSHDOWNLOAD_STATE_DIR points the app at a separate list and
            // settings, so it can be tested without touching the real ones.
            let state_dir = match std::env::var_os("RUSHDOWNLOAD_STATE_DIR") {
                Some(dir) => std::path::PathBuf::from(dir),
                None => app.path().app_data_dir()?,
            };
            // Everything lands in its own folder inside the user's Downloads.
            let system_downloads = app
                .path()
                .download_dir()
                .unwrap_or_else(|_| state_dir.join("Downloads"));
            let download_dir = system_downloads.join("Rush Downloads");
            let engine = tauri::async_runtime::block_on(async {
                Engine::new(&state_dir, Settings::new(download_dir.clone()))
            })?;

            // The window is opened here rather than from the configuration,
            // as its graphics follow a preference (see shell.rs).
            let prefs = shell::Prefs::load(&state_dir);
            shell::open_main_window(app.handle(), prefs.get())?;
            app.manage(prefs);
            shell::tray(app.handle())?;

            // Installs from before this folder existed saved straight into
            // Downloads. Move that old default over once; a folder the user
            // picked themselves (or picks later) is left alone.
            let moved = state_dir.join("default-folder-v2");
            if !moved.exists() {
                let mut settings = engine.settings();
                if settings.download_dir == system_downloads {
                    settings.download_dir = download_dir.clone();
                    engine.set_settings(settings);
                }
                let _ = std::fs::write(&moved, b"");
            }
            if engine.settings().download_dir == download_dir {
                let _ = std::fs::create_dir_all(&download_dir);
            }

            watch_clipboard(app.handle().clone());

            // The browser extension sends the links it finds here. A test
            // copy of the app (its own state folder) stays out of the way of
            // the real one unless it is given a port of its own.
            let extension_port = match std::env::var("RUSHDOWNLOAD_EXT_PORT") {
                Ok(port) => port.parse().ok(),
                Err(_) => std::env::var_os("RUSHDOWNLOAD_STATE_DIR").is_none().then_some(extension::PORT),
            };
            if let Some(port) = extension_port {
                extension::listen(app.handle().clone(), engine.clone(), port);
            }

            // Push every change to the UI.
            let handle = app.handle().clone();
            let watched = engine.clone();
            let mut changes = engine.subscribe();
            tauri::async_runtime::spawn(async move {
                while changes.changed().await.is_ok() {
                    let _ = handle.emit("downloads", watched.list());
                }
            });

            app.manage(AppState(engine));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_downloads,
            download_items,
            set_clipboard_watch,
            add_download,
            scan_page,
            torrent_preview,
            site_kind,
            site_files,
            add_site_files,
            media_info,
            link_size,
            profile_of,
            collection_of,
            profile_face,
            profile_stats,
            playlist_info,
            signin_open,
            signin_bounds,
            browser_go,
            browser_autoplay,
            browser_keep_sign_in,
            signin_close,
            sign_in_status,
            import_sign_in,
            import_browser_sign_in,
            forget_sign_ins,
            pause_download,
            resume_download,
            remove_download,
            get_settings,
            set_settings,
            open_file,
            reveal_file,
            open_download_dir,
            import_links,
            tool_status,
            shell::get_app_prefs,
            shell::set_app_prefs,
            shell::set_tray_text
        ])
        .run(tauri::generate_context!())
        .expect("error while running RushDownload");
}

#[cfg(test)]
mod sign_in_tests {
    use super::*;

    #[test]
    fn only_a_single_link_counts_as_a_copied_link() {
        let yes = |text: &str| assert_eq!(copied_link(text).as_deref(), Some(text.trim()), "{text}");
        yes("https://example.com/files/setup.zip");
        yes("  http://example.com/watch?v=abc123  ");
        yes("magnet:?xt=urn:btih:ABCDEF&dn=Some+Name");
        yes(r#""C:\Users\me\Downloads\ubuntu.iso.torrent""#);
        for no in [
            "",
            "just some words I copied",
            "see https://example.com for more",
            "https://example.com/a\nhttps://example.com/b",
            "https://",
            "https://nodots",
            "ftp://example.com/file.zip",
            "ubuntu.iso.torrent",
            "C:\\Users\\me\\notes.txt",
            "password123",
        ] {
            assert_eq!(copied_link(no), None, "{no:?}");
        }
    }

    #[test]
    fn only_plain_web_addresses_are_accepted_from_a_page() {
        let good = caught_media(r#"{"url":"https://cdn.test/v/clip.mp4?t=1","kind":"video","referer":"https://site.test/watch/1"}"#);
        assert_eq!(
            good,
            Some(CaughtMedia {
                url: "https://cdn.test/v/clip.mp4?t=1".into(),
                kind: "video".into(),
                referer: "https://site.test/watch/1".into(),
                label: String::new(),
                heights: Vec::new(),
                master: false,
            })
        );
        let labelled = |label: &str| {
            caught_media(&format!(
                r#"{{"url":"https://c.test/a.mp4","kind":"video","referer":"https://s.test/","label":"{label}"}}"#
            ))
            .unwrap()
            .label
        };
        assert_eq!(labelled("1080p"), "1080p");
        assert_eq!(labelled("<b>720p"), "", "anything but a plain resolution is dropped");
        assert_eq!(labelled("p"), "");
        let stream = caught_media(r#"{"url":"https://cdn.test/a.m3u8","kind":"stream","referer":"https://site.test/"}"#);
        assert_eq!(stream.unwrap().kind, "stream");
        // a playlist's qualities come through; nonsense among them does not
        let listed = caught_media(
            r#"{"url":"https://cdn.test/play","kind":"stream","referer":"https://site.test/","master":true,"heights":[1080,720,"x",5,99999,480]}"#,
        )
        .unwrap();
        assert_eq!(listed.heights, vec![1080, 720, 480]);
        assert!(listed.master);
        // only a stream can be a list of qualities
        let file = caught_media(r#"{"url":"https://c.test/a.mp4","kind":"video","referer":"https://s.test/","master":true}"#);
        assert!(!file.unwrap().master);
        // an unknown kind is treated as a video; anything not a web address is dropped
        assert_eq!(caught_media(r#"{"url":"https://c.test/a.mp4","kind":"<b>","referer":"https://s.test/"}"#).unwrap().kind, "video");
        assert!(caught_media(r#"{"url":"file:///C:/secret.txt","kind":"video","referer":"https://s.test/"}"#).is_none());
        assert!(caught_media(r#"{"url":"javascript:alert(1)","kind":"video","referer":"https://s.test/"}"#).is_none());
        assert!(caught_media(r#"{"url":"https://c.test/a.mp4","kind":"video"}"#).is_none());
        assert!(caught_media("not json").is_none());
    }

    #[test]
    fn only_a_web_address_is_taken_as_a_page_picture() {
        assert_eq!(
            page_picture(r#"{"__page":true,"thumbnail":"https://cdn.test/t/abc.jpg"}"#).as_deref(),
            Some("https://cdn.test/t/abc.jpg")
        );
        assert!(page_picture(r#"{"thumbnail":"javascript:alert(1)"}"#).is_none());
        assert!(page_picture(r#"{"thumbnail":"file:///C:/x.png"}"#).is_none());
        assert!(page_picture(r#"{"other":1}"#).is_none());
    }

    #[test]
    fn sign_in_pages_shrink_to_fit_small_areas_only() {
        assert_eq!(sign_in_zoom(1600.0, 1000.0), 1.0, "never enlarged");
        assert!((sign_in_zoom(880.0, 630.0) - 0.7).abs() < 0.001, "the shorter side decides");
        assert!((sign_in_zoom(420.0, 2000.0) - 0.55).abs() < 0.001, "never unreadably small");
    }

    #[test]
    fn imported_sign_ins_are_merged_and_bad_files_add_nothing() {
        let dir = std::env::temp_dir().join(format!("rush-cookie-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("state").join("cookies.txt");
        let line = |domain: &str, name: &str, value: &str| format!("{domain}	TRUE	/	TRUE	0	{name}	{value}");

        // A browser export, with its header, a blank line and an http-only cookie.
        let export = vec![
            "# Netscape HTTP Cookie File".to_string(),
            String::new(),
            format!("#HttpOnly_{}", line(".youtube.com", "SID", "one")),
            line(".google.com", "NID", "one"),
        ];
        assert_eq!(merge_cookies(&file, &export).unwrap(), 2);
        assert_eq!(signed_in_sites(&file), vec!["youtube.com"], "only real sign-in cookies count");
        merge_cookies(&file, &[line(".google.com", "SID", "one")]).unwrap();
        assert_eq!(signed_in_sites(&file), vec!["google.com", "youtube.com"]);

        // A later sign-in elsewhere keeps those, and a newer cookie replaces the old one.
        let later = vec![
            line(".x.com", "auth_token", "abc"),
            line(".google.com", "NID", "two"),
            // only visited, never signed in to
            line(".instagram.com", "csrftoken", "x"),
            line("github.com", "_octo", "x"),
        ];
        assert_eq!(merge_cookies(&file, &later).unwrap(), 4);
        assert_eq!(signed_in_sites(&file), vec!["google.com", "x.com", "youtube.com"]);
        let saved = std::fs::read_to_string(&file).unwrap();
        assert!(saved.starts_with("# Netscape HTTP Cookie File"));
        assert!(saved.contains("NID	two") && !saved.contains("NID	one"));
        assert!(saved.contains("#HttpOnly_.youtube.com"));
        assert_eq!(saved.lines().filter(|l| cookie_key(l).is_some()).count(), 6);

        // A file that is not a cookies file adds nothing.
        let notes = vec!["just some notes".to_string(), "https://example.com/a.zip".to_string()];
        assert_eq!(merge_cookies(&file, &notes).unwrap(), 0);
        assert_eq!(signed_in_sites(&file).len(), 3);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
