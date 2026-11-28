//! The proxy commands the interface calls (Settings, Network, Proxy, and the
//! connection picker of a new download). Every change answers with the
//! proxies as they now are, so the interface never shows a stale list.

use rushdownload_engine::{PasswordChange, ProxyBook, ProxyChoice, ProxyProfile, ProxyTest};
use tauri::State;

use crate::AppState;

#[tauri::command]
pub fn list_proxies(state: State<AppState>) -> ProxyBook {
    state.0.proxies()
}

/// Saves a new proxy (`profile.id` 0) or changes one.
#[tauri::command]
pub fn save_proxy(state: State<AppState>, profile: ProxyProfile, password: PasswordChange) -> Result<ProxyBook, String> {
    state.0.save_proxy(profile, password)?;
    Ok(state.0.proxies())
}

#[tauri::command]
pub fn delete_proxy(state: State<AppState>, id: u64) -> Result<ProxyBook, String> {
    state.0.delete_proxy(id)?;
    Ok(state.0.proxies())
}

/// `id` none: downloads on the default connection connect directly.
#[tauri::command]
pub fn set_default_proxy(state: State<AppState>, id: Option<u64>) -> Result<ProxyBook, String> {
    state.0.set_default_proxy(id)?;
    Ok(state.0.proxies())
}

/// Tries a proxy as it is filled in, saved or not.
#[tauri::command]
pub async fn test_proxy(
    state: State<'_, AppState>,
    profile: ProxyProfile,
    password: PasswordChange,
    url: Option<String>,
) -> Result<ProxyTest, String> {
    let engine = state.0.clone();
    Ok(engine.test_proxy(profile, password, url).await)
}

#[tauri::command]
pub fn set_download_proxy(state: State<AppState>, id: u64, choice: ProxyChoice) -> Result<(), String> {
    state.0.set_download_proxy(id, choice)
}
