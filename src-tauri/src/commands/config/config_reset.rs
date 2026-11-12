use crate::state::config_models::Config;
use crate::SharedConfig;
use tauri::{AppHandle, State};

#[tauri::command]
pub fn config_reset(app: AppHandle, cfg: State<'_, SharedConfig>) -> Result<Config, String> {
  cfg
    .modify(&app, super::reset_user_config)
    .map(super::public_config)
    .map_err(|e| e.to_string())
}
