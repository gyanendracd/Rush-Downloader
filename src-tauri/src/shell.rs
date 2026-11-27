//! The app's place on the desktop: starting when the user signs in, living
//! in the tray while its window is closed, and the window's graphics.
//!
//! These are kept apart from the engine's settings, in `app-prefs.json`
//! beside them, because the graphics choice is needed before the window
//! exists, and none of them is about downloading.

use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Runtime, WebviewWindowBuilder, Window, WindowEvent};
use tauri_plugin_autostart::ManagerExt;

/// What the system passes when it starts the app at sign-in.
pub const AUTOSTART_ARG: &str = "--autostart";

/// The switches Tauri gives the window's WebView2 by default; giving any
/// switches replaces them, so they are given again with the app's own.
const DEFAULT_BROWSER_ARGS: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppPrefs {
    /// Start when the user signs in to the computer.
    pub start_with_system: bool,
    /// Closing the window leaves the app running in the tray.
    pub close_to_tray: bool,
    /// A system notification when a download finishes or fails (sent by the
    /// interface, in the user's language).
    pub notifications: bool,
    /// Let the window draw with the graphics card. Read at start only.
    pub hardware_acceleration: bool,
}

impl Default for AppPrefs {
    fn default() -> Self {
        Self {
            start_with_system: false,
            close_to_tray: false,
            notifications: true,
            hardware_acceleration: true,
        }
    }
}

/// The preferences in memory and where they are kept.
pub struct Prefs {
    file: PathBuf,
    now: Mutex<AppPrefs>,
}

impl Prefs {
    pub fn load(state_dir: &std::path::Path) -> Self {
        let file = state_dir.join("app-prefs.json");
        let now = std::fs::read(&file)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        Self { file, now: Mutex::new(now) }
    }

    pub fn get(&self) -> AppPrefs {
        *self.now.lock().unwrap()
    }

    fn set(&self, prefs: AppPrefs) {
        *self.now.lock().unwrap() = prefs;
        if let Ok(bytes) = serde_json::to_vec_pretty(&prefs) {
            let _ = std::fs::write(&self.file, bytes);
        }
    }
}

/// The tray menu's two lines, kept so the interface can put them in the
/// user's language.
pub struct TrayText<R: Runtime> {
    show: MenuItem<R>,
    quit: MenuItem<R>,
}

/// Opens the main window, drawn without the graphics card when that is
/// turned off. Started by the system at sign-in with the tray on, it opens
/// hidden: the app waits in the tray.
pub fn open_main_window(app: &AppHandle, prefs: AppPrefs) -> tauri::Result<()> {
    let config = app
        .config()
        .app
        .windows
        .iter()
        .find(|w| w.label == "main")
        .cloned()
        .ok_or_else(|| tauri::Error::WindowNotFound)?;
    let mut window = WebviewWindowBuilder::from_config(app, &config)?;
    if !prefs.hardware_acceleration {
        window = window.additional_browser_args(&format!("{DEFAULT_BROWSER_ARGS} --disable-gpu --disable-gpu-compositing"));
    }
    let quietly = prefs.close_to_tray && std::env::args().any(|a| a == AUTOSTART_ARG);
    window.visible(!quietly).build()?;
    Ok(())
}

/// The tray icon: a click brings the window back; its menu shows the window
/// or quits the app.
pub fn tray(app: &AppHandle) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "Show RushDownload", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;
    let mut icon = TrayIconBuilder::with_id("main")
        .tooltip("RushDownload")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => bring_back(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                bring_back(tray.app_handle());
            }
        });
    if let Some(image) = app.default_window_icon() {
        icon = icon.icon(image.clone());
    }
    icon.build(app)?;
    app.manage(TrayText { show, quit });
    Ok(())
}

/// Shows the main window again, in front.
fn bring_back(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// With the tray on, closing the main window hides it instead: the app keeps
/// downloading, and the tray brings it back.
pub fn on_window_event(window: &Window, event: &WindowEvent) {
    if window.label() != "main" {
        return;
    }
    if let WindowEvent::CloseRequested { api, .. } = event {
        let to_tray = window.state::<Prefs>().get().close_to_tray;
        if to_tray {
            api.prevent_close();
            let _ = window.hide();
        }
    }
}

/// The preferences, with starting at sign-in as the system has it now.
#[tauri::command]
pub fn get_app_prefs(app: AppHandle) -> AppPrefs {
    let mut prefs = app.state::<Prefs>().get();
    if let Ok(on) = app.autolaunch().is_enabled() {
        prefs.start_with_system = on;
    }
    prefs
}

/// Saves the preferences; starting at sign-in is set with the system at once.
#[tauri::command]
pub fn set_app_prefs(app: AppHandle, prefs: AppPrefs) -> Result<(), String> {
    let launcher = app.autolaunch();
    let on = launcher.is_enabled().unwrap_or(false);
    if prefs.start_with_system != on {
        let done = if prefs.start_with_system { launcher.enable() } else { launcher.disable() };
        done.map_err(|e| format!("could not change starting with the system: {e}"))?;
    }
    app.state::<Prefs>().set(prefs);
    Ok(())
}

/// The tray menu's lines in the interface's language.
#[tauri::command]
pub fn set_tray_text(app: AppHandle, show: String, quit: String) {
    if let Some(text) = app.try_state::<TrayText<tauri::Wry>>() {
        let _ = text.show.set_text(show);
        let _ = text.quit.set_text(quit);
    }
}
