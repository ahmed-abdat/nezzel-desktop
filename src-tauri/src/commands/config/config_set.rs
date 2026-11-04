use crate::state::config_models::Config;
use crate::SharedConfig;
use serde_json::Value;
use tauri::{AppHandle, State};

#[tauri::command]
pub fn config_set(
  app: AppHandle,
  cfg: State<'_, SharedConfig>,
  patch: Value,
) -> Result<Config, String> {
  cfg
    .apply_patch(&super::client_patch(patch), &app)
    .map(super::public_config)
    .map_err(|e| e.to_string())
}
