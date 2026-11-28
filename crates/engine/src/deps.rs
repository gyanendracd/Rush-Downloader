//! The helper programs the app keeps itself: yt-dlp and gallery-dl.
//!
//! Each comes only from its project's own release page (yt-dlp from GitHub,
//! gallery-dl from Codeberg), and is checked against the SHA-256 sums the
//! project publishes with every release before it is used; a release
//! without sums is not installed. A new version is downloaded beside the one
//! in use, checked, run once (`--version`) and only then made current, so a
//! failed update leaves the working copy as it was. The version before is
//! kept, to go back to.
//!
//! Layout, inside the app's state folder:
//!
//! ```text
//! tools/tools.json                 what is installed, and when last checked
//! tools/yt-dlp/2026.11.23/yt-dlp.exe
//! tools/yt-dlp/2026.10.30/yt-dlp.exe   (the version before, kept)
//! ```
//!
//! A copy the user installed themselves (on PATH, or with winget) still
//! works: the app's own copy is used first when there is one, theirs
//! otherwise, and theirs is never replaced or updated by the app.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;

use crate::Inner;

/// How often the app looks for a new release of a tool it keeps up to date.
const CHECK_EVERY: Duration = Duration::from_secs(24 * 60 * 60);
/// Nothing the app installs is anywhere near this large.
const MAX_SIZE: u64 = 300 << 20;
/// A first run of a freshly installed tool unpacks it, and an antivirus
/// may look at it first: this long, at most.
const SMOKE_TIMEOUT: Duration = Duration::from_secs(120);

/// One tool and where its releases come from.
struct Source {
    name: &'static str,
    /// The latest release, as JSON (GitHub's and Codeberg's forms agree on
    /// the fields read here).
    latest: &'static str,
    /// The release asset holding the SHA-256 sums.
    sums: &'static str,
}

const SOURCES: [Source; 2] = [
    Source {
        name: "yt-dlp",
        latest: "https://api.github.com/repos/yt-dlp/yt-dlp/releases/latest",
        sums: "SHA2-256SUMS",
    },
    Source {
        name: "gallery-dl",
        latest: "https://codeberg.org/api/v1/repos/mikf/gallery-dl/releases/latest",
        sums: "SHA256SUMS",
    },
];

/// The names of the tools the app can keep.
pub const MANAGED_TOOLS: [&str; 2] = ["yt-dlp", "gallery-dl"];

/// The release file built for this computer, or `None` where the project
/// publishes none (gallery-dl for macOS: a copy installed by the user is
/// used there).
fn asset_for(tool: &str, os: &str, arch: &str) -> Option<&'static str> {
    Some(match (tool, os, arch) {
        ("yt-dlp", "windows", "x86") => "yt-dlp_x86.exe",
        ("yt-dlp", "windows", "aarch64") => "yt-dlp_arm64.exe",
        ("yt-dlp", "windows", _) => "yt-dlp.exe",
        ("yt-dlp", "macos", _) => "yt-dlp_macos",
        ("yt-dlp", "linux", "aarch64") => "yt-dlp_linux_aarch64",
        ("yt-dlp", "linux", "x86_64") => "yt-dlp_linux",
        ("gallery-dl", "windows", "x86") => "gallery-dl_x86.exe",
        // Windows on ARM runs the x64 build.
        ("gallery-dl", "windows", _) => "gallery-dl.exe",
        ("gallery-dl", "linux", "x86_64") => "gallery-dl.bin",
        _ => return None,
    })
}

fn this_asset(tool: &str) -> Option<&'static str> {
    asset_for(tool, std::env::consts::OS, std::env::consts::ARCH)
}

/// The program's file name on this computer.
fn exe_name(tool: &str) -> String {
    if cfg!(windows) {
        format!("{tool}.exe")
    } else {
        tool.to_string()
    }
}

/// "v1.32.21" and "2026.11.23" as numbers to compare: newer is larger.
pub(crate) fn version_key(version: &str) -> Vec<u64> {
    version
        .trim()
        .trim_start_matches(['v', 'V'])
        .split(|c: char| !c.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .map(|part| part.parse().unwrap_or(0))
        .collect()
}

fn newer(candidate: &str, than: &str) -> bool {
    version_key(candidate) > version_key(than)
}

/// The sum of `asset` in a sums file ("<hex>  <name>" lines).
fn sum_of(sums: &str, asset: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (hash, name) = line.trim().split_once(char::is_whitespace)?;
        let name = name.trim().trim_start_matches('*');
        (name == asset && hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit())).then(|| hash.to_ascii_lowercase())
    })
}

/// What the app remembers about one tool (tools.json).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Kept {
    /// The version in use, in `tools/<name>/<version>/`.
    #[serde(default)]
    current: Option<String>,
    /// The version before it, kept to go back to.
    #[serde(default)]
    previous: Option<String>,
    #[serde(default)]
    installed_at: Option<u64>,
    #[serde(default)]
    last_checked: Option<u64>,
    /// The newest release seen at the last check.
    #[serde(default)]
    latest: Option<String>,
    #[serde(default)]
    notes_url: Option<String>,
    /// Why the last check or update failed.
    #[serde(default)]
    error: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Book {
    #[serde(default)]
    schema: u32,
    #[serde(default)]
    tools: HashMap<String, Kept>,
}

/// Where a tool in use comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolSource {
    /// Installed and kept up to date by the app.
    Managed,
    /// Installed by the user (PATH, winget); left alone.
    System,
    Missing,
}

/// What an install or check is doing now.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolPhase {
    Checking,
    Downloading,
    Verifying,
    Installing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct ToolProgress {
    pub phase: ToolPhase,
    pub done: u64,
    pub total: Option<u64>,
}

/// One tool, for display.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ToolInfo {
    pub name: String,
    pub source: ToolSource,
    /// The program used now.
    pub path: Option<PathBuf>,
    /// Its version, where known.
    pub version: Option<String>,
    /// The newest release seen at the last check.
    pub latest: Option<String>,
    /// A newer release than the app's own copy (or than a copy the user
    /// installed, which the app's own would then replace in use).
    pub update_available: bool,
    /// The project publishes a build for this computer.
    pub can_install: bool,
    /// When the app's own copy was installed, seconds since 1970.
    pub installed_at: Option<u64>,
    pub last_checked: Option<u64>,
    /// The version kept to go back to.
    pub previous: Option<String>,
    pub notes_url: Option<String>,
    /// While a check or an install runs.
    pub busy: Option<ToolProgress>,
    pub error: Option<String>,
}

/// The app's own copies of the tools, for the lookup in media.rs, which has
/// no engine at hand: by name, the program to run.
static IN_USE: RwLock<Option<HashMap<String, PathBuf>>> = RwLock::new(None);

/// The app's own copy of a tool, if it has one and it is still there.
pub(crate) fn managed_path(name: &str) -> Option<PathBuf> {
    let guard = IN_USE.read().ok()?;
    guard.as_ref()?.get(name).filter(|p| p.is_file()).cloned()
}

/// The dependency manager's state, held by the engine.
pub(crate) struct Deps {
    root: PathBuf,
    book: Mutex<Book>,
    busy: Mutex<HashSet<String>>,
    progress: Mutex<HashMap<String, ToolProgress>>,
    /// Versions of copies the user installed, by path, found by running
    /// them (`--version`) now and then.
    system_versions: Mutex<HashMap<PathBuf, (std::time::SystemTime, String)>>,
    /// When progress was last announced, so a download does not flood the
    /// interface.
    announced: Mutex<Option<Instant>>,
    /// Goes up whenever anything shown about the tools changes, so the
    /// interface asks again only then (finding a tool looks at the disk).
    revision: std::sync::atomic::AtomicU64,
    /// Tests: release pages on a local server, and no trial run.
    #[cfg(test)]
    test_latest: Mutex<HashMap<&'static str, String>>,
    #[cfg(test)]
    skip_trial_run: std::sync::atomic::AtomicBool,
}

impl Deps {
    pub(crate) fn new(state_dir: &Path) -> Deps {
        let root = state_dir.join("tools");
        let book: Book = std::fs::read(root.join("tools.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let deps = Deps {
            root,
            book: Mutex::new(book),
            busy: Mutex::new(HashSet::new()),
            progress: Mutex::new(HashMap::new()),
            system_versions: Mutex::new(HashMap::new()),
            announced: Mutex::new(None),
            revision: std::sync::atomic::AtomicU64::new(1),
            #[cfg(test)]
            test_latest: Mutex::new(HashMap::new()),
            #[cfg(test)]
            skip_trial_run: std::sync::atomic::AtomicBool::new(false),
        };
        deps.publish();
        deps
    }

    fn dir(&self, name: &str, version: &str) -> PathBuf {
        self.root.join(name).join(version)
    }

    fn program(&self, name: &str, version: &str) -> PathBuf {
        self.dir(name, version).join(exe_name(name))
    }

    /// Makes the copies in use known to the lookup.
    fn publish(&self) {
        let map: HashMap<String, PathBuf> = {
            let book = self.book.lock().unwrap();
            book.tools
                .iter()
                .filter_map(|(name, kept)| Some((name.clone(), self.program(name, kept.current.as_deref()?))))
                .collect()
        };
        if let Ok(mut slot) = IN_USE.write() {
            *slot = Some(map);
        }
    }

    fn save(&self) {
        let bytes = {
            let mut book = self.book.lock().unwrap();
            book.schema = 1;
            serde_json::to_vec_pretty(&*book)
        };
        let Ok(bytes) = bytes else { return };
        let _ = std::fs::create_dir_all(&self.root);
        let file = self.root.join("tools.json");
        let tmp = file.with_extension("json.tmp");
        if std::fs::write(&tmp, bytes).is_ok() {
            let _ = std::fs::rename(&tmp, &file);
        }
    }

    fn edit(&self, name: &str, f: impl FnOnce(&mut Kept)) {
        f(self.book.lock().unwrap().tools.entry(name.to_string()).or_default());
        self.save();
        self.publish();
        self.bump();
    }

    fn bump(&self) {
        self.revision.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    pub(crate) fn revision(&self) -> u64 {
        self.revision.load(std::sync::atomic::Ordering::Relaxed)
    }

    fn kept(&self, name: &str) -> Kept {
        self.book.lock().unwrap().tools.get(name).cloned().unwrap_or_default()
    }

    pub(crate) fn info(&self, name: &str) -> ToolInfo {
        let kept = self.kept(name);
        let own = kept.current.as_deref().map(|v| (v.to_string(), self.program(name, v))).filter(|(_, p)| p.is_file());
        let mut error = kept.error.clone();
        if kept.current.is_some() && own.is_none() && error.is_none() {
            error = Some("The app's copy is gone from its folder; an antivirus may have removed it. Install it again".into());
        }
        let (source, path, version) = match own {
            Some((v, p)) => (ToolSource::Managed, Some(p), Some(v)),
            None => match crate::media::find_tool(name) {
                Some(p) => {
                    let version = self.system_versions.lock().unwrap().get(&p).map(|(_, v)| v.clone());
                    (ToolSource::System, Some(p), version)
                }
                None => (ToolSource::Missing, None, None),
            },
        };
        let update_available = match (&kept.latest, &version, source) {
            (Some(latest), _, ToolSource::Missing) => !latest.is_empty() && this_asset(name).is_some(),
            (Some(latest), Some(v), _) => newer(latest, v) && this_asset(name).is_some(),
            _ => false,
        };
        ToolInfo {
            name: name.to_string(),
            source,
            path,
            version,
            latest: kept.latest,
            update_available,
            can_install: this_asset(name).is_some(),
            installed_at: kept.installed_at.filter(|_| source == ToolSource::Managed),
            last_checked: kept.last_checked,
            previous: kept.previous.filter(|v| self.program(name, v).is_file()),
            notes_url: kept.notes_url,
            busy: self.progress.lock().unwrap().get(name).copied(),
            error,
        }
    }

    /// Removes version folders other than the current and the previous one.
    /// One still in use by a running program cannot be removed; it goes
    /// next time.
    fn prune(&self, name: &str) {
        let kept = self.kept(name);
        let keep: Vec<&str> = [kept.current.as_deref(), kept.previous.as_deref()].into_iter().flatten().collect();
        for entry in std::fs::read_dir(self.root.join(name)).into_iter().flatten().flatten() {
            let file_name = entry.file_name().to_string_lossy().into_owned();
            if !keep.contains(&file_name.as_str()) {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    }
}

/// The latest release, as both forges describe it.
#[derive(Deserialize)]
struct Release {
    tag_name: String,
    #[serde(default)]
    html_url: Option<String>,
    #[serde(default)]
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    #[serde(default)]
    size: u64,
    browser_download_url: String,
    /// "sha256:<hex>" on GitHub.
    #[serde(default)]
    digest: Option<String>,
}

/// Clears a tool's progress when an install or check ends, however it ends.
struct Busy<'a> {
    inner: &'a Inner,
    name: String,
}

impl Drop for Busy<'_> {
    fn drop(&mut self) {
        self.inner.deps.busy.lock().unwrap().remove(&self.name);
        self.inner.deps.progress.lock().unwrap().remove(&self.name);
        self.inner.deps.bump();
        self.inner.notify();
    }
}

fn source(name: &str) -> Result<&'static Source, String> {
    SOURCES.iter().find(|s| s.name == name).ok_or_else(|| format!("RushDownload does not install {name}"))
}

fn now() -> u64 {
    crate::now()
}

impl Inner {
    fn tool_progress(&self, name: &str, phase: ToolPhase, done: u64, total: Option<u64>) {
        self.deps.progress.lock().unwrap().insert(name.to_string(), ToolProgress { phase, done, total });
        self.deps.bump();
        let mut last = self.deps.announced.lock().unwrap();
        if last.is_none_or(|at| at.elapsed() > Duration::from_millis(200)) || done == 0 {
            *last = Some(Instant::now());
            drop(last);
            self.notify();
        }
    }

    fn take_busy(&self, name: &str) -> Result<Busy<'_>, String> {
        if !self.deps.busy.lock().unwrap().insert(name.to_string()) {
            return Err(format!("{name} is already being checked or updated"));
        }
        Ok(Busy { inner: self, name: name.to_string() })
    }

    async fn latest_release(&self, src: &Source) -> Result<Release, String> {
        let client = self.client()?;
        #[cfg(test)]
        let url = self.deps.test_latest.lock().unwrap().get(src.name).cloned().unwrap_or_else(|| src.latest.to_string());
        #[cfg(not(test))]
        let url = src.latest;
        let resp = client
            .get(url)
            .header(reqwest::header::ACCEPT, "application/json")
            .timeout(Duration::from_secs(30))
            .send()
            .await
            .map_err(|e| format!("could not reach the release page: {e}"))?;
        let status = resp.status();
        if status == reqwest::StatusCode::FORBIDDEN || status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err("the release page is asking for a pause (too many checks); try again in an hour".into());
        }
        if !status.is_success() {
            return Err(format!("the release page answered {status}"));
        }
        let body = resp.bytes().await.map_err(|e| format!("the release page could not be read: {e}"))?;
        serde_json::from_slice::<Release>(&body).map_err(|e| format!("the release page could not be read: {e}"))
    }

    /// Looks for a new release of one tool and remembers what it found.
    pub(crate) async fn check_tool(&self, name: &str) -> Result<ToolInfo, String> {
        let src = source(name)?;
        let _busy = self.take_busy(name)?;
        self.tool_progress(name, ToolPhase::Checking, 0, None);
        self.refresh_system_version(name).await;
        match self.latest_release(src).await {
            Ok(release) => {
                let latest = release.tag_name.trim_start_matches(['v', 'V']).to_string();
                tracing::info!(tool = name, %latest, "checked for a new release");
                self.deps.edit(name, |k| {
                    k.latest = Some(latest);
                    k.notes_url = release.html_url;
                    k.last_checked = Some(now());
                    k.error = None;
                });
            }
            Err(e) => {
                tracing::warn!(tool = name, "release check failed: {e}");
                self.deps.edit(name, |k| {
                    k.last_checked = Some(now());
                    k.error = Some(format!("Could not check for updates: {e}"));
                });
                return Err(e);
            }
        }
        Ok(self.deps.info(name))
    }

    /// Learns the version of a copy the user installed, by running it, once
    /// per change of the file.
    async fn refresh_system_version(&self, name: &str) {
        let Some(path) = crate::media::find_tool_on_system(name) else { return };
        let Ok(modified) = std::fs::metadata(&path).and_then(|m| m.modified()) else { return };
        if self.deps.system_versions.lock().unwrap().get(&path).is_some_and(|(at, _)| *at == modified) {
            return;
        }
        if let Ok(version) = run_version(&path).await {
            self.deps.system_versions.lock().unwrap().insert(path, (modified, version));
            self.deps.bump();
        }
    }

    /// Installs the latest release of a tool, or updates the app's copy to
    /// it. The copy in use stays in use until the new one has been checked
    /// against the project's sums and has run.
    pub(crate) async fn install_tool(&self, name: &str) -> Result<ToolInfo, String> {
        let src = source(name)?;
        let asset_name = this_asset(name).ok_or_else(|| {
            format!("{name} publishes no build for this computer; install it yourself and RushDownload will use it")
        })?;
        let _busy = self.take_busy(name)?;
        let result = self.install(src, asset_name).await;
        if let Err(e) = &result {
            tracing::warn!(tool = name, "install failed: {e}");
            self.deps.edit(name, |k| k.error = Some(format!("The update failed, nothing was changed: {e}")));
        }
        result.map(|_| self.deps.info(name))
    }

    async fn install(&self, src: &Source, asset_name: &str) -> Result<(), String> {
        let name = src.name;
        self.tool_progress(name, ToolPhase::Checking, 0, None);
        let release = self.latest_release(src).await?;
        let version = release.tag_name.trim_start_matches(['v', 'V']).to_string();
        // It names a folder: nothing that could lead out of it.
        if version_key(&version).is_empty() || version.contains(['/', '\\', ':']) || version.contains("..") {
            return Err("the release has no usable version name".into());
        }
        let asset = release
            .assets
            .iter()
            .find(|a| a.name == asset_name)
            .ok_or_else(|| format!("the release {version} has no {asset_name}"))?;
        let sums_asset = release
            .assets
            .iter()
            .find(|a| a.name == src.sums)
            .ok_or_else(|| format!("the release {version} publishes no checksums, so it is not installed"))?;
        if asset.size > MAX_SIZE {
            return Err(format!("{asset_name} is larger than expected ({} bytes)", asset.size));
        }
        let client = self.client()?;
        let sums = client
            .get(&sums_asset.browser_download_url)
            .timeout(Duration::from_secs(30))
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(|e| format!("could not download the checksums: {e}"))?
            .text()
            .await
            .map_err(|e| format!("could not download the checksums: {e}"))?;
        let expected = sum_of(&sums, asset_name).ok_or_else(|| format!("the checksums do not list {asset_name}"))?;
        // GitHub states each file's sum too: both must agree.
        if let Some(digest) = asset.digest.as_deref().and_then(|d| d.strip_prefix("sha256:")) {
            if !digest.eq_ignore_ascii_case(&expected) {
                return Err("the release's checksums disagree with each other; it is not installed".into());
            }
        }

        // Downloaded into a folder of its own, next to the one in use.
        let staging = self.deps.root.join(name).join(format!(".new-{version}"));
        let _ = std::fs::remove_dir_all(&staging);
        std::fs::create_dir_all(&staging).map_err(|e| format!("could not create {}: {e}", staging.display()))?;
        let cleanup = scopeguard(staging.clone());
        let part = staging.join(format!("{}.part", exe_name(name)));
        let mut resp = client
            .get(&asset.browser_download_url)
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(|e| format!("could not download {asset_name}: {e}"))?;
        let total = resp.content_length().or(Some(asset.size).filter(|s| *s > 0));
        let mut file = tokio::fs::File::create(&part).await.map_err(|e| format!("could not write {}: {e}", part.display()))?;
        let mut hash = ring::digest::Context::new(&ring::digest::SHA256);
        let mut done = 0u64;
        self.tool_progress(name, ToolPhase::Downloading, 0, total);
        loop {
            let chunk = tokio::time::timeout(Duration::from_secs(60), resp.chunk())
                .await
                .map_err(|_| "the download stalled".to_string())?
                .map_err(|e| format!("the download broke off: {e}"))?;
            let Some(chunk) = chunk else { break };
            done += chunk.len() as u64;
            if done > MAX_SIZE {
                return Err(format!("{asset_name} is larger than expected"));
            }
            hash.update(&chunk);
            file.write_all(&chunk).await.map_err(|e| format!("could not write {}: {e}", part.display()))?;
            self.tool_progress(name, ToolPhase::Downloading, done, total);
        }
        file.flush().await.map_err(|e| e.to_string())?;
        drop(file);

        self.tool_progress(name, ToolPhase::Verifying, done, total);
        let got: String = hash.finish().as_ref().iter().map(|b| format!("{b:02x}")).collect();
        if got != expected {
            return Err(format!("{asset_name} does not match its published checksum; it was deleted"));
        }
        let program = staging.join(exe_name(name));
        std::fs::rename(&part, &program).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
        }
        #[cfg(test)]
        let skip = self.deps.skip_trial_run.load(std::sync::atomic::Ordering::Relaxed);
        #[cfg(not(test))]
        let skip = false;
        let said = if skip {
            String::new()
        } else {
            run_version(&program).await.map_err(|e| format!("the new {name} would not run: {e}"))?
        };
        tracing::info!(tool = name, %version, said, "new version checked and runs");

        self.tool_progress(name, ToolPhase::Installing, done, total);
        let target = self.deps.dir(name, &version);
        let old = self.deps.kept(name).current;
        if old.as_deref() == Some(version.as_str()) {
            // The same version again (a repair): the old folder goes first.
            let _ = std::fs::remove_dir_all(&target);
        }
        if target.exists() {
            let _ = std::fs::remove_dir_all(&target);
        }
        std::fs::rename(&staging, &target).map_err(|e| format!("could not put the new version in place: {e}"))?;
        cleanup.disarm();
        self.deps.edit(name, |k| {
            if old.as_deref() != Some(version.as_str()) {
                k.previous = old.clone();
            }
            k.current = Some(version.clone());
            k.installed_at = Some(now());
            k.latest = Some(version.clone());
            k.notes_url = release.html_url.clone();
            k.last_checked = Some(now());
            k.error = None;
        });
        self.deps.prune(name);
        tracing::info!(tool = name, %version, previous = ?old, "installed");
        Ok(())
    }

    /// Goes back to the version used before the last update.
    pub(crate) fn rollback_tool(&self, name: &str) -> Result<ToolInfo, String> {
        source(name)?;
        let _busy = self.take_busy(name)?;
        let kept = self.deps.kept(name);
        let previous = kept
            .previous
            .filter(|v| self.deps.program(name, v).is_file())
            .ok_or_else(|| format!("There is no earlier version of {name} to go back to"))?;
        self.deps.edit(name, |k| {
            k.previous = k.current.take();
            k.current = Some(previous.clone());
            k.installed_at = Some(now());
            k.error = None;
        });
        tracing::info!(tool = name, version = %previous, "went back to the earlier version");
        drop(_busy);
        Ok(self.deps.info(name))
    }

    /// What the app does on its own, at start and every hour: installs a
    /// tool that is missing altogether, and keeps its own copies up to date
    /// (checking each at most once a day). Copies the user installed are
    /// left alone. Nothing at all happens with the setting off.
    pub(crate) async fn tend_tools(self: &Arc<Self>) {
        for name in MANAGED_TOOLS {
            self.refresh_system_version(name).await;
        }
        if !self.state.lock().unwrap().settings.auto_update_tools {
            self.notify();
            return;
        }
        for name in MANAGED_TOOLS {
            if this_asset(name).is_none() {
                continue;
            }
            let info = self.deps.info(name);
            let due = info.last_checked.is_none_or(|at| now().saturating_sub(at) >= CHECK_EVERY.as_secs());
            let result = match info.source {
                ToolSource::Missing => self.install_tool(name).await.map(|_| ()),
                ToolSource::Managed if due => match self.check_tool(name).await {
                    Ok(checked) if checked.update_available => self.install_tool(name).await.map(|_| ()),
                    Ok(_) => Ok(()),
                    Err(e) => Err(e),
                },
                _ => Ok(()),
            };
            if let Err(e) = result {
                tracing::warn!(tool = name, "automatic tool upkeep: {e}");
            }
        }
        self.notify();
    }
}

/// Runs `program --version` and returns the first line it prints.
async fn run_version(program: &Path) -> Result<String, String> {
    let mut cmd = crate::media::command(program);
    cmd.arg("--version").stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
    let child = cmd.spawn().map_err(|e| e.to_string())?;
    let out = tokio::time::timeout(SMOKE_TIMEOUT, child.wait_with_output())
        .await
        .map_err(|_| "it did not answer in time".to_string())?
        .map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or_default().to_string();
    if !out.status.success() || line.is_empty() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(err.lines().last().unwrap_or("it gave no version").trim().to_string());
    }
    Ok(line)
}

/// Deletes a folder when dropped, unless disarmed: what an install that
/// stopped half way leaves behind.
struct Guard(Option<PathBuf>);

fn scopeguard(dir: PathBuf) -> Guard {
    Guard(Some(dir))
}

impl Guard {
    fn disarm(mut self) {
        self.0 = None;
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        if let Some(dir) = self.0.take() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_computer_gets_its_own_build() {
        assert_eq!(asset_for("yt-dlp", "windows", "x86_64"), Some("yt-dlp.exe"));
        assert_eq!(asset_for("yt-dlp", "windows", "aarch64"), Some("yt-dlp_arm64.exe"));
        assert_eq!(asset_for("yt-dlp", "windows", "x86"), Some("yt-dlp_x86.exe"));
        assert_eq!(asset_for("yt-dlp", "macos", "aarch64"), Some("yt-dlp_macos"));
        assert_eq!(asset_for("yt-dlp", "linux", "x86_64"), Some("yt-dlp_linux"));
        assert_eq!(asset_for("yt-dlp", "linux", "aarch64"), Some("yt-dlp_linux_aarch64"));
        assert_eq!(asset_for("yt-dlp", "linux", "riscv64"), None);
        assert_eq!(asset_for("gallery-dl", "windows", "x86_64"), Some("gallery-dl.exe"));
        assert_eq!(asset_for("gallery-dl", "windows", "aarch64"), Some("gallery-dl.exe"));
        assert_eq!(asset_for("gallery-dl", "linux", "x86_64"), Some("gallery-dl.bin"));
        assert_eq!(asset_for("gallery-dl", "macos", "aarch64"), None, "no macOS build is published");
        assert_eq!(asset_for("ffmpeg", "windows", "x86_64"), None);
    }

    #[test]
    fn versions_compare_as_numbers() {
        assert!(newer("2026.11.23", "2026.10.30"));
        assert!(newer("2026.11.23", "2026.9.30"), "not as text");
        assert!(newer("v1.32.21", "1.32.9"));
        assert!(newer("2026.11.23.123456", "2026.11.23"));
        assert!(!newer("1.32.21", "v1.32.21"));
        assert!(!newer("2026.01.01", "2026.11.23"));
        assert_eq!(version_key("v1.32.21"), vec![1, 32, 21]);
    }

    #[test]
    fn reads_the_published_sums() {
        let sums = "622ED48023CAC1DAA52A3BF3A6AC38F03514DE214B5F0FA069389FD0F6D17E5C  yt-dlp.exe\n\
                    9bb324203d02970b70dab6c9f6a48bd5ff185db7859d562cdacd1dc655ccb72a *yt-dlp_arm64.exe\n\
                    nonsense line\n\
                    abc  yt-dlp_x86.exe\n";
        assert_eq!(sum_of(sums, "yt-dlp.exe").as_deref(), Some("622ed48023cac1daa52a3bf3a6ac38f03514de214b5f0fa069389fd0f6d17e5c"));
        assert!(sum_of(sums, "yt-dlp_arm64.exe").is_some(), "a binary-mode star is allowed");
        assert_eq!(sum_of(sums, "yt-dlp_x86.exe"), None, "a sum of the wrong length is no sum");
        assert_eq!(sum_of(sums, "yt-dlp"), None, "names match whole");
    }

    #[test]
    fn a_half_done_install_is_cleaned_up() {
        let dir = std::env::temp_dir().join(format!("rd-guard-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        drop(scopeguard(dir.clone()));
        assert!(!dir.exists());
        std::fs::create_dir_all(&dir).unwrap();
        scopeguard(dir.clone()).disarm();
        assert!(dir.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A release server for one tool: what `/release` describes can be
    /// changed between requests.
    struct FakeForge {
        base: String,
        release: Arc<Mutex<FakeRelease>>,
        downloads: Arc<std::sync::atomic::AtomicUsize>,
    }

    #[derive(Clone)]
    struct FakeRelease {
        tag: String,
        body: Vec<u8>,
        /// The sum the sums file states; `None` for a release without one.
        stated: Option<String>,
    }

    fn sha256(data: &[u8]) -> String {
        ring::digest::digest(&ring::digest::SHA256, data).as_ref().iter().map(|b| format!("{b:02x}")).collect()
    }

    async fn forge(asset: &'static str) -> FakeForge {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let release = Arc::new(Mutex::new(FakeRelease { tag: "v1.0.0".into(), body: b"one".to_vec(), stated: None }));
        let downloads = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (rel, count, url) = (release.clone(), downloads.clone(), base.clone());
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { return };
                let (rel, count, url) = (rel.clone(), count.clone(), url.clone());
                tokio::spawn(async move {
                    let mut head = Vec::new();
                    let mut chunk = [0u8; 1024];
                    while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                        match sock.read(&mut chunk).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => head.extend_from_slice(&chunk[..n]),
                        }
                    }
                    let path = String::from_utf8_lossy(&head).split_whitespace().nth(1).unwrap_or("/").to_string();
                    let r = rel.lock().unwrap().clone();
                    let body: Vec<u8> = match path.as_str() {
                        "/release" => {
                            let mut assets = vec![serde_json::json!({
                                "name": asset, "size": r.body.len(), "browser_download_url": format!("{url}/bin"),
                            })];
                            if r.stated.is_some() {
                                assets.push(serde_json::json!({ "name": "SHA2-256SUMS", "size": 100, "browser_download_url": format!("{url}/sums") }));
                            }
                            serde_json::json!({ "tag_name": r.tag, "html_url": format!("{url}/notes"), "assets": assets }).to_string().into_bytes()
                        }
                        "/sums" => format!("{}  other.exe\n{}  {asset}\n", "0".repeat(64), r.stated.unwrap_or_default()).into_bytes(),
                        "/bin" => {
                            count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            r.body
                        }
                        _ => Vec::new(),
                    };
                    let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                    let _ = sock.write_all(head.as_bytes()).await;
                    let _ = sock.write_all(&body).await;
                });
            }
        });
        FakeForge { base, release, downloads }
    }

    fn set_release(forge: &FakeForge, tag: &str, body: &[u8], stated: Option<String>) {
        *forge.release.lock().unwrap() = FakeRelease { tag: tag.into(), body: body.to_vec(), stated };
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn installs_checks_updates_refuses_bad_files_and_goes_back() {
        let Some(asset) = this_asset("yt-dlp") else { return };
        let forge = forge(asset).await;
        let dir = std::env::temp_dir().join(format!("rd-deps-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut settings = crate::Settings::new(dir.join("out"));
        settings.auto_update_tools = false;
        settings.secure_dns = false;
        let engine = crate::Engine::with_secrets(dir.join("state"), settings, Arc::new(crate::MemoryStore::default())).unwrap();
        let inner = &engine.inner;
        inner.deps.test_latest.lock().unwrap().insert("yt-dlp", format!("{}/release", forge.base));
        inner.deps.skip_trial_run.store(true, std::sync::atomic::Ordering::Relaxed);
        let file = |v: &str| dir.join("state").join("tools").join("yt-dlp").join(v).join(exe_name("yt-dlp"));

        // a release without checksums is not installed
        set_release(&forge, "v1.0.0", b"one", None);
        let err = engine.install_tool("yt-dlp").await.unwrap_err();
        assert!(err.contains("no checksums"), "{err}");
        assert_eq!(forge.downloads.load(std::sync::atomic::Ordering::Relaxed), 0, "nothing was downloaded");

        // a good one is
        set_release(&forge, "v1.0.0", b"one", Some(sha256(b"one")));
        let info = engine.install_tool("yt-dlp").await.unwrap();
        assert_eq!((info.source, info.version.as_deref()), (ToolSource::Managed, Some("1.0.0")));
        assert_eq!(std::fs::read(file("1.0.0")).unwrap(), b"one");
        assert_eq!(crate::media::find_tool("yt-dlp"), Some(file("1.0.0")), "the app's own copy is used first");
        assert!(!info.update_available);
        assert_eq!(info.notes_url, Some(format!("{}/notes", forge.base)));

        // a check sees a newer release
        set_release(&forge, "v1.1.0", b"two", Some(sha256(b"two")));
        let checked = inner.check_tool("yt-dlp").await.unwrap();
        assert_eq!(checked.latest.as_deref(), Some("1.1.0"));
        assert!(checked.update_available);
        assert!(checked.last_checked.is_some());

        // a file that does not match its sum is thrown away; the copy in use stays
        set_release(&forge, "v1.1.0", b"tampered", Some(sha256(b"two")));
        let err = engine.install_tool("yt-dlp").await.unwrap_err();
        assert!(err.contains("does not match"), "{err}");
        let info = engine.tools_info().into_iter().find(|t| t.name == "yt-dlp").unwrap();
        assert_eq!(info.version.as_deref(), Some("1.0.0"));
        assert!(info.error.as_deref().unwrap().contains("nothing was changed"));
        assert!(!file("1.1.0").exists());
        assert!(!dir.join("state").join("tools").join("yt-dlp").join(".new-1.1.0").exists(), "the half-done install is gone");

        // the real update goes in, keeping the version before
        set_release(&forge, "v1.1.0", b"two", Some(sha256(b"two")));
        let info = engine.install_tool("yt-dlp").await.unwrap();
        assert_eq!(info.version.as_deref(), Some("1.1.0"));
        assert_eq!(info.previous.as_deref(), Some("1.0.0"));
        assert!(info.error.is_none(), "the failure before is forgotten");

        // a third version: only the current and the one before are kept
        set_release(&forge, "v1.2.0", b"three", Some(sha256(b"three")));
        engine.install_tool("yt-dlp").await.unwrap();
        assert!(!file("1.0.0").exists());
        assert!(file("1.1.0").exists() && file("1.2.0").exists());

        // going back swaps the two
        let back = engine.rollback_tool("yt-dlp").unwrap();
        assert_eq!((back.version.as_deref(), back.previous.as_deref()), (Some("1.1.0"), Some("1.2.0")));
        assert_eq!(crate::media::find_tool("yt-dlp"), Some(file("1.1.0")));

        // one operation at a time per tool
        let held = inner.take_busy("yt-dlp").unwrap();
        assert!(engine.install_tool("yt-dlp").await.unwrap_err().contains("already"));
        assert!(engine.rollback_tool("yt-dlp").unwrap_err().contains("already"));
        drop(held);

        // what was installed is remembered after a restart
        drop(engine);
        let mut settings = crate::Settings::new(dir.join("out"));
        settings.auto_update_tools = false;
        let again = crate::Engine::with_secrets(dir.join("state"), settings, Arc::new(crate::MemoryStore::default())).unwrap();
        let info = again.tools_info().into_iter().find(|t| t.name == "yt-dlp").unwrap();
        assert_eq!((info.source, info.version.as_deref()), (ToolSource::Managed, Some("1.1.0")));

        // an antivirus took the file away: the app says so, and falls back
        std::fs::remove_file(file("1.1.0")).unwrap();
        let info = again.tools_info().into_iter().find(|t| t.name == "yt-dlp").unwrap();
        assert_ne!(info.source, ToolSource::Managed);
        assert!(info.error.as_deref().unwrap().contains("antivirus"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
