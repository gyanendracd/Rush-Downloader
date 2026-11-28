//! The helper-program commands the interface calls (Settings, Advanced,
//! Engines). Progress of a check or an install also arrives as "tools"
//! events (see lib.rs), so these may be called and left to run.

use rushdownload_engine::ToolInfo;
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

use crate::AppState;

#[tauri::command]
pub fn tools_info(state: State<AppState>) -> Vec<ToolInfo> {
    state.0.tools_info()
}

/// Looks for new releases of every tool now.
#[tauri::command]
pub async fn check_tools(state: State<'_, AppState>) -> Result<Vec<ToolInfo>, String> {
    let engine = state.0.clone();
    Ok(engine.check_tools().await)
}

/// Installs or updates one tool; answers once it is done or has failed.
#[tauri::command]
pub async fn install_tool(state: State<'_, AppState>, name: String) -> Result<ToolInfo, String> {
    let engine = state.0.clone();
    engine.install_tool(&name).await
}

#[tauri::command]
pub fn rollback_tool(state: State<AppState>, name: String) -> Result<ToolInfo, String> {
    state.0.rollback_tool(&name)
}

/// Opens a tool's release notes in the browser: only the page its release
/// came from, on its project's own site.
#[tauri::command]
pub fn open_release_notes(app: AppHandle, state: State<AppState>, name: String) -> Result<(), String> {
    let url = state
        .0
        .tools_info()
        .into_iter()
        .find(|t| t.name == name)
        .and_then(|t| t.notes_url)
        .ok_or("There are no release notes to show yet; check for updates first")?;
    let parsed = tauri::Url::parse(&url).map_err(|_| "The release notes have an address that cannot be opened".to_string())?;
    let trusted = parsed.scheme() == "https" && matches!(parsed.host_str(), Some("github.com" | "codeberg.org"));
    if !trusted {
        return Err("The release notes are not on the project's own site, so they are not opened".into());
    }
    app.opener().open_url(url, None::<&str>).map_err(|e| e.to_string())
}
