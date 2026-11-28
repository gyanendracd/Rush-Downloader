//! Site integrations: turns a share link or a project page into the files
//! behind it, with their names and sizes, so the user can pick which ones
//! to download. One small resolver per site, each reading only what the
//! site gives any visitor (its public API or its own download links).
//!
//! Nothing here gets around a sign-in, a payment or copy protection. Sites
//! that sell access or only hand files to their own app are listed in
//! [`kind`] as blocked, with the reason, so the app can say so plainly.
//!
//! Video and audio pages are not handled here: the site backend (yt-dlp,
//! gallery-dl) already reads them. The exceptions are video sites the
//! backend does not know whose pages name the uploaded file (PMVHaven).

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use base64::Engine as _;
use regex::Regex;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::Inner;

/// Most files one listing returns; a bigger folder is cut there.
const MAX_FILES: usize = 5000;

/// One file a site link holds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SiteFile {
    /// Folders inside the download's own folder, then the file's name.
    pub path: Vec<String>,
    /// A direct link to the file (or, with `late`, what to ask for one).
    pub url: String,
    pub size: Option<u64>,
    /// Sent with the download (a cookie the site handed out, a referer).
    #[serde(default)]
    pub headers: Vec<(String, String)>,
    /// For sites whose direct links expire, which resolver asks the site
    /// for a fresh one right before the file downloads.
    #[serde(default)]
    pub late: Option<String>,
    /// What that resolver needs (an id, a key). For MEGA, also the key the
    /// file is decrypted with once downloaded.
    #[serde(default)]
    pub meta: Option<String>,
}

/// What a site link holds.
#[derive(Clone, Debug, Serialize)]
pub struct SiteFiles {
    /// The site's name, as shown to the user.
    pub site: String,
    /// The collection's name: the folder its files are saved in.
    pub title: String,
    pub files: Vec<SiteFile>,
    /// Set when the list was cut short or some files were left out.
    pub partial: bool,
}

/// How the app handles a link of a known site.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Support {
    /// Read here: the user picks files from the list.
    Files,
    /// The site sells access or only lets its own app download.
    Blocked,
}

/// Why a site cannot be downloaded from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Why {
    /// Games: only the store's own app downloads them, with copy protection.
    Store,
    /// Paid courses: the videos are for paying, signed-in students only.
    Paid,
    /// Files are encrypted in the browser with a key the app never gets.
    Encrypted,
    /// A search engine: it points at other sites, which hold the files.
    Search,
    /// The site hands files only to its own web page (a wait, a click, a
    /// site-only key); download them there, in the browser.
    SiteOnly,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct SiteKind {
    pub site: &'static str,
    pub support: Support,
    pub why: Option<Why>,
}

/// Sites whose files cannot be downloaded by any app but their own, and why.
const BLOCKED: &[(&str, &str, Why)] = &[
    ("store.steampowered.com", "Steam", Why::Store),
    ("steampowered.com", "Steam", Why::Store),
    ("steamcommunity.com", "Steam", Why::Store),
    ("epicgames.com", "Epic Games", Why::Store),
    ("battle.net", "Battle.net", Why::Store),
    ("blizzard.com", "Battle.net", Why::Store),
    ("ea.com", "EA app", Why::Store),
    ("ubisoft.com", "Ubisoft Connect", Why::Store),
    ("ubisoftconnect.com", "Ubisoft Connect", Why::Store),
    ("rockstargames.com", "Rockstar Games", Why::Store),
    ("xbox.com", "Xbox", Why::Store),
    ("playstation.com", "PlayStation Store", Why::Store),
    ("coursera.org", "Coursera", Why::Paid),
    ("udemy.com", "Udemy", Why::Paid),
    ("skillshare.com", "Skillshare", Why::Paid),
    ("futurelearn.com", "FutureLearn", Why::Paid),
    ("wormhole.app", "Wormhole", Why::Encrypted),
    ("sync.com", "Sync.com", Why::Encrypted),
    ("datasetsearch.research.google.com", "Google Dataset Search", Why::Search),
    // Its API is kept for its own website; free visitors cannot list a folder from an app.
    ("gofile.io", "GoFile", Why::SiteOnly),
    // A wait and a click on the site itself come before each free download.
    ("1fichier.com", "1fichier", Why::SiteOnly),
];

/// A link the resolvers here know, read from the link alone.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Route {
    GitHub { owner: String, repo: String, tag: Option<String>, all: bool },
    GitHubBlob { owner: String, repo: String, rest: String },
    GitLab { path: String, tag: Option<String> },
    Bitbucket { owner: String, repo: String },
    HuggingFace { kind: &'static str, id: String },
    Kaggle { owner: String, slug: String },
    Zenodo { id: String },
    Figshare { id: String },
    Archive { id: String },
    Drive { id: String },
    DriveFolder { id: String },
    GoogleDoc { kind: &'static str, id: String },
    Dropbox { url: String, folder: bool },
    OneDrive { url: String },
    Box { shared: String, file: Option<String>, folder: Option<String> },
    MediaFire { url: String },
    MediaFireFolder { key: String },
    Mega { handle: String, key: String },
    MegaFolder { handle: String, key: String },
    WeTransfer { url: String },
    PCloud { code: String },
    SwissTransfer { id: String },
    Sendspace { url: String },
    Commons { title: String },
    Arxiv { id: String },
    Gutenberg { n: String },
    OpenLibrary { path: String },
    PolyHaven { id: String },
    Unsplash { id: String, slug: String },
    Pexels { id: String, slug: String, video: bool },
    SourceForge { project: String, path: Option<String> },
    Chocolatey { id: String, version: Option<String> },
    Ninite { apps: String },
    Ckan { base: String, id: String },
    EuData { id: String },
    WorldBank { code: String },
    AwsRegistry { name: String },
    S3 { bucket: String, prefix: String },
    Osm { kind: String, id: String },
    Ocw { slug: String },
    NasaImage { id: String },
    Dryad { doi: String },
    ArtStation { hash: String },
    PmvHaven { id: String },
    Itch { url: String },
    Listing { url: String },
}

/// Hosts whose folder pages ("Index of /...") are lists of files.
const LISTING_HOSTS: &[&str] = &[
    "noaa.gov",
    "usgs.gov",
    "nasa.gov",
    "geofabrik.de",
    "planet.openstreetmap.org",
    "ftp.gnu.org",
    "mirrors.kernel.org",
    "cdn.kernel.org",
];

fn host_is(host: &str, domain: &str) -> bool {
    host == domain || host.ends_with(&format!(".{domain}"))
}

fn segments(url: &Url) -> Vec<String> {
    url.path_segments()
        .map(|s| {
            s.filter(|p| !p.is_empty())
                .map(|p| percent_encoding::percent_decode_str(p).decode_utf8_lossy().to_string())
                .collect()
        })
        .unwrap_or_default()
}

fn query(url: &Url, name: &str) -> Option<String> {
    url.query_pairs().find(|(k, _)| k == name).map(|(_, v)| v.to_string())
}

/// Which resolver reads a link, if any.
fn route(link: &str) -> Option<Route> {
    let url = Url::parse(link.trim()).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let host = url.host_str()?.to_ascii_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host).to_string();
    let seg = segments(&url);
    let s = |i: usize| seg.get(i).map(String::as_str).unwrap_or("");
    let fragment = url.fragment().unwrap_or("").to_string();

    Some(match host.as_str() {
        "github.com" if seg.len() >= 2 => {
            let (owner, repo) = (s(0).to_string(), s(1).trim_end_matches(".git").to_string());
            // Not repositories: the site's own pages.
            if matches!(
                owner.as_str(),
                "orgs" | "settings" | "marketplace" | "topics" | "collections" | "sponsors" | "features" | "about" | "login"
            ) {
                return None;
            }
            match (s(2), s(3)) {
                ("", _) => Route::GitHub { owner, repo, tag: None, all: false },
                ("releases", "") => Route::GitHub { owner, repo, tag: None, all: true },
                ("releases", "latest") => Route::GitHub { owner, repo, tag: None, all: false },
                ("releases", "tag") if seg.len() >= 5 => {
                    Route::GitHub { owner, repo, tag: Some(seg[4..].join("/")), all: false }
                }
                ("blob", _) if seg.len() >= 5 => Route::GitHubBlob { owner, repo, rest: seg[3..].join("/") },
                ("tree", _) if seg.len() == 4 => Route::GitHub { owner, repo, tag: None, all: false },
                // A release file is a plain link; anything else is a page.
                _ => return None,
            }
        }
        "gitlab.com" if seg.len() >= 2 => {
            let dash = seg.iter().position(|p| p == "-");
            let project = &seg[..dash.unwrap_or(seg.len())];
            if project.len() < 2 {
                return None;
            }
            let after: Vec<&str> = dash.map(|d| seg[d + 1..].iter().map(String::as_str).collect()).unwrap_or_default();
            let tag = match after.as_slice() {
                [] | ["releases"] | ["tree", ..] => None,
                ["releases", "permalink", "latest", ..] => None,
                ["releases", tag, ..] => Some(tag.to_string()),
                _ => return None,
            };
            Route::GitLab { path: project.join("/"), tag }
        }
        "bitbucket.org" if seg.len() == 2 || (seg.len() == 3 && s(2) == "downloads") => {
            Route::Bitbucket { owner: s(0).into(), repo: s(1).into() }
        }
        "huggingface.co" | "hf.co" => {
            let (kind, rest) = match s(0) {
                "datasets" => ("datasets", &seg[1.min(seg.len())..]),
                "spaces" => ("spaces", &seg[1.min(seg.len())..]),
                "models" | "docs" | "blog" | "papers" | "login" | "join" | "pricing" | "settings" | "organizations" => {
                    return None
                }
                _ => ("models", &seg[..]),
            };
            if rest.len() < 2 || !matches!(rest.get(2).map(String::as_str), None | Some("tree") | Some("files")) {
                return None;
            }
            Route::HuggingFace { kind, id: format!("{}/{}", rest[0], rest[1]) }
        }
        "kaggle.com" if s(0) == "datasets" && seg.len() >= 3 => {
            Route::Kaggle { owner: s(1).into(), slug: s(2).into() }
        }
        "zenodo.org" if matches!(s(0), "records" | "record") && !s(1).is_empty() => Route::Zenodo { id: s(1).into() },
        "doi.org" if s(0) == "10.5281" && s(1).starts_with("zenodo.") => {
            Route::Zenodo { id: s(1).trim_start_matches("zenodo.").into() }
        }
        h if (host_is(h, "figshare.com")) && seg.iter().any(|p| p == "articles") => {
            let id = seg.iter().rev().find(|p| p.chars().all(|c| c.is_ascii_digit()) && p.len() >= 4)?;
            Route::Figshare { id: id.clone() }
        }
        "archive.org" if matches!(s(0), "details" | "download") && !s(1).is_empty() => {
            Route::Archive { id: s(1).into() }
        }
        "drive.google.com" => {
            if s(0) == "file" && s(1) == "d" && !s(2).is_empty() {
                Route::Drive { id: s(2).into() }
            } else if let Some(i) = seg.iter().position(|p| p == "folders") {
                Route::DriveFolder { id: seg.get(i + 1)?.clone() }
            } else if let Some(id) = query(&url, "id") {
                if s(0) == "embeddedfolderview" { Route::DriveFolder { id } } else { Route::Drive { id } }
            } else {
                return None;
            }
        }
        "drive.usercontent.google.com" => Route::Drive { id: query(&url, "id")? },
        "docs.google.com" => {
            let kind = match s(0) {
                "document" => "document",
                "spreadsheets" => "spreadsheets",
                "presentation" => "presentation",
                _ => return None,
            };
            let at = seg.iter().position(|p| p == "d")?;
            Route::GoogleDoc { kind, id: seg.get(at + 1)?.clone() }
        }
        "dropbox.com" | "dl.dropboxusercontent.com" if matches!(s(0), "s" | "scl" | "sh") => {
            let folder = s(0) == "sh" || (s(0) == "scl" && s(1) == "fo");
            Route::Dropbox { url: link.trim().to_string(), folder }
        }
        "1drv.ms" | "onedrive.live.com" => Route::OneDrive { url: link.trim().to_string() },
        h if host_is(h, "box.com") && s(0) == "s" && !s(1).is_empty() => Route::Box {
            shared: s(1).into(),
            file: (s(2) == "file").then(|| s(3).to_string()).filter(|f| !f.is_empty()),
            folder: (s(2) == "folder").then(|| s(3).to_string()).filter(|f| !f.is_empty()),
        },
        "mediafire.com" => match s(0) {
            "file" | "file_premium" | "view" if !s(1).is_empty() => Route::MediaFire { url: link.trim().to_string() },
            "folder" if !s(1).is_empty() => Route::MediaFireFolder { key: s(1).into() },
            _ if url.query().is_some_and(|q| !q.contains('=')) => {
                Route::MediaFire { url: link.trim().to_string() }
            }
            _ => return None,
        },
        "mega.nz" | "mega.co.nz" => {
            if matches!(s(0), "file" | "folder") && !s(1).is_empty() && !fragment.is_empty() {
                // A link into a folder ("#key/file/handle") opens that folder.
                let key = fragment.split('/').next().unwrap_or("").to_string();
                if s(0) == "file" {
                    Route::Mega { handle: s(1).into(), key }
                } else {
                    Route::MegaFolder { handle: s(1).into(), key }
                }
            } else if let Some(rest) = fragment.strip_prefix("!") {
                let (handle, key) = rest.split_once('!')?;
                Route::Mega { handle: handle.into(), key: key.into() }
            } else if let Some(rest) = fragment.strip_prefix("F!") {
                let (handle, key) = rest.split_once('!')?;
                Route::MegaFolder { handle: handle.into(), key: key.into() }
            } else {
                return None;
            }
        }
        "we.tl" | "wetransfer.com" if host == "we.tl" || matches!(s(0), "downloads") || host.ends_with(".wetransfer.com") => {
            Route::WeTransfer { url: link.trim().to_string() }
        }
        h if (host_is(h, "pcloud.link") || host_is(h, "pcloud.com")) && query(&url, "code").is_some() => {
            Route::PCloud { code: query(&url, "code")? }
        }
        "swisstransfer.com" if s(0) == "d" && !s(1).is_empty() => Route::SwissTransfer { id: s(1).into() },
        "sendspace.com" if s(0) == "file" && !s(1).is_empty() => Route::Sendspace { url: link.trim().to_string() },
        "commons.wikimedia.org" | "en.wikipedia.org" if s(0) == "wiki" && s(1).starts_with("File:") => {
            Route::Commons { title: s(1).into() }
        }
        "arxiv.org" | "export.arxiv.org" if matches!(s(0), "abs" | "pdf") && seg.len() >= 2 => {
            let id = seg[1..].join("/");
            Route::Arxiv { id: id.trim_end_matches(".pdf").into() }
        }
        "gutenberg.org" if s(0) == "ebooks" && s(1).chars().all(|c| c.is_ascii_digit()) && !s(1).is_empty() => {
            Route::Gutenberg { n: s(1).into() }
        }
        "openlibrary.org" if matches!(s(0), "books" | "works") && !s(1).is_empty() => {
            Route::OpenLibrary { path: format!("{}/{}", s(0), s(1)) }
        }
        "polyhaven.com" if s(0) == "a" && !s(1).is_empty() => Route::PolyHaven { id: s(1).into() },
        "unsplash.com" if s(0) == "photos" && !s(1).is_empty() => {
            let slug = s(1).to_string();
            let id = slug.rsplit('-').next().unwrap_or(&slug).to_string();
            Route::Unsplash { id, slug }
        }
        "pexels.com" if matches!(s(0), "photo" | "video") && !s(1).is_empty() => {
            let slug = s(1).to_string();
            let id = slug.rsplit('-').next().filter(|i| i.chars().all(|c| c.is_ascii_digit()))?.to_string();
            Route::Pexels { id, slug, video: s(0) == "video" }
        }
        "sourceforge.net" if s(0) == "projects" && !s(1).is_empty() => {
            let path = (s(2) == "files" && seg.len() > 3)
                .then(|| seg[3..].iter().filter(|p| *p != "download").cloned().collect::<Vec<_>>().join("/"))
                .filter(|p| !p.is_empty() && p != "latest");
            Route::SourceForge { project: s(1).into(), path }
        }
        "community.chocolatey.org" | "chocolatey.org" if s(0) == "packages" && !s(1).is_empty() => {
            Route::Chocolatey { id: s(1).into(), version: seg.get(2).cloned() }
        }
        "ninite.com" if !s(0).is_empty() && s(0) != "applist" && s(0) != "help" => Route::Ninite { apps: s(0).into() },
        "catalog.data.gov" if s(0) == "dataset" && !s(1).is_empty() => {
            Route::Ckan { base: "https://catalog.data.gov".into(), id: s(1).into() }
        }
        "data.europa.eu" if seg.iter().any(|p| p == "datasets") => {
            let at = seg.iter().position(|p| p == "datasets")?;
            Route::EuData { id: seg.get(at + 1)?.clone() }
        }
        "data.worldbank.org" if s(0) == "indicator" && !s(1).is_empty() => Route::WorldBank { code: s(1).into() },
        "registry.opendata.aws" if !s(0).is_empty() => Route::AwsRegistry { name: s(0).into() },
        h if h.ends_with(".s3.amazonaws.com") || (h.starts_with("s3.") && h.ends_with(".amazonaws.com")) => {
            if h.starts_with("s3.") || h.starts_with("s3-") {
                Route::S3 { bucket: s(0).into(), prefix: seg.get(1..).map(|p| p.join("/")).unwrap_or_default() }
            } else {
                let bucket = h.split(".s3").next()?.to_string();
                Route::S3 { bucket, prefix: seg.join("/") }
            }
        }
        "openstreetmap.org" if matches!(s(0), "node" | "way" | "relation") && !s(1).is_empty() => {
            Route::Osm { kind: s(0).into(), id: s(1).into() }
        }
        "ocw.mit.edu" if s(0) == "courses" && !s(1).is_empty() => Route::Ocw { slug: s(1).into() },
        "images.nasa.gov" if s(0) == "details" && !s(1).is_empty() => Route::NasaImage { id: s(1).into() },
        "images.nasa.gov" if s(0).starts_with("details-") => Route::NasaImage { id: s(0)["details-".len()..].into() },
        "datadryad.org" if seg.iter().any(|p| p.starts_with("doi:")) => {
            let at = seg.iter().position(|p| p.starts_with("doi:"))?;
            Route::Dryad { doi: seg[at..].join("/") }
        }
        "artstation.com" if s(0) == "artwork" && !s(1).is_empty() => Route::ArtStation { hash: s(1).into() },
        // "/video/<title slug>_<id>" or "/videos/<id>"
        "pmvhaven.com" if matches!(s(0), "video" | "videos") => {
            let id = s(1).rsplit('_').next()?;
            if id.len() != 24 || !id.chars().all(|c| c.is_ascii_hexdigit()) {
                return None;
            }
            Route::PmvHaven { id: id.to_ascii_lowercase() }
        }
        h if h.ends_with(".itch.io") && seg.len() == 1 => Route::Itch { url: link.trim().to_string() },
        h if LISTING_HOSTS.iter().any(|d| host_is(h, d)) && (url.path().ends_with('/') || url.path().is_empty()) => {
            Route::Listing { url: link.trim().to_string() }
        }
        _ => return None,
    })
}

fn site_name(route: &Route) -> &'static str {
    match route {
        Route::GitHub { .. } | Route::GitHubBlob { .. } => "GitHub",
        Route::GitLab { .. } => "GitLab",
        Route::Bitbucket { .. } => "Bitbucket",
        Route::HuggingFace { .. } => "Hugging Face",
        Route::Kaggle { .. } => "Kaggle",
        Route::Zenodo { .. } => "Zenodo",
        Route::Figshare { .. } => "Figshare",
        Route::Archive { .. } => "Internet Archive",
        Route::Drive { .. } | Route::DriveFolder { .. } | Route::GoogleDoc { .. } => "Google Drive",
        Route::Dropbox { .. } => "Dropbox",
        Route::OneDrive { .. } => "OneDrive",
        Route::Box { .. } => "Box",
        Route::MediaFire { .. } | Route::MediaFireFolder { .. } => "MediaFire",
        Route::Mega { .. } | Route::MegaFolder { .. } => "MEGA",
        Route::WeTransfer { .. } => "WeTransfer",
        Route::PCloud { .. } => "pCloud",
        Route::SwissTransfer { .. } => "SwissTransfer",
        Route::Sendspace { .. } => "Sendspace",
        Route::Commons { .. } => "Wikimedia Commons",
        Route::Arxiv { .. } => "arXiv",
        Route::Gutenberg { .. } => "Project Gutenberg",
        Route::OpenLibrary { .. } => "Open Library",
        Route::PolyHaven { .. } => "Poly Haven",
        Route::Unsplash { .. } => "Unsplash",
        Route::Pexels { .. } => "Pexels",
        Route::SourceForge { .. } => "SourceForge",
        Route::Chocolatey { .. } => "Chocolatey",
        Route::Ninite { .. } => "Ninite",
        Route::Ckan { .. } => "Data.gov",
        Route::EuData { .. } => "data.europa.eu",
        Route::WorldBank { .. } => "World Bank",
        Route::AwsRegistry { .. } | Route::S3 { .. } => "AWS Open Data",
        Route::Osm { .. } => "OpenStreetMap",
        Route::Ocw { .. } => "MIT OpenCourseWare",
        Route::NasaImage { .. } => "NASA Image Library",
        Route::Dryad { .. } => "Dryad",
        Route::ArtStation { .. } => "ArtStation",
        Route::PmvHaven { .. } => "PMVHaven",
        Route::Itch { .. } => "itch.io",
        Route::Listing { .. } => "Folder listing",
    }
}

/// What the app does with a link of a known site: lists its files, or says
/// why it cannot. `None` for every other link (a file, a video page...).
pub fn kind(link: &str) -> Option<SiteKind> {
    let url = Url::parse(link.trim()).ok()?;
    let host = url.host_str()?.to_ascii_lowercase();
    // LinkedIn Learning is a paid course site; the rest of LinkedIn is not.
    if host_is(&host, "linkedin.com") && url.path().starts_with("/learning") {
        return Some(SiteKind { site: "LinkedIn Learning", support: Support::Blocked, why: Some(Why::Paid) });
    }
    if host_is(&host, "proton.me") && url.path().starts_with("/drive") || host == "drive.proton.me" {
        return Some(SiteKind { site: "Proton Drive", support: Support::Blocked, why: Some(Why::Encrypted) });
    }
    if let Some((_, site, why)) = BLOCKED.iter().find(|(domain, _, _)| host_is(&host, domain)) {
        return Some(SiteKind { site, support: Support::Blocked, why: Some(*why) });
    }
    route(link).map(|r| SiteKind { site: site_name(&r), support: Support::Files, why: None })
}

/// Lists the files a link of a known site holds.
pub async fn resolve(inner: &Arc<Inner>, link: &str) -> Result<SiteFiles, String> {
    let route = route(link).ok_or_else(|| "RushDownload does not read this site's links".to_string())?;
    let site = site_name(&route).to_string();
    let web = Web { client: inner.client()? };
    let (title, mut files) = tokio::time::timeout(Duration::from_secs(60), read(&web, route))
        .await
        .map_err(|_| format!("{site} took too long to answer"))??;
    if files.is_empty() {
        return Err(format!("{site} lists no files at this link that can be downloaded without signing in"));
    }
    let partial = files.len() > MAX_FILES;
    files.truncate(MAX_FILES);
    fill_sizes(&web, &mut files).await;
    for f in files.iter_mut() {
        f.path = f.path.iter().map(|p| crate::probe::sanitize(p)).filter(|p| !p.is_empty()).collect();
        if f.path.is_empty() {
            f.path = vec!["file".into()];
        }
    }
    let title = crate::probe::sanitize(title.trim());
    Ok(SiteFiles { site, title: if title.is_empty() { "download".into() } else { title }, files, partial })
}

/// Asks the server for the size of files the site did not give one for,
/// when there are few enough to ask about quickly.
async fn fill_sizes(web: &Web, files: &mut [SiteFile]) {
    const MOST: usize = 60;
    let no_size: Vec<usize> = (0..files.len()).filter(|&i| files[i].size.is_none() && files[i].late.is_none()).collect();
    if no_size.is_empty() || no_size.len() > MOST {
        return;
    }
    let mut set = tokio::task::JoinSet::new();
    let limit = Arc::new(tokio::sync::Semaphore::new(10));
    for i in no_size {
        let (url, headers) = (files[i].url.clone(), files[i].headers.clone());
        let (client, limit) = (web.client.clone(), limit.clone());
        set.spawn(async move {
            let _turn = limit.acquire_owned().await;
            let asked = crate::probe::probe(&client, &url, &headers);
            let size = match tokio::time::timeout(Duration::from_secs(8), asked).await {
                Ok(Ok(p)) if !p.is_html => p.total.filter(|t| *t > 0),
                _ => None,
            };
            (i, size)
        });
    }
    let all = tokio::time::timeout(Duration::from_secs(20), async {
        let mut found = Vec::new();
        while let Some(Ok(answer)) = set.join_next().await {
            found.push(answer);
        }
        found
    })
    .await
    .unwrap_or_default();
    for (i, size) in all {
        files[i].size = size;
    }
}

/// A fresh direct link for a file whose link expires, right before it is
/// downloaded. Files with lasting links come back as they are.
/// `client`: the download's own (its connection).
pub(crate) async fn direct(client: reqwest::Client, file: &SiteFile) -> Result<(String, Vec<(String, String)>), String> {
    let web = Web { client };
    let url = match file.late.as_deref() {
        None => file.url.clone(),
        Some("mediafire") => mediafire_direct(&web, &file.url).await?,
        Some("mega") => mega_direct(&web, file.meta.as_deref().unwrap_or("")).await?,
        Some("pcloud") => pcloud_direct(&web, file.meta.as_deref().unwrap_or("")).await?,
        Some("wetransfer") => wetransfer_direct(&web, file.meta.as_deref().unwrap_or("")).await?,
        Some("sendspace") => sendspace_direct(&web, &file.url).await?,
        Some("itch") => itch_direct(&web, file.meta.as_deref().unwrap_or("")).await?,
        Some(other) => return Err(format!("unknown site step {other}")),
    };
    Ok((url, file.headers.clone()))
}

/* ---------- requests ---------- */

struct Web {
    /// The connection to ask the site over.
    client: reqwest::Client,
}

/// Some sites only serve pages to a browser.
const BROWSER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

impl Web {
    async fn send(&self, rb: reqwest::RequestBuilder) -> Result<reqwest::Response, String> {
        let resp = tokio::time::timeout(Duration::from_secs(30), rb.send())
            .await
            .map_err(|_| "the site took too long to answer".to_string())?
            .map_err(|e| format!("could not reach the site: {e}"))?;
        Ok(resp)
    }

    async fn json(&self, url: &str) -> Result<Value, String> {
        self.json_with(self.client.get(url)).await
    }

    async fn json_with(&self, rb: reqwest::RequestBuilder) -> Result<Value, String> {
        let resp = self.send(rb.header("Accept", "application/json")).await?;
        let status = resp.status();
        let text = resp.text().await.map_err(|e| format!("could not read the answer: {e}"))?;
        if !status.is_success() {
            return Err(http_error(status));
        }
        serde_json::from_str(&text).map_err(|_| "the site answered with something other than data".to_string())
    }

    fn post_json(&self, url: impl reqwest::IntoUrl, body: &Value) -> reqwest::RequestBuilder {
        self.client.post(url).header("Content-Type", "application/json").body(body.to_string())
    }

    async fn page(&self, url: &str) -> Result<String, String> {
        self.text(self.client.get(url).header("User-Agent", BROWSER_AGENT).header("Accept", "text/html,*/*")).await
    }

    async fn text(&self, rb: reqwest::RequestBuilder) -> Result<String, String> {
        let resp = self.send(rb).await?;
        let status = resp.status();
        if !status.is_success() {
            return Err(http_error(status));
        }
        resp.text().await.map_err(|e| format!("could not read the page: {e}"))
    }

    /// Where a link ends up after redirects, without reading its body.
    async fn final_url(&self, url: &str) -> Result<String, String> {
        let resp = self.send(self.client.get(url).header("User-Agent", BROWSER_AGENT)).await?;
        Ok(resp.url().to_string())
    }

    /// The file name and size a direct link gives, by asking for one byte.
    async fn head(&self, url: &str, headers: &[(String, String)]) -> (Option<String>, Option<u64>) {
        let asked = crate::probe::probe(&self.client, url, headers);
        match tokio::time::timeout(Duration::from_secs(15), asked).await {
            Ok(Ok(p)) if !p.is_html => (p.filename, p.total.filter(|t| *t > 0)),
            _ => (None, None),
        }
    }
}

fn http_error(status: reqwest::StatusCode) -> String {
    match status.as_u16() {
        401 | 403 => format!("the site refused (HTTP {}): the files are private or need a sign-in", status.as_u16()),
        404 | 410 => format!("nothing found at this link (HTTP {}): it may have been removed or expired", status.as_u16()),
        429 => "the site is limiting requests right now (HTTP 429); try again in a few minutes".into(),
        _ => format!("the site answered HTTP {status}"),
    }
}

fn file(path: Vec<String>, url: String, size: Option<u64>) -> SiteFile {
    SiteFile { path, url, size, headers: Vec::new(), late: None, meta: None }
}

fn one(name: &str, url: String, size: Option<u64>) -> SiteFile {
    file(vec![name.to_string()], url, size)
}

fn num(v: &Value) -> Option<u64> {
    v.as_u64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok())).or_else(|| v.as_f64().map(|f| f as u64))
}

fn text_of(v: &Value) -> String {
    v.as_str().unwrap_or("").to_string()
}

fn enc(part: &str) -> String {
    percent_encoding::utf8_percent_encode(part, percent_encoding::NON_ALPHANUMERIC).to_string()
}

/// A path's parts encoded for a link, slashes kept.
fn enc_path(path: &str) -> String {
    path.split('/').map(|p| {
        const KEEP: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
            .remove(b'-').remove(b'_').remove(b'.').remove(b'~');
        percent_encoding::utf8_percent_encode(p, KEEP).to_string()
    }).collect::<Vec<_>>().join("/")
}

fn split_path(path: &str) -> Vec<String> {
    path.split('/').filter(|p| !p.is_empty()).map(str::to_string).collect()
}

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("valid pattern")
}

fn html_unescape(s: &str) -> String {
    s.replace("&amp;", "&").replace("&quot;", "\"").replace("&#39;", "'").replace("&#x27;", "'").replace("&lt;", "<").replace("&gt;", ">")
}

/// A page's title: its `og:title`, else its `<title>`.
fn page_title(html: &str) -> Option<String> {
    static OG: OnceLock<Regex> = OnceLock::new();
    static TITLE: OnceLock<Regex> = OnceLock::new();
    let og = OG.get_or_init(|| re(r#"(?is)<meta[^>]+property=["']og:title["'][^>]+content=["']([^"']+)["']"#));
    let title = TITLE.get_or_init(|| re(r"(?is)<title[^>]*>(.*?)</title>"));
    og.captures(html)
        .or_else(|| title.captures(html))
        .map(|c| html_unescape(c[1].trim()))
        .filter(|t| !t.is_empty())
}

/* ---------- reading each site ---------- */

async fn read(web: &Web, route: Route) -> Result<(String, Vec<SiteFile>), String> {
    match route {
        Route::GitHub { owner, repo, tag, all } => github(web, &owner, &repo, tag, all).await,
        Route::GitHubBlob { owner, repo, rest } => {
            let url = format!("https://raw.githubusercontent.com/{owner}/{repo}/{}", enc_path(&rest));
            let name = rest.rsplit('/').next().unwrap_or("file").to_string();
            let (_, size) = web.head(&url, &[]).await;
            Ok((repo, vec![one(&name, url, size)]))
        }
        Route::GitLab { path, tag } => gitlab(web, &path, tag).await,
        Route::Bitbucket { owner, repo } => bitbucket(web, &owner, &repo).await,
        Route::HuggingFace { kind, id } => huggingface(web, kind, &id).await,
        Route::Kaggle { owner, slug } => {
            let url = format!("https://www.kaggle.com/api/v1/datasets/download/{owner}/{slug}");
            let (_, size) = web.head(&url, &[]).await;
            Ok((slug.clone(), vec![one(&format!("{slug}.zip"), url, size)]))
        }
        Route::Zenodo { id } => zenodo(web, &id).await,
        Route::Figshare { id } => figshare(web, &id).await,
        Route::Archive { id } => archive(web, &id).await,
        Route::Drive { id } => drive_file(web, &id).await,
        Route::DriveFolder { id } => drive_folder(web, &id).await,
        Route::GoogleDoc { kind, id } => Ok(google_doc(kind, &id)),
        Route::Dropbox { url, folder } => dropbox(web, &url, folder).await,
        Route::OneDrive { url } => onedrive(web, &url).await,
        Route::Box { shared, file, folder } => box_shared(web, &shared, file, folder).await,
        Route::MediaFire { url } => mediafire(web, &url).await,
        Route::MediaFireFolder { key } => mediafire_folder(web, &key).await,
        Route::Mega { handle, key } => mega_file(web, &handle, &key).await,
        Route::MegaFolder { handle, key } => mega_folder(web, &handle, &key).await,
        Route::WeTransfer { url } => wetransfer(web, &url).await,
        Route::PCloud { code } => pcloud(web, &code).await,
        Route::SwissTransfer { id } => swisstransfer(web, &id).await,
        Route::Sendspace { url } => sendspace(web, &url).await,
        Route::Commons { title } => commons(web, &title).await,
        Route::Arxiv { id } => Ok(arxiv(&id)),
        Route::Gutenberg { n } => gutenberg(web, &n).await,
        Route::OpenLibrary { path } => openlibrary(web, &path).await,
        Route::PolyHaven { id } => polyhaven(web, &id).await,
        Route::Unsplash { id, slug } => {
            let url = format!("https://unsplash.com/photos/{id}/download?force=true");
            let (_, size) = web.head(&url, &[]).await;
            Ok((slug.clone(), vec![one(&format!("{slug}.jpg"), url, size)]))
        }
        Route::Pexels { id, slug, video } => pexels(web, &id, &slug, video).await,
        Route::SourceForge { project, path } => sourceforge(web, &project, path).await,
        Route::Chocolatey { id, version } => {
            let url = match &version {
                Some(v) => format!("https://community.chocolatey.org/api/v2/package/{id}/{v}"),
                None => format!("https://community.chocolatey.org/api/v2/package/{id}"),
            };
            let (name, size) = web.head(&url, &[]).await;
            let name = name.unwrap_or_else(|| format!("{id}.nupkg"));
            Ok((id, vec![one(&name, url, size)]))
        }
        Route::Ninite { apps } => {
            let url = format!("https://ninite.com/{apps}/ninite.exe");
            let (name, size) = web.head(&url, &[]).await;
            Ok((format!("Ninite {apps}"), vec![one(&name.unwrap_or_else(|| "Ninite.exe".into()), url, size)]))
        }
        Route::Ckan { base, id } => ckan(web, &base, &id).await,
        Route::EuData { id } => eudata(web, &id).await,
        Route::WorldBank { code } => Ok(worldbank(&code)),
        Route::AwsRegistry { name } => aws_registry(web, &name).await,
        Route::S3 { bucket, prefix } => s3(web, &bucket, &prefix).await,
        Route::Osm { kind, id } => {
            let url = if kind == "node" {
                format!("https://api.openstreetmap.org/api/0.6/node/{id}")
            } else {
                format!("https://api.openstreetmap.org/api/0.6/{kind}/{id}/full")
            };
            Ok((format!("{kind} {id}"), vec![one(&format!("{kind}-{id}.osm"), url, None)]))
        }
        Route::Ocw { slug } => ocw(web, &slug).await,
        Route::NasaImage { id } => nasa_image(web, &id).await,
        Route::Dryad { doi } => dryad(web, &doi).await,
        Route::ArtStation { hash } => artstation(web, &hash).await,
        Route::PmvHaven { id } => pmvhaven(web, &id).await,
        Route::Itch { url } => itch(web, &url).await,
        Route::Listing { url } => listing(web, &url).await,
    }
}

/* GitHub: a repository's latest release (or a chosen one, or the last
   few), each with its files and the source code as ZIP and TAR.GZ. With
   no release, the source of the default branch. */
async fn github(web: &Web, owner: &str, repo: &str, tag: Option<String>, all: bool) -> Result<(String, Vec<SiteFile>), String> {
    let api = format!("https://api.github.com/repos/{owner}/{repo}");
    let releases: Vec<Value> = match (&tag, all) {
        (Some(tag), _) => vec![web.json(&format!("{api}/releases/tags/{}", enc_path(tag))).await?],
        (None, true) => web.json(&format!("{api}/releases?per_page=5")).await?.as_array().cloned().unwrap_or_default(),
        (None, false) => match web.json(&format!("{api}/releases/latest")).await {
            Ok(r) => vec![r],
            Err(e) if e.contains("404") => Vec::new(),
            Err(e) => return Err(e),
        },
    };
    let mut files = Vec::new();
    let several = releases.len() > 1;
    for rel in &releases {
        let tag = text_of(&rel["tag_name"]);
        let folder: Vec<String> = if several { vec![tag.clone()] } else { Vec::new() };
        let at = |name: &str| folder.iter().cloned().chain([name.to_string()]).collect::<Vec<_>>();
        for asset in rel["assets"].as_array().into_iter().flatten() {
            let name = text_of(&asset["name"]);
            files.push(file(at(&name), text_of(&asset["browser_download_url"]), num(&asset["size"])));
        }
        let plain = tag.trim_start_matches('v');
        files.push(file(at(&format!("{repo}-{plain}.zip")), format!("https://github.com/{owner}/{repo}/archive/refs/tags/{}.zip", enc_path(&tag)), None));
        files.push(file(at(&format!("{repo}-{plain}.tar.gz")), format!("https://github.com/{owner}/{repo}/archive/refs/tags/{}.tar.gz", enc_path(&tag)), None));
    }
    let title = match releases.as_slice() {
        [one] => format!("{repo} {}", text_of(&one["tag_name"])),
        _ => repo.to_string(),
    };
    if releases.is_empty() {
        let info = web.json(&api).await?;
        let branch = info["default_branch"].as_str().unwrap_or("main").to_string();
        files.push(one(&format!("{repo}-{branch}.zip"), format!("https://github.com/{owner}/{repo}/archive/refs/heads/{branch}.zip"), None));
        files.push(one(&format!("{repo}-{branch}.tar.gz"), format!("https://github.com/{owner}/{repo}/archive/refs/heads/{branch}.tar.gz"), None));
    }
    Ok((title, files))
}

async fn gitlab(web: &Web, path: &str, tag: Option<String>) -> Result<(String, Vec<SiteFile>), String> {
    let api = format!("https://gitlab.com/api/v4/projects/{}", enc(path));
    let project = web.json(&api).await?;
    let name = project["path"].as_str().unwrap_or(path).to_string();
    let release = match &tag {
        Some(t) => Some(web.json(&format!("{api}/releases/{}", enc(t))).await?),
        None => web.json(&format!("{api}/releases?per_page=1")).await?.as_array().and_then(|a| a.first().cloned()),
    };
    let mut files = Vec::new();
    match release {
        Some(rel) => {
            let tag = text_of(&rel["tag_name"]);
            for link in rel["assets"]["links"].as_array().into_iter().flatten() {
                let url = link["direct_asset_url"].as_str().or(link["url"].as_str()).unwrap_or("").to_string();
                let fname = link["name"].as_str().map(str::to_string).unwrap_or_else(|| crate::probe::url_filename(&url).unwrap_or_default());
                files.push(one(&fname, url, None));
            }
            for src in rel["assets"]["sources"].as_array().into_iter().flatten() {
                let format = text_of(&src["format"]);
                if matches!(format.as_str(), "zip" | "tar.gz") {
                    files.push(one(&format!("{name}-{tag}.{format}"), text_of(&src["url"]), None));
                }
            }
            Ok((format!("{name} {tag}"), files))
        }
        None => {
            let branch = project["default_branch"].as_str().unwrap_or("main").to_string();
            for format in ["zip", "tar.gz"] {
                files.push(one(
                    &format!("{name}-{branch}.{format}"),
                    format!("https://gitlab.com/{path}/-/archive/{branch}/{name}-{branch}.{format}"),
                    None,
                ));
            }
            Ok((name, files))
        }
    }
}

async fn bitbucket(web: &Web, owner: &str, repo: &str) -> Result<(String, Vec<SiteFile>), String> {
    let api = format!("https://api.bitbucket.org/2.0/repositories/{owner}/{repo}");
    let info = web.json(&api).await?;
    let mut files = Vec::new();
    if let Ok(downloads) = web.json(&format!("{api}/downloads?pagelen=100")).await {
        for d in downloads["values"].as_array().into_iter().flatten() {
            files.push(one(&text_of(&d["name"]), text_of(&d["links"]["self"]["href"]), num(&d["size"])));
        }
    }
    let branch = info["mainbranch"]["name"].as_str().unwrap_or("main").to_string();
    for format in ["zip", "tar.gz"] {
        files.push(one(&format!("{repo}-{branch}.{format}"), format!("https://bitbucket.org/{owner}/{repo}/get/{branch}.{format}"), None));
    }
    Ok((repo.to_string(), files))
}

async fn huggingface(web: &Web, kind: &str, id: &str) -> Result<(String, Vec<SiteFile>), String> {
    let tree = web.json(&format!("https://huggingface.co/api/{kind}/{id}/tree/main?recursive=true")).await?;
    let prefix = match kind {
        "datasets" => "datasets/",
        "spaces" => "spaces/",
        _ => "",
    };
    let files = tree
        .as_array()
        .into_iter()
        .flatten()
        .filter(|e| e["type"] == "file")
        .map(|e| {
            let path = text_of(&e["path"]);
            let size = num(&e["lfs"]["size"]).or_else(|| num(&e["size"]));
            file(split_path(&path), format!("https://huggingface.co/{prefix}{id}/resolve/main/{}", enc_path(&path)), size)
        })
        .collect();
    Ok((id.rsplit('/').next().unwrap_or(id).to_string(), files))
}

async fn zenodo(web: &Web, id: &str) -> Result<(String, Vec<SiteFile>), String> {
    let rec = web.json(&format!("https://zenodo.org/api/records/{id}")).await?;
    let files = rec["files"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|f| {
            let key = text_of(&f["key"]);
            let url = f["links"]["self"].as_str().map(str::to_string)
                .unwrap_or_else(|| format!("https://zenodo.org/records/{id}/files/{}?download=1", enc_path(&key)));
            file(split_path(&key), url, num(&f["size"]))
        })
        .collect();
    Ok((rec["metadata"]["title"].as_str().unwrap_or(id).to_string(), files))
}

async fn figshare(web: &Web, id: &str) -> Result<(String, Vec<SiteFile>), String> {
    let rec = web.json(&format!("https://api.figshare.com/v2/articles/{id}")).await?;
    let files = rec["files"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|f| f["is_link_only"] != true)
        .map(|f| one(&text_of(&f["name"]), text_of(&f["download_url"]), num(&f["size"])))
        .collect();
    Ok((rec["title"].as_str().unwrap_or(id).to_string(), files))
}

/// Internet Archive: every file of an item but the archive's own records
/// about it. Files of an item lent out to signed-in readers are left out.
async fn archive(web: &Web, id: &str) -> Result<(String, Vec<SiteFile>), String> {
    let meta = web.json(&format!("https://archive.org/metadata/{id}")).await?;
    if meta.as_object().is_none_or(|o| o.is_empty()) || meta["files"].is_null() {
        return Err("nothing found at this link: the item does not exist or is dark".into());
    }
    let title = match &meta["metadata"]["title"] {
        Value::Array(a) => a.first().map(text_of).unwrap_or_default(),
        v => text_of(v),
    };
    let files = meta["files"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|f| f["source"] != "metadata" && f["private"] != "true" && f["private"] != true)
        .filter(|f| {
            let name = f["name"].as_str().unwrap_or("");
            let lent = name.contains("_encrypted") || name.ends_with(".lcpdf") || name.contains("_lcp.");
            !name.ends_with("_archive.torrent") && !name.starts_with("__ia_thumb") && !lent
        })
        .map(|f| {
            let name = text_of(&f["name"]);
            file(split_path(&name), format!("https://archive.org/download/{id}/{}", enc_path(&name)), num(&f["size"]))
        })
        .collect();
    Ok((if title.is_empty() { id.to_string() } else { title }, files))
}

fn drive_link(id: &str) -> String {
    format!("https://drive.usercontent.google.com/download?id={id}&export=download&confirm=t")
}

async fn drive_file(web: &Web, id: &str) -> Result<(String, Vec<SiteFile>), String> {
    let url = drive_link(id);
    let (name, size) = web.head(&url, &[]).await;
    let Some(name) = name.filter(|n| n != "download") else {
        return Err("Google Drive did not hand out this file: it is private, or has been downloaded too often today".into());
    };
    Ok((name.rsplit_once('.').map(|(s, _)| s.to_string()).unwrap_or_else(|| name.clone()), vec![one(&name, url, size)]))
}

/// A shared Drive folder, through the page Drive shows for embedding it,
/// which lists its files to anyone with the link. Sub-folders are read too,
/// three levels down.
async fn drive_folder(web: &Web, id: &str) -> Result<(String, Vec<SiteFile>), String> {
    static ENTRY: OnceLock<Regex> = OnceLock::new();
    let entry = ENTRY.get_or_init(|| {
        re(r#"(?s)<div class="flip-entry" id="entry-([\w-]+)".*?<a href="([^"]+)".*?<div class="flip-entry-title">(.*?)</div>"#)
    });
    let mut files = Vec::new();
    let mut title = String::new();
    let mut queue = vec![(id.to_string(), Vec::<String>::new())];
    while let Some((folder, path)) = queue.pop() {
        let html = web.page(&format!("https://drive.google.com/embeddedfolderview?id={folder}")).await?;
        if path.is_empty() {
            title = page_title(&html).unwrap_or_else(|| id.to_string());
        }
        for c in entry.captures_iter(&html) {
            let (child, href, name) = (c[1].to_string(), html_unescape(&c[2]), html_unescape(c[3].trim()));
            if href.contains("/folders/") {
                if path.len() < 3 {
                    queue.push((child, path.iter().cloned().chain([name]).collect()));
                }
            } else if let Some((kind, ext)) = [("/document/", "docx"), ("/spreadsheets/", "xlsx"), ("/presentation/", "pptx")]
                .iter()
                .find(|(k, _)| href.contains(k))
            {
                let kind = kind.trim_matches('/');
                let url = format!("https://docs.google.com/{kind}/d/{child}/export?format={ext}");
                let named = if name.to_ascii_lowercase().ends_with(&format!(".{ext}")) { name } else { format!("{name}.{ext}") };
                files.push(file(path.iter().cloned().chain([named]).collect(), url, None));
            } else {
                files.push(file(path.iter().cloned().chain([name]).collect(), drive_link(&child), None));
            }
            if files.len() > MAX_FILES {
                break;
            }
        }
    }
    if files.is_empty() && title.is_empty() {
        return Err("Google Drive did not show this folder: it is private".into());
    }
    Ok((title, files))
}

/// A Google document, sheet or slide deck, in the formats Google exports.
fn google_doc(kind: &str, id: &str) -> (String, Vec<SiteFile>) {
    let formats: &[&str] = match kind {
        "document" => &["pdf", "docx", "odt", "txt", "epub"],
        "spreadsheets" => &["xlsx", "pdf", "ods", "csv"],
        _ => &["pptx", "pdf", "odp"],
    };
    let files = formats
        .iter()
        .map(|f| one(&format!("{kind}-{id}.{f}"), format!("https://docs.google.com/{kind}/d/{id}/export?format={f}"), None))
        .collect();
    (format!("{kind} {id}"), files)
}

async fn dropbox(web: &Web, link: &str, folder: bool) -> Result<(String, Vec<SiteFile>), String> {
    let mut url = Url::parse(link).map_err(|_| "not a valid link".to_string())?;
    let pairs: Vec<(String, String)> = url.query_pairs().filter(|(k, _)| k != "dl" && k != "raw").map(|(k, v)| (k.into(), v.into())).collect();
    url.query_pairs_mut().clear().extend_pairs(pairs).append_pair("dl", "1");
    let url = url.to_string();
    let (name, size) = web.head(&url, &[]).await;
    let name = name.ok_or("Dropbox did not hand out this link: it has been removed or is private")?;
    let title = if folder { name.trim_end_matches(".zip").to_string() } else { name.clone() };
    Ok((title, vec![one(&name, url, size)]))
}

/// OneDrive: a shared link's files, through OneDrive's public sharing API.
async fn onedrive(web: &Web, link: &str) -> Result<(String, Vec<SiteFile>), String> {
    let token = format!("u!{}", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(link.trim()));
    let api = format!("https://api.onedrive.com/v1.0/shares/{token}/root?$expand=children");
    match web.json(&api).await {
        Ok(root) => {
            let title = text_of(&root["name"]);
            let mut files = Vec::new();
            let items: Vec<Value> = match root["children"].as_array() {
                Some(children) if root["folder"].is_object() => children.clone(),
                _ => vec![root.clone()],
            };
            for item in items.iter().filter(|i| i["file"].is_object()) {
                let url = item["@content.downloadUrl"].as_str().map(str::to_string).unwrap_or_else(|| {
                    format!("https://api.onedrive.com/v1.0/shares/{token}/items/{}/content", text_of(&item["id"]))
                });
                files.push(one(&text_of(&item["name"]), url, num(&item["size"])));
            }
            Ok((title, files))
        }
        // Newer links: the page's own download address.
        Err(_) => {
            let landed = web.final_url(link).await?;
            let url = landed.replacen("/redir?", "/download?", 1).replacen("/embed?", "/download?", 1);
            let (name, size) = web.head(&url, &[]).await;
            let name = name.ok_or("OneDrive did not hand out this link without a sign-in")?;
            Ok((name.clone(), vec![one(&name, url, size)]))
        }
    }
}

/// Box: a shared file, or the files of a shared folder (two levels of
/// sub-folders down). Box hands its files to a browser only.
async fn box_shared(web: &Web, shared: &str, file_id: Option<String>, folder: Option<String>) -> Result<(String, Vec<SiteFile>), String> {
    let browser = vec![("User-Agent".to_string(), BROWSER_AGENT.to_string())];
    let link = |id: &str| format!("https://app.box.com/index.php?rm=box_download_shared_file&shared_name={shared}&file_id=f_{id}");
    if let Some(id) = file_id {
        let url = link(&id);
        let (name, size) = web.head(&url, &browser).await;
        let name = name.ok_or("Box did not hand out this file: it is private or the link has expired")?;
        let mut f = one(&name, url, size);
        f.headers = browser;
        return Ok((name, vec![f]));
    }
    static ITEM: OnceLock<Regex> = OnceLock::new();
    let item = ITEM.get_or_init(|| re(r#""typedID":"([fd])_(\d+)","type":"(?:file|folder)"[^{}]*?"name":"((?:[^"\\]|\\.)*)","itemSize":(\d+)"#));
    let mut files = Vec::new();
    let mut title = String::new();
    let mut queue = vec![(folder, Vec::<String>::new())];
    let mut pages = 0;
    while let Some((at, path)) = queue.pop() {
        pages += 1;
        if pages > 30 {
            break;
        }
        let page = match &at {
            Some(id) => format!("https://app.box.com/s/{shared}/folder/{id}"),
            None => format!("https://app.box.com/s/{shared}"),
        };
        let html = web.page(&page).await?;
        if title.is_empty() {
            title = page_title(&html).unwrap_or_else(|| shared.to_string()).trim_end_matches(" | Powered by Box").to_string();
        }
        let mut seen = std::collections::HashSet::new();
        for c in item.captures_iter(&html) {
            let (kind, id) = (&c[1], c[2].to_string());
            if !seen.insert(id.clone()) {
                continue;
            }
            let name = serde_json::from_str::<String>(&format!("\"{}\"", &c[3])).unwrap_or_else(|_| c[3].to_string());
            if kind == "d" {
                if path.len() < 2 {
                    queue.push((Some(id), path.iter().cloned().chain([name]).collect()));
                }
            } else {
                let mut f = file(path.iter().cloned().chain([name]).collect(), link(&id), c[4].parse().ok());
                f.headers = browser.clone();
                files.push(f);
            }
        }
    }
    Ok((title, files))
}

/* MediaFire: a file's page carries its download link; folders are listed by
   MediaFire's public API. Their direct links do not last, so each file's is
   read from its page right before it downloads. */
async fn mediafire(web: &Web, link: &str) -> Result<(String, Vec<SiteFile>), String> {
    let direct = mediafire_direct(web, link).await?;
    let (name, size) = web.head(&direct, &[]).await;
    let name = name.or_else(|| crate::probe::url_filename(&direct)).unwrap_or_else(|| "file".into());
    let mut f = one(&name, link.to_string(), size);
    f.late = Some("mediafire".into());
    Ok((name, vec![f]))
}

async fn mediafire_direct(web: &Web, page: &str) -> Result<String, String> {
    static LINK: OnceLock<Regex> = OnceLock::new();
    static SCRAMBLED: OnceLock<Regex> = OnceLock::new();
    let link = LINK.get_or_init(|| re(r#"href="(https?://download\d*\.mediafire\.com/[^"]+)""#));
    let scrambled = SCRAMBLED.get_or_init(|| re(r#"data-scrambled-url="([^"]+)""#));
    let html = web.page(page).await?;
    if let Some(c) = link.captures(&html) {
        return Ok(html_unescape(&c[1]));
    }
    if let Some(c) = scrambled.captures(&html) {
        if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(&c[1]) {
            if let Ok(url) = String::from_utf8(bytes) {
                return Ok(url);
            }
        }
    }
    Err("MediaFire shows no download for this file: it was removed, or is held for review".into())
}

async fn mediafire_folder(web: &Web, key: &str) -> Result<(String, Vec<SiteFile>), String> {
    let info = web
        .json(&format!("https://www.mediafire.com/api/1.5/folder/get_info.php?folder_key={key}&response_format=json"))
        .await
        .ok();
    let title = info.as_ref().map(|i| text_of(&i["response"]["folder_info"]["name"])).filter(|n| !n.is_empty()).unwrap_or_else(|| key.to_string());
    let mut files = Vec::new();
    let mut queue = vec![(key.to_string(), Vec::<String>::new())];
    while let Some((folder, path)) = queue.pop() {
        for (content, chunk) in [("files", 1), ("folders", 1)] {
            let api = format!(
                "https://www.mediafire.com/api/1.5/folder/get_content.php?folder_key={folder}&content_type={content}&chunk={chunk}&chunk_size=1000&response_format=json"
            );
            let data = web.json(&api).await?;
            let listing = &data["response"]["folder_content"];
            if content == "files" {
                for f in listing["files"].as_array().into_iter().flatten() {
                    let name = text_of(&f["filename"]);
                    let page = f["links"]["normal_download"].as_str().map(str::to_string)
                        .unwrap_or_else(|| format!("https://www.mediafire.com/file/{}", text_of(&f["quickkey"])));
                    let mut sf = file(path.iter().cloned().chain([name]).collect(), page, num(&f["size"]));
                    sf.late = Some("mediafire".into());
                    files.push(sf);
                }
            } else if path.len() < 3 {
                for f in listing["folders"].as_array().into_iter().flatten() {
                    queue.push((text_of(&f["folderkey"]), path.iter().cloned().chain([text_of(&f["name"])]).collect()));
                }
            }
        }
    }
    Ok((title, files))
}

/* ---------- MEGA ----------
   MEGA keeps every file encrypted (AES-128 in counter mode); the key is the
   part of the link after '#', which the browser never sends to MEGA. The
   app reads it the same way MEGA's own page does, downloads the encrypted
   file and decrypts it on this computer. */

fn mega_b64(s: &str) -> Result<Vec<u8>, String> {
    let clean: String = s.trim().trim_end_matches('=').replace('+', "-").replace('/', "_").replace(',', "");
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(clean)
        .map_err(|_| "the MEGA link's key is damaged; copy the whole link, including the part after #".to_string())
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn aes(key: &[u8]) -> aes::Aes128 {
    use aes::cipher::KeyInit;
    aes::Aes128::new_from_slice(key).expect("16-byte key")
}

fn aes_ecb_decrypt(key: &[u8], data: &[u8]) -> Vec<u8> {
    use aes::cipher::{generic_array::GenericArray, BlockDecrypt};
    let cipher = aes(key);
    let mut out = data.to_vec();
    for block in out.chunks_exact_mut(16) {
        cipher.decrypt_block(GenericArray::from_mut_slice(block));
    }
    out
}

/// A file's 32-byte key folded into the AES key and the counter's start.
fn mega_file_key(full: &[u8]) -> Result<([u8; 16], [u8; 8]), String> {
    if full.len() != 32 {
        return Err("the MEGA link's key is incomplete; copy the whole link".into());
    }
    let mut key = [0u8; 16];
    for i in 0..16 {
        key[i] = full[i] ^ full[i + 16];
    }
    let mut nonce = [0u8; 8];
    nonce.copy_from_slice(&full[16..24]);
    Ok((key, nonce))
}

/// A node's name, from its attributes (AES-CBC with a zero IV).
fn mega_name(attr: &str, key: &[u8]) -> Option<String> {
    use aes::cipher::{generic_array::GenericArray, BlockDecrypt};
    let data = mega_b64(attr).ok()?;
    let cipher = aes(key);
    let mut prev = [0u8; 16];
    let mut out = Vec::with_capacity(data.len());
    for block in data.chunks_exact(16) {
        let mut b = GenericArray::clone_from_slice(block);
        cipher.decrypt_block(&mut b);
        for i in 0..16 {
            out.push(b[i] ^ prev[i]);
        }
        prev.copy_from_slice(block);
    }
    let text = String::from_utf8_lossy(&out);
    let json = text.strip_prefix("MEGA")?.trim_end_matches('\0');
    serde_json::from_str::<Value>(json).ok()?["n"].as_str().map(str::to_string)
}

async fn mega_api(web: &Web, body: Value, folder: Option<&str>) -> Result<Value, String> {
    let url = match folder {
        Some(n) => format!("https://g.api.mega.co.nz/cs?id=0&n={n}"),
        None => "https://g.api.mega.co.nz/cs?id=0".to_string(),
    };
    let answer = web.json_with(web.post_json(url, &json!([body]))).await?;
    let first = answer.as_array().and_then(|a| a.first().cloned()).unwrap_or(answer);
    if let Some(code) = first.as_i64() {
        return Err(match code {
            -2 | -9 => "nothing found at this MEGA link: it was removed".into(),
            -11 => "MEGA refused: this link is not public".into(),
            -16 => "MEGA has taken this file down".into(),
            -17 | -18 => "MEGA is busy or this link went over its transfer limit; try again later".into(),
            other => format!("MEGA answered error {other}"),
        });
    }
    Ok(first)
}

async fn mega_file(web: &Web, handle: &str, key: &str) -> Result<(String, Vec<SiteFile>), String> {
    let full = mega_b64(key)?;
    let (aes_key, _) = mega_file_key(&full)?;
    let info = mega_api(web, json!({"a": "g", "p": handle}), None).await?;
    let name = mega_name(info["at"].as_str().unwrap_or(""), &aes_key).ok_or("the MEGA link's key does not fit this file")?;
    let mut f = one(&name, format!("https://mega.nz/file/{handle}"), num(&info["s"]));
    f.late = Some("mega".into());
    f.meta = Some(format!("p:{handle}:{}", b64(&full)));
    Ok((name.rsplit_once('.').map(|(s, _)| s.to_string()).unwrap_or_else(|| name.clone()), vec![f]))
}

async fn mega_folder(web: &Web, handle: &str, key: &str) -> Result<(String, Vec<SiteFile>), String> {
    let master = mega_b64(key)?;
    if master.len() != 16 {
        return Err("the MEGA folder link's key is incomplete; copy the whole link".into());
    }
    let listing = mega_api(web, json!({"a": "f", "c": 1, "r": 1, "ca": 1}), Some(handle)).await?;
    struct Node {
        parent: String,
        name: String,
        folder: bool,
        key: Vec<u8>,
        size: Option<u64>,
    }
    let mut nodes: HashMap<String, Node> = HashMap::new();
    let mut order = Vec::new();
    for n in listing["f"].as_array().into_iter().flatten() {
        let h = text_of(&n["h"]);
        let folder = n["t"] == 1;
        let attr = n["a"].as_str().unwrap_or("");
        let found = n["k"].as_str().unwrap_or("").split('/').find_map(|part| {
            let enc_key = mega_b64(part.split_once(':')?.1).ok()?;
            if enc_key.len() % 16 != 0 {
                return None;
            }
            let key = aes_ecb_decrypt(&master, &enc_key);
            let attr_key = if folder { key.get(..16).map(<[u8]>::to_vec) } else { mega_file_key(&key).ok().map(|(k, _)| k.to_vec()) }?;
            mega_name(attr, &attr_key).map(|name| (key, name))
        });
        let Some((key, name)) = found else { continue };
        order.push(h.clone());
        nodes.insert(h, Node { parent: text_of(&n["p"]), name, folder, key, size: num(&n["s"]) });
    }
    let root = order.iter().find(|h| nodes.get(*h).is_some_and(|n| n.folder && !nodes.contains_key(&n.parent))).cloned();
    let title = root.as_ref().and_then(|r| nodes.get(r)).map(|n| n.name.clone()).unwrap_or_else(|| handle.to_string());
    let mut files = Vec::new();
    for h in &order {
        let node = &nodes[h];
        if node.folder {
            continue;
        }
        let mut path = vec![node.name.clone()];
        let mut at = node.parent.clone();
        while let Some(p) = nodes.get(&at) {
            if Some(&at) == root.as_ref() {
                break;
            }
            path.insert(0, p.name.clone());
            at = p.parent.clone();
        }
        let mut f = file(path, format!("https://mega.nz/folder/{handle}"), node.size);
        f.late = Some("mega".into());
        f.meta = Some(format!("n:{handle}:{h}:{}", b64(&node.key)));
        files.push(f);
    }
    Ok((title, files))
}

async fn mega_direct(web: &Web, meta: &str) -> Result<String, String> {
    let parts: Vec<&str> = meta.split(':').collect();
    let info = match parts.as_slice() {
        ["p", handle, _] => mega_api(web, json!({"a": "g", "g": 1, "ssl": 1, "p": handle}), None).await?,
        ["n", folder, node, _] => mega_api(web, json!({"a": "g", "g": 1, "ssl": 1, "n": node}), Some(folder)).await?,
        _ => return Err("damaged MEGA file record".into()),
    };
    info["g"].as_str().map(str::to_string).filter(|g| !g.is_empty()).ok_or_else(|| "MEGA gave no download for this file".into())
}

/// The key a MEGA file is decrypted with, from its record.
pub(crate) fn mega_key(file: &SiteFile) -> Option<([u8; 16], [u8; 8])> {
    if file.late.as_deref() != Some("mega") {
        return None;
    }
    let key = file.meta.as_deref()?.rsplit(':').next()?;
    mega_file_key(&mega_b64(key).ok()?).ok()
}

/// Decrypts a downloaded MEGA file in place.
pub(crate) fn mega_decrypt(path: &std::path::Path, key: [u8; 16], nonce: [u8; 8]) -> std::io::Result<()> {
    use aes::cipher::{generic_array::GenericArray, BlockEncrypt};
    use std::io::{Read, Seek, SeekFrom, Write};
    let cipher = aes(&key);
    let mut f = std::fs::OpenOptions::new().read(true).write(true).open(path)?;
    let mut buf = vec![0u8; 1 << 20];
    let mut offset = 0u64;
    loop {
        f.seek(SeekFrom::Start(offset))?;
        let mut n = 0;
        while n < buf.len() {
            let got = f.read(&mut buf[n..])?;
            if got == 0 {
                break;
            }
            n += got;
        }
        if n == 0 {
            break;
        }
        let mut counter = offset / 16;
        for chunk in buf[..n].chunks_mut(16) {
            let mut block = [0u8; 16];
            block[..8].copy_from_slice(&nonce);
            block[8..].copy_from_slice(&counter.to_be_bytes());
            let mut b = GenericArray::from(block);
            cipher.encrypt_block(&mut b);
            for (byte, k) in chunk.iter_mut().zip(b.iter()) {
                *byte ^= k;
            }
            counter += 1;
        }
        f.seek(SeekFrom::Start(offset))?;
        f.write_all(&buf[..n])?;
        offset += n as u64;
    }
    f.sync_all()
}

/* ---------- transfer services ---------- */

async fn wetransfer(web: &Web, link: &str) -> Result<(String, Vec<SiteFile>), String> {
    let landed = web.final_url(link).await?;
    let url = Url::parse(&landed).map_err(|_| "not a valid link".to_string())?;
    let seg = segments(&url);
    let at = seg.iter().position(|p| p == "downloads").ok_or("WeTransfer did not open this transfer")?;
    let rest = &seg[at + 1..];
    let (id, recipient, hash) = match rest {
        [id, hash] => (id.clone(), None, hash.clone()),
        [id, recipient, hash, ..] => (id.clone(), Some(recipient.clone()), hash.clone()),
        _ => return Err("WeTransfer did not open this transfer: it has expired".into()),
    };
    let mut body = json!({"security_hash": hash});
    if let Some(r) = &recipient {
        body["recipient_id"] = json!(r);
    }
    let info = web
        .json_with(web.post_json(format!("https://wetransfer.com/api/v4/transfers/{id}/prepare-download"), &body))
        .await?;
    if info["state"].as_str().is_some_and(|s| s != "downloadable") {
        return Err("this WeTransfer transfer has expired or was deleted".into());
    }
    let files = info["items"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|item| {
            let mut f = one(&text_of(&item["name"]), landed.clone(), num(&item["size"]));
            f.late = Some("wetransfer".into());
            f.meta = Some(serde_json::to_string(&json!({"id": id, "hash": hash, "recipient": recipient, "file": item["id"]})).unwrap_or_default());
            f
        })
        .collect();
    Ok((info["display_name"].as_str().filter(|s| !s.is_empty()).unwrap_or("WeTransfer").to_string(), files))
}

async fn wetransfer_direct(web: &Web, meta: &str) -> Result<String, String> {
    let m: Value = serde_json::from_str(meta).map_err(|_| "damaged WeTransfer record".to_string())?;
    let mut body = json!({"security_hash": m["hash"], "intent": "single_file", "file_ids": [m["file"]]});
    if m["recipient"].is_string() {
        body["recipient_id"] = m["recipient"].clone();
    }
    let id = text_of(&m["id"]);
    let answer = web
        .json_with(web.post_json(format!("https://wetransfer.com/api/v4/transfers/{id}/download"), &body))
        .await?;
    answer["direct_link"].as_str().map(str::to_string).ok_or_else(|| "WeTransfer gave no download for this file".into())
}

async fn pcloud(web: &Web, code: &str) -> Result<(String, Vec<SiteFile>), String> {
    let mut last = String::new();
    for host in ["api.pcloud.com", "eapi.pcloud.com"] {
        let data = web.json(&format!("https://{host}/showpublink?code={code}")).await?;
        if data["result"] != 0 {
            last = text_of(&data["error"]);
            continue;
        }
        let meta = &data["metadata"];
        let mut files = Vec::new();
        fn walk(node: &Value, path: Vec<String>, host: &str, code: &str, files: &mut Vec<SiteFile>) {
            if node["isfolder"] == true {
                for child in node["contents"].as_array().into_iter().flatten() {
                    let mut p = path.clone();
                    if child["isfolder"] == true {
                        p.push(text_of(&child["name"]));
                    }
                    walk(child, p, host, code, files);
                }
            } else {
                let name = text_of(&node["name"]);
                let mut f = file(path.into_iter().chain([name]).collect(), format!("https://u.pcloud.link/publink/show?code={code}"), num(&node["size"]));
                f.late = Some("pcloud".into());
                f.meta = Some(format!("{host}|{code}|{}", num(&node["fileid"]).unwrap_or(0)));
                files.push(f);
            }
        }
        walk(meta, Vec::new(), host, code, &mut files);
        return Ok((text_of(&meta["name"]), files));
    }
    Err(format!("pCloud did not open this link: {last}"))
}

async fn pcloud_direct(web: &Web, meta: &str) -> Result<String, String> {
    let mut it = meta.split('|');
    let (host, code, id) = (it.next().unwrap_or(""), it.next().unwrap_or(""), it.next().unwrap_or(""));
    let mut api = format!("https://{host}/getpublinkdownload?code={code}&forcedownload=1");
    if id != "0" {
        api.push_str(&format!("&fileid={id}"));
    }
    let data = web.json(&api).await?;
    let server = data["hosts"].as_array().and_then(|h| h.first()).and_then(Value::as_str).ok_or("pCloud gave no download for this file")?;
    Ok(format!("https://{server}{}", text_of(&data["path"])))
}

async fn swisstransfer(web: &Web, id: &str) -> Result<(String, Vec<SiteFile>), String> {
    let data = web.json(&format!("https://www.swisstransfer.com/api/links/{id}")).await?;
    let d = &data["data"];
    if d["container"].is_null() {
        return Err("this SwissTransfer link has expired or has a password".into());
    }
    let host = d["downloadHost"].as_str().unwrap_or("dl.swisstransfer.com").to_string();
    let link = d["linkUUID"].as_str().unwrap_or(id).to_string();
    let files = d["container"]["files"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|f| {
            one(
                &text_of(&f["fileName"]),
                format!("https://{host}/api/download/{link}/{}", text_of(&f["UUID"])),
                num(&f["fileSizeInBytes"]),
            )
        })
        .collect();
    let title = d["container"]["message"].as_str().filter(|m| !m.is_empty() && m.len() < 80).unwrap_or("SwissTransfer").to_string();
    Ok((title, files))
}

async fn sendspace(web: &Web, link: &str) -> Result<(String, Vec<SiteFile>), String> {
    let direct = sendspace_direct(web, link).await?;
    let (name, size) = web.head(&direct, &[]).await;
    let name = name.or_else(|| crate::probe::url_filename(&direct)).unwrap_or_else(|| "file".into());
    let mut f = one(&name, link.to_string(), size);
    f.late = Some("sendspace".into());
    Ok((name, vec![f]))
}

async fn sendspace_direct(web: &Web, page: &str) -> Result<String, String> {
    static LINK: OnceLock<Regex> = OnceLock::new();
    let link = LINK.get_or_init(|| re(r#"id="download_button"\s+href="([^"]+)"|href="([^"]+)"[^>]*id="download_button""#));
    let html = web.page(page).await?;
    link.captures(&html)
        .and_then(|c| c.get(1).or(c.get(2)))
        .map(|m| html_unescape(m.as_str()))
        .ok_or_else(|| "Sendspace shows no download for this file: it was removed".into())
}

/* ---------- images, books, research ---------- */

async fn commons(web: &Web, title: &str) -> Result<(String, Vec<SiteFile>), String> {
    let api = format!(
        "https://commons.wikimedia.org/w/api.php?action=query&titles={}&prop=imageinfo&iiprop=url%7Csize&format=json",
        enc(title)
    );
    let data = web.json(&api).await?;
    let mut files = Vec::new();
    for page in data["query"]["pages"].as_object().into_iter().flat_map(|m| m.values()) {
        for info in page["imageinfo"].as_array().into_iter().flatten() {
            let url = text_of(&info["url"]);
            let name = crate::probe::url_filename(&url).unwrap_or_else(|| title.trim_start_matches("File:").to_string());
            files.push(one(&name, url, num(&info["size"])));
        }
    }
    Ok((title.trim_start_matches("File:").to_string(), files))
}

fn arxiv(id: &str) -> (String, Vec<SiteFile>) {
    let name = id.replace('/', "_");
    (
        format!("arXiv {id}"),
        vec![
            one(&format!("{name}.pdf"), format!("https://arxiv.org/pdf/{id}"), None),
            one(&format!("{name}-source.tar.gz"), format!("https://arxiv.org/src/{id}"), None),
        ],
    )
}

/// Project Gutenberg: every format of a book, from its own mirror of the
/// book's folder (the main site turns many networks away).
async fn gutenberg(web: &Web, n: &str) -> Result<(String, Vec<SiteFile>), String> {
    let folder = format!("https://gutenberg.pglaf.org/cache/epub/{n}/");
    let (_, files) = listing(web, &folder).await?;
    let books: Vec<SiteFile> = files
        .into_iter()
        .filter(|f| {
            let name = f.path.last().map(String::as_str).unwrap_or("");
            [".epub", ".mobi", ".txt", "-h.zip", "-images.html"].iter().any(|ext| name.ends_with(ext)) && !name.contains("qrcode")
        })
        .collect();
    static TITLE: OnceLock<Regex> = OnceLock::new();
    let title_re = TITLE.get_or_init(|| re(r"(?s)<dcterms:title>(.*?)</dcterms:title>"));
    let rdf = web.text(web.client.get(format!("{folder}pg{n}.rdf"))).await.unwrap_or_default();
    let title = title_re.captures(&rdf).map(|c| html_unescape(c[1].trim())).unwrap_or_else(|| format!("Gutenberg {n}"));
    Ok((title.lines().next().unwrap_or("").to_string(), books))
}

/// Open Library: books scanned by the Internet Archive that anyone may read.
async fn openlibrary(web: &Web, path: &str) -> Result<(String, Vec<SiteFile>), String> {
    let mut rec = web.json(&format!("https://openlibrary.org/{path}.json")).await?;
    if path.starts_with("works/") {
        let editions = web.json(&format!("https://openlibrary.org/{path}/editions.json?limit=50")).await?;
        if let Some(ed) = editions["entries"].as_array().into_iter().flatten().find(|e| e["ocaid"].is_string()) {
            rec = ed.clone();
        }
    }
    let Some(ocaid) = rec["ocaid"].as_str() else {
        return Err("Open Library has no scan of this book that can be downloaded".into());
    };
    let (_, files) = archive(web, ocaid).await?;
    let readable = [".pdf", ".epub", ".txt", ".djvu", ".zip", ".mobi"];
    if !files.iter().any(|f| f.path.last().is_some_and(|n| readable.iter().any(|e| n.to_ascii_lowercase().ends_with(e)))) {
        return Err("this book is lent to signed-in readers only; Open Library does not let it be downloaded".into());
    }
    Ok((rec["title"].as_str().unwrap_or(ocaid).to_string(), files))
}

/// Poly Haven: every format and resolution of a model, texture or HDRI.
async fn polyhaven(web: &Web, id: &str) -> Result<(String, Vec<SiteFile>), String> {
    let data = web.json(&format!("https://api.polyhaven.com/files/{id}")).await?;
    let mut files = Vec::new();
    fn walk(v: &Value, path: Vec<String>, files: &mut Vec<SiteFile>) {
        if let (Some(url), Some(size)) = (v["url"].as_str(), num(&v["size"])) {
            let name = crate::probe::url_filename(url).unwrap_or_else(|| "file".into());
            let mut p = path.clone();
            p.push(name);
            files.push(file(p, url.to_string(), Some(size)));
            for (k, inner) in v["include"].as_object().into_iter().flatten() {
                let mut p = path.clone();
                p.extend(split_path(k));
                if let Some(u) = inner["url"].as_str() {
                    files.push(file(p, u.to_string(), num(&inner["size"])));
                }
            }
            return;
        }
        if let Some(obj) = v.as_object() {
            for (k, child) in obj {
                let mut p = path.clone();
                p.push(k.clone());
                walk(child, p, files);
            }
        }
    }
    walk(&data, Vec::new(), &mut files);
    Ok((id.to_string(), files))
}

async fn pexels(web: &Web, id: &str, slug: &str, video: bool) -> Result<(String, Vec<SiteFile>), String> {
    let (name, url) = if video {
        (format!("{slug}.mp4"), format!("https://www.pexels.com/download/video/{id}/"))
    } else {
        (format!("{slug}.jpeg"), format!("https://images.pexels.com/photos/{id}/pexels-photo-{id}.jpeg"))
    };
    let (_, size) = web.head(&url, &[]).await;
    Ok((slug.to_string(), vec![one(&name, url, size)]))
}

async fn sourceforge(web: &Web, project: &str, path: Option<String>) -> Result<(String, Vec<SiteFile>), String> {
    let url = match &path {
        Some(p) => format!("https://downloads.sourceforge.net/project/{project}/{}", enc_path(p)),
        None => format!("https://sourceforge.net/projects/{project}/files/latest/download"),
    };
    let (name, size) = web.head(&url, &[]).await;
    let name = name
        .or_else(|| path.as_deref().and_then(|p| p.rsplit('/').next()).map(str::to_string))
        .ok_or("SourceForge did not hand out a file for this project")?;
    Ok((project.to_string(), vec![one(&name, url, size)]))
}

/* ---------- open data ---------- */

async fn ckan(web: &Web, base: &str, id: &str) -> Result<(String, Vec<SiteFile>), String> {
    let data = match web.json(&format!("{base}/api/3/action/package_show?id={}", enc(id))).await {
        Ok(d) if d["result"].is_object() => d,
        // The catalog's own page lists each data file's download link.
        _ => return dataset_page(web, &format!("{base}/dataset/{id}"), id).await,
    };
    let r = &data["result"];
    let files = r["resources"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|res| {
            let url = res["url"].as_str().filter(|u| u.starts_with("http"))?.to_string();
            let fname = crate::probe::url_filename(&url).filter(|n| n.contains('.')).unwrap_or_else(|| {
                let format = res["format"].as_str().unwrap_or("").to_ascii_lowercase();
                let base = res["name"].as_str().unwrap_or("resource");
                if format.is_empty() { base.to_string() } else { format!("{base}.{format}") }
            });
            Some(one(&fname, url, num(&res["size"])))
        })
        .collect();
    Ok((r["title"].as_str().unwrap_or(id).to_string(), files))
}

/// A data set's page: the files it links to on other servers.
async fn dataset_page(web: &Web, page: &str, id: &str) -> Result<(String, Vec<SiteFile>), String> {
    let html = web.page(page).await?;
    let here = Url::parse(page).map_err(|_| "not a valid link".to_string())?;
    let host = here.host_str().unwrap_or("").to_string();
    static HREF: OnceLock<Regex> = OnceLock::new();
    let href = HREF.get_or_init(|| re(r#"href="(https?://[^"]+)""#));
    const DATA: &[&str] = &[
        "csv", "json", "geojson", "xml", "xls", "xlsx", "zip", "gz", "kml", "kmz", "pdf", "txt", "parquet", "shp", "tsv", "rdf", "nc",
    ];
    let mut files: Vec<SiteFile> = Vec::new();
    for c in href.captures_iter(&html) {
        let url = html_unescape(&c[1]);
        let Ok(parsed) = Url::parse(&url) else { continue };
        if parsed.host_str().is_none_or(|h| h == host) {
            continue;
        }
        let name = crate::probe::url_filename(&url).unwrap_or_default();
        let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
        let wanted = DATA.contains(&ext.as_str()) || url.contains("accessType=DOWNLOAD") || url.contains("/download");
        if !wanted || files.iter().any(|f| f.url == url) {
            continue;
        }
        let name = if files.iter().any(|f| f.path.last() == Some(&name)) || name.is_empty() {
            format!("{id}-{}{}", files.len() + 1, if ext.is_empty() { String::new() } else { format!(".{ext}") })
        } else {
            name
        };
        files.push(one(&name, url, None));
    }
    Ok((page_title(&html).unwrap_or_else(|| id.to_string()), files))
}

async fn eudata(web: &Web, id: &str) -> Result<(String, Vec<SiteFile>), String> {
    let data = web.json(&format!("https://data.europa.eu/api/hub/search/datasets/{}", enc(id))).await?;
    let r = &data["result"];
    let pick = |v: &Value| -> String {
        v.get("en").and_then(Value::as_str).map(str::to_string)
            .or_else(|| v.as_object().and_then(|o| o.values().next()).and_then(Value::as_str).map(str::to_string))
            .unwrap_or_default()
    };
    let mut files = Vec::new();
    for d in r["distributions"].as_array().into_iter().flatten() {
        let url = d["download_url"].as_array().and_then(|a| a.first()).and_then(Value::as_str)
            .or_else(|| d["access_url"].as_array().and_then(|a| a.first()).and_then(Value::as_str));
        let Some(url) = url else { continue };
        let format = d["format"]["id"].as_str().unwrap_or("").to_ascii_lowercase();
        // Map and query services answer questions; they are not files.
        let service = ["wms", "wfs", "wmts", "wcs", "api", "sparql", "html", "rest", "service"].contains(&format.as_str());
        if service && d["download_url"].as_array().is_none_or(|a| a.is_empty()) {
            continue;
        }
        let label = pick(&d["title"]);
        let fname = crate::probe::url_filename(url).filter(|n| n.contains('.')).unwrap_or_else(|| {
            let base = if label.is_empty() { "resource".to_string() } else { label.clone() };
            if format.is_empty() { base } else { format!("{base}.{format}") }
        });
        files.push(one(&fname, url.to_string(), num(&d["byte_size"])));
    }
    Ok((pick(&r["title"]), files))
}

fn worldbank(code: &str) -> (String, Vec<SiteFile>) {
    let files = [("csv", "zip"), ("excel", "xls"), ("xml", "zip")]
        .iter()
        .map(|(format, ext)| {
            one(
                &format!("{code}-{format}.{ext}"),
                format!("https://api.worldbank.org/v2/en/indicator/{code}?downloadformat={format}"),
                None,
            )
        })
        .collect();
    (code.to_string(), files)
}

/// The AWS Open Data Registry: a data set's public S3 buckets, listed.
async fn aws_registry(web: &Web, name: &str) -> Result<(String, Vec<SiteFile>), String> {
    let yaml = web
        .text(web.client.get(format!("https://raw.githubusercontent.com/awslabs/open-data-registry/main/datasets/{name}.yaml")))
        .await?;
    static ARN: OnceLock<Regex> = OnceLock::new();
    let arn = ARN.get_or_init(|| re(r"arn:aws:s3:::([a-z0-9.\-]+)(?:/([^\s]*))?"));
    let title = yaml.lines().find_map(|l| l.strip_prefix("Name:")).map(|t| t.trim().to_string()).unwrap_or_else(|| name.to_string());
    let Some(c) = arn.captures(&yaml) else {
        return Err("this data set is not kept in a public S3 bucket".into());
    };
    let (_, files) = s3(web, &c[1], c.get(2).map_or("", |m| m.as_str())).await?;
    Ok((title, files))
}

async fn s3(web: &Web, bucket: &str, prefix: &str) -> Result<(String, Vec<SiteFile>), String> {
    let prefix = prefix.trim_start_matches('/');
    let api = format!("https://{bucket}.s3.amazonaws.com/?list-type=2&max-keys=1000&prefix={}", enc(prefix));
    let xml = web.text(web.client.get(api)).await?;
    static ITEM: OnceLock<Regex> = OnceLock::new();
    let item = ITEM.get_or_init(|| re(r"(?s)<Contents>.*?<Key>(.*?)</Key>.*?<Size>(\d+)</Size>.*?</Contents>"));
    let files = item
        .captures_iter(&xml)
        .filter(|c| !c[1].ends_with('/'))
        .map(|c| {
            let key = html_unescape(&c[1]);
            let rel = key.strip_prefix(prefix).unwrap_or(&key).trim_start_matches('/').to_string();
            file(split_path(&rel), format!("https://{bucket}.s3.amazonaws.com/{}", enc_path(&key)), c[2].parse().ok())
        })
        .collect();
    let title = if prefix.is_empty() { bucket.to_string() } else { prefix.trim_end_matches('/').rsplit('/').next().unwrap_or(bucket).to_string() };
    Ok((title, files))
}

/// MIT OpenCourseWare: a course's download page offers the whole course
/// as one ZIP; its videos are on YouTube and the Internet Archive.
async fn ocw(web: &Web, slug: &str) -> Result<(String, Vec<SiteFile>), String> {
    let html = web.page(&format!("https://ocw.mit.edu/courses/{slug}/download/")).await?;
    static ZIP: OnceLock<Regex> = OnceLock::new();
    let zip = ZIP.get_or_init(|| re(r#"href="([^"]+\.zip)""#));
    let mut files: Vec<SiteFile> = zip
        .captures_iter(&html)
        .map(|c| {
            let href = html_unescape(&c[1]);
            let url = if href.starts_with("http") { href } else { format!("https://ocw.mit.edu{href}") };
            let name = crate::probe::url_filename(&url).unwrap_or_else(|| format!("{slug}.zip"));
            one(&name, url, None)
        })
        .collect();
    files.dedup_by(|a, b| a.url == b.url);
    for f in files.iter_mut() {
        f.size = web.head(&f.url, &[]).await.1;
    }
    Ok((page_title(&html).map(|t| t.replace(" | MIT OpenCourseWare", "")).unwrap_or_else(|| slug.to_string()), files))
}

/// itch.io: the downloads of a game its maker gives away. Paid games (and
/// ones that ask for a payment first) are left alone.
async fn itch(web: &Web, page: &str) -> Result<(String, Vec<SiteFile>), String> {
    let html = web.page(page).await?;
    static UPLOAD: OnceLock<Regex> = OnceLock::new();
    let upload = UPLOAD.get_or_init(|| {
        re(r#"(?s)data-upload_id="(\d+)".*?<strong[^>]*class="name"[^>]*title="([^"]+)".*?(?:<span class="file_size"><span>([^<]+)</span>)?"#)
    });
    let base = page.trim_end_matches('/');
    let files: Vec<SiteFile> = upload
        .captures_iter(&html)
        .map(|c| {
            let mut f = one(&html_unescape(&c[2]), base.to_string(), c.get(3).and_then(|s| parse_size(s.as_str())));
            f.late = Some("itch".into());
            f.meta = Some(format!("{base}|{}", &c[1]));
            f
        })
        .collect();
    if files.is_empty() && (html.contains("buy_row") || html.contains("Buy Now")) {
        return Err("this itch.io game is paid; buy it on itch.io first".into());
    }
    Ok((page_title(&html).unwrap_or_else(|| "itch.io".into()).split(" by ").next().unwrap_or("itch.io").to_string(), files))
}

async fn itch_direct(web: &Web, meta: &str) -> Result<String, String> {
    let (page, upload) = meta.split_once('|').ok_or("damaged itch.io record")?;
    let answer = web
        .json_with(web.client.post(format!("{page}/file/{upload}?source=view_game&as_props=1&after_download_lightbox=true")).header("User-Agent", BROWSER_AGENT))
        .await?;
    answer["url"].as_str().map(str::to_string).ok_or_else(|| "itch.io gave no download for this file; it may need a payment first".into())
}

fn parse_size(s: &str) -> Option<u64> {
    let s = s.trim();
    let (n, unit) = s.split_at(s.find(|c: char| c.is_ascii_alphabetic())?);
    let n: f64 = n.trim().parse().ok()?;
    let mult = match unit.trim().to_ascii_lowercase().as_str() {
        "b" | "bytes" => 1.0,
        "kb" | "kib" => 1024.0,
        "mb" | "mib" => 1048576.0,
        "gb" | "gib" => 1073741824.0,
        _ => return None,
    };
    Some((n * mult) as u64)
}

/// NASA's image and video library: each size of a picture or video.
async fn nasa_image(web: &Web, id: &str) -> Result<(String, Vec<SiteFile>), String> {
    let data = web.json(&format!("https://images-api.nasa.gov/asset/{}", enc(id))).await?;
    let files = data["collection"]["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|i| i["href"].as_str())
        .filter(|h| !h.ends_with(".json") && !h.ends_with(".srt") && !h.ends_with(".vtt"))
        .map(|h| {
            let url = h.replacen("http://", "https://", 1).replace(' ', "%20");
            let name = crate::probe::url_filename(&url).unwrap_or_else(|| id.to_string());
            one(&name, url, None)
        })
        .collect();
    Ok((id.to_string(), files))
}

/// Dryad: the files of a data set's latest version.
async fn dryad(web: &Web, doi: &str) -> Result<(String, Vec<SiteFile>), String> {
    let base = "https://datadryad.org";
    let set = web.json(&format!("{base}/api/v2/datasets/{}", enc(doi))).await?;
    let version = set["_links"]["stash:version"]["href"].as_str().ok_or("Dryad lists no published version of this data set")?;
    let listed = web.json(&format!("{base}{version}/files?per_page=100")).await?;
    let files = listed["_embedded"]["stash:files"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|f| {
            let href = f["_links"]["stash:download"]["href"].as_str()?;
            Some(one(&text_of(&f["path"]), format!("{base}{href}"), num(&f["size"])))
        })
        .collect();
    Ok((set["title"].as_str().unwrap_or(doi).to_string(), files))
}

/// ArtStation: the pictures of one artwork, at full size.
async fn artstation(web: &Web, hash: &str) -> Result<(String, Vec<SiteFile>), String> {
    let rb = web.client.get(format!("https://www.artstation.com/projects/{hash}.json")).header("User-Agent", BROWSER_AGENT);
    let data = web.json_with(rb).await?;
    let files = data["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|a| a["has_image"] == true)
        .filter_map(|a| {
            let url = a["image_url"].as_str()?.to_string();
            let name = crate::probe::url_filename(&url).unwrap_or_else(|| format!("{hash}.jpg"));
            Some(one(&name, url, None))
        })
        .collect();
    Ok((data["title"].as_str().unwrap_or(hash).to_string(), files))
}

/// PMVHaven: the video file as it was uploaded. The site's player only
/// plays it as a stream, but the page's own data names the original file.
async fn pmvhaven(web: &Web, id: &str) -> Result<(String, Vec<SiteFile>), String> {
    let html = web.page(&format!("https://pmvhaven.com/videos/{id}")).await?;
    let video = pmvhaven_video(&html, id).ok_or("PMVHaven's page does not name this video's file")?;
    let ext = crate::probe::url_filename(&video.url)
        .and_then(|n| n.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()))
        .filter(|e| !e.is_empty() && e.len() <= 5)
        .unwrap_or_else(|| "mp4".into());
    let title = video.title.unwrap_or_else(|| id.to_string());
    Ok((title.clone(), vec![one(&format!("{title}.{ext}"), video.url, video.size)]))
}

struct PmvVideo {
    title: Option<String>,
    url: String,
    size: Option<u64>,
}

/// The video with this id in a PMVHaven page's data. The page is a Nuxt
/// app: its data is one JSON list in which every object's fields are the
/// places of their values in that same list.
fn pmvhaven_video(html: &str, id: &str) -> Option<PmvVideo> {
    static DATA: OnceLock<Regex> = OnceLock::new();
    let data = DATA.get_or_init(|| re(r#"(?is)<script[^>]*\bid=["']__NUXT_DATA__["'][^>]*>(.*?)</script>"#));
    let list: Vec<Value> = serde_json::from_str(&data.captures(html)?[1]).ok()?;
    let at = |v: &Value| v.as_u64().and_then(|i| list.get(i as usize));
    let text = |o: &serde_json::Map<String, Value>, k: &str| {
        o.get(k).and_then(at).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
    };
    list.iter().filter_map(Value::as_object).find_map(|o| {
        if text(o, "_id")?.to_ascii_lowercase() != id {
            return None;
        }
        let url = text(o, "videoUrl").filter(|u| u.starts_with("https://") || u.starts_with("http://"))?;
        let size = o.get("fileSize").and_then(at).and_then(num).filter(|n| *n > 0);
        Some(PmvVideo { title: text(o, "title"), url, size })
    })
}

/// A web server's own list of the files in a folder ("Index of /...").
async fn listing(web: &Web, page: &str) -> Result<(String, Vec<SiteFile>), String> {
    let html = web.page(page).await?;
    let base = Url::parse(page).map_err(|_| "not a valid link".to_string())?;
    static HREF: OnceLock<Regex> = OnceLock::new();
    let href = HREF.get_or_init(|| re(r#"(?i)<a\s[^>]*href="([^"?#]+)""#));
    let mut files = Vec::new();
    for c in href.captures_iter(&html) {
        let h = html_unescape(&c[1]);
        if h.ends_with('/') || h.starts_with("..") || h.starts_with("mailto:") {
            continue;
        }
        let Ok(url) = base.join(&h) else { continue };
        // Only files inside this folder.
        if !url.as_str().starts_with(base.as_str()) || url.as_str() == base.as_str() {
            continue;
        }
        let Some(name) = crate::probe::url_filename(url.as_str()).filter(|n| n.contains('.')) else { continue };
        // Links to other pages, not files.
        let lower = name.to_ascii_lowercase();
        let page = [".html", ".htm", ".php", ".asp", ".aspx", ".jsp", ".shtml"].iter().any(|e| lower.ends_with(e));
        if page && !lower.ends_with("-images.html") {
            continue;
        }
        if files.iter().any(|f: &SiteFile| f.url == url.as_str()) {
            continue;
        }
        files.push(one(&name, url.to_string(), None));
    }
    let title = base.path_segments().and_then(|s| s.filter(|p| !p.is_empty()).next_back().map(str::to_string)).unwrap_or_else(|| base.host_str().unwrap_or("files").to_string());
    Ok((title, files))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(link: &str) -> Option<Route> {
        route(link)
    }

    #[test]
    fn links_go_to_their_site() {
        assert_eq!(
            r("https://github.com/BurntSushi/ripgrep"),
            Some(Route::GitHub { owner: "BurntSushi".into(), repo: "ripgrep".into(), tag: None, all: false })
        );
        assert_eq!(
            r("https://github.com/BurntSushi/ripgrep/releases/tag/14.1.0"),
            Some(Route::GitHub { owner: "BurntSushi".into(), repo: "ripgrep".into(), tag: Some("14.1.0".into()), all: false })
        );
        // a release file is a plain link, downloaded as it is
        assert_eq!(r("https://github.com/a/b/releases/download/v1/x.zip"), None);
        assert_eq!(r("https://github.com/orgs/rust-lang"), None);
        assert_eq!(r("https://github.com/rust-lang"), None);
        assert_eq!(r("https://gitlab.com/inkscape/inkscape/-/releases"), Some(Route::GitLab { path: "inkscape/inkscape".into(), tag: None }));
        assert_eq!(
            r("https://huggingface.co/datasets/stanfordnlp/imdb"),
            Some(Route::HuggingFace { kind: "datasets", id: "stanfordnlp/imdb".into() })
        );
        assert_eq!(r("https://huggingface.co/openai-community/gpt2"), Some(Route::HuggingFace { kind: "models", id: "openai-community/gpt2".into() }));
        assert_eq!(r("https://huggingface.co/openai-community/gpt2/resolve/main/config.json"), None);
        assert_eq!(r("https://drive.google.com/file/d/1AbC/view?usp=sharing"), Some(Route::Drive { id: "1AbC".into() }));
        assert_eq!(r("https://drive.google.com/drive/folders/1XyZ?usp=sharing"), Some(Route::DriveFolder { id: "1XyZ".into() }));
        assert_eq!(r("https://drive.google.com/open?id=1AbC"), Some(Route::Drive { id: "1AbC".into() }));
        assert_eq!(r("https://mega.nz/file/abcD#k3y"), Some(Route::Mega { handle: "abcD".into(), key: "k3y".into() }));
        assert_eq!(r("https://mega.nz/#!abcD!k3y"), Some(Route::Mega { handle: "abcD".into(), key: "k3y".into() }));
        assert_eq!(r("https://mega.nz/folder/fOld#k3y/file/x"), Some(Route::MegaFolder { handle: "fOld".into(), key: "k3y".into() }));
        assert_eq!(r("https://mega.nz/file/abcD"), None, "no key, nothing to decrypt with");
        assert_eq!(r("https://zenodo.org/records/7654321"), Some(Route::Zenodo { id: "7654321".into() }));
        assert_eq!(r("https://archive.org/details/some-item"), Some(Route::Archive { id: "some-item".into() }));
        assert_eq!(r("https://www.kaggle.com/datasets/uciml/iris"), Some(Route::Kaggle { owner: "uciml".into(), slug: "iris".into() }));
        assert!(matches!(r("https://www.dropbox.com/scl/fi/abc/x.pdf?rlkey=1&dl=0"), Some(Route::Dropbox { folder: false, .. })));
        assert!(matches!(r("https://www.dropbox.com/scl/fo/abc/def?rlkey=1&dl=0"), Some(Route::Dropbox { folder: true, .. })));
        assert!(matches!(r("https://www.mediafire.com/file/abc/x.zip/file"), Some(Route::MediaFire { .. })));
        assert_eq!(r("https://unsplash.com/photos/a-cat-on-a-roof-AbC123"), Some(Route::Unsplash { id: "AbC123".into(), slug: "a-cat-on-a-roof-AbC123".into() }));
        assert_eq!(r("https://www.youtube.com/watch?v=x"), None);
        assert!(matches!(r("https://www.ncei.noaa.gov/data/global-hourly/"), Some(Route::Listing { .. })));
        assert_eq!(r("https://www.ncei.noaa.gov/data/global-hourly/a.csv"), None);
        assert_eq!(r("https://pmvhaven.com/video/_6b0a7a52ed9dcfdc03b4143d?from=home"), Some(Route::PmvHaven { id: "6b0a7a52ed9dcfdc03b4143d".into() }));
        assert_eq!(r("https://pmvhaven.com/video/Some-Title_6B0A7A52ED9DCFDC03B4143D"), Some(Route::PmvHaven { id: "6b0a7a52ed9dcfdc03b4143d".into() }));
        assert_eq!(r("https://pmvhaven.com/videos/6b0a7a52ed9dcfdc03b4143d"), Some(Route::PmvHaven { id: "6b0a7a52ed9dcfdc03b4143d".into() }));
        assert_eq!(r("https://pmvhaven.com/profile/someone"), None);
        assert_eq!(r("https://pmvhaven.com/video/not-a-video"), None);
        assert_eq!(r("https://sourceforge.net/projects/sevenzip/files/7-Zip/23.01/7z2301-x64.exe/download"),
            Some(Route::SourceForge { project: "sevenzip".into(), path: Some("7-Zip/23.01/7z2301-x64.exe".into()) }));
    }

    #[test]
    fn stores_and_paid_courses_say_why() {
        let steam = kind("https://store.steampowered.com/app/570/Dota_2/").unwrap();
        assert_eq!((steam.support, steam.why), (Support::Blocked, Some(Why::Store)));
        assert_eq!(kind("https://www.udemy.com/course/x/").unwrap().why, Some(Why::Paid));
        assert_eq!(kind("https://www.linkedin.com/learning/x").unwrap().why, Some(Why::Paid));
        assert_eq!(kind("https://www.linkedin.com/in/someone"), None);
        assert_eq!(kind("https://drive.proton.me/urls/ABC#key").unwrap().why, Some(Why::Encrypted));
        assert_eq!(kind("https://github.com/a/b").unwrap().support, Support::Files);
    }

    #[test]
    fn mega_names_and_data_decrypt() {
        use aes::cipher::{generic_array::GenericArray, BlockEncrypt};
        // A made-up file key; its attributes encrypted the way MEGA does.
        let full: Vec<u8> = (0u8..32).collect();
        let (key, nonce) = mega_file_key(&full).unwrap();
        let mut plain = b"MEGA{\"n\":\"hello.txt\"}".to_vec();
        plain.resize(32, 0);
        let cipher = aes(&key);
        let mut prev = [0u8; 16];
        let mut enc_attr = Vec::new();
        for block in plain.chunks(16) {
            let mut b = [0u8; 16];
            for i in 0..16 {
                b[i] = block[i] ^ prev[i];
            }
            let mut g = GenericArray::from(b);
            cipher.encrypt_block(&mut g);
            prev.copy_from_slice(&g);
            enc_attr.extend_from_slice(&g);
        }
        assert_eq!(mega_name(&b64(&enc_attr), &key).as_deref(), Some("hello.txt"));
        // counter mode is its own inverse: decrypting twice gives the data back
        let dir = std::env::temp_dir().join(format!("rush-mega-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("f.bin");
        let data: Vec<u8> = (0..3_000_017u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(&path, &data).unwrap();
        mega_decrypt(&path, key, nonce).unwrap();
        assert_ne!(std::fs::read(&path).unwrap(), data);
        mega_decrypt(&path, key, nonce).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), data);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn pmvhaven_page_names_the_original_file() {
        // Trimmed from a real page: another video's card comes first.
        let html = r#"<html><script type="application/json" id="__NUXT_DATA__" data-ssr="true">[["ShallowReactive",1],{"other":2,"video":7},
            {"_id":3,"videoUrl":4,"title":5,"fileSize":6},"aaaaaaaaaaaaaaaaaaaaaaaa","https://cdn.test/videos/other.mp4","Other",10,
            {"_id":8,"title":9,"videoUrl":10,"fileSize":11,"hlsMasterPlaylistUrl":12},
            "6b0a7a52ed9dcfdc03b4143d","Some title","https://cdn.test/videos/up_j3biqi6h.mp4",248436263,"https://cdn.test/videos/up_j3biqi6h.mp4/master.m3u8"]</script></html>"#;
        let v = pmvhaven_video(html, "6b0a7a52ed9dcfdc03b4143d").unwrap();
        assert_eq!(v.url, "https://cdn.test/videos/up_j3biqi6h.mp4");
        assert_eq!(v.title.as_deref(), Some("Some title"));
        assert_eq!(v.size, Some(248436263));
        assert!(pmvhaven_video(html, "bbbbbbbbbbbbbbbbbbbbbbbb").is_none());
        assert!(pmvhaven_video("<html></html>", "6b0a7a52ed9dcfdc03b4143d").is_none());
    }

    #[test]
    fn sizes_read_from_pages() {
        assert_eq!(parse_size("12.5 MB"), Some(13107200));
        assert_eq!(parse_size("900 kB"), Some(921600));
        assert_eq!(parse_size("big"), None);
    }
}
