//! The request-profile commands (Settings, Network, Request profiles). Each
//! change answers with the profiles as they now are. Private values (a
//! password, a token, cookies) go in and never come back out.

use rushdownload_engine::{PasswordChange, RequestBook, RequestOptions};
use tauri::State;

use crate::AppState;

#[tauri::command]
pub fn list_request_profiles(state: State<AppState>) -> RequestBook {
    state.0.request_profiles()
}

/// Saves a new profile (`id` 0) or changes one.
#[tauri::command]
pub fn save_request_profile(
    state: State<AppState>,
    id: u64,
    name: String,
    options: RequestOptions,
    password: PasswordChange,
    cookies: PasswordChange,
) -> Result<RequestBook, String> {
    state.0.save_request_profile(id, &name, options, password, cookies)?;
    Ok(state.0.request_profiles())
}

#[tauri::command]
pub fn delete_request_profile(state: State<AppState>, id: u64) -> Result<RequestBook, String> {
    state.0.delete_request_profile(id)?;
    Ok(state.0.request_profiles())
}

#[tauri::command]
pub fn set_default_request_profile(state: State<AppState>, id: Option<u64>) -> Result<RequestBook, String> {
    state.0.set_default_request_profile(id)?;
    Ok(state.0.request_profiles())
}
