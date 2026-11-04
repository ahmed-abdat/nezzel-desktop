use crate::SharedConfig;
use std::env::args;
use tauri::{AppHandle, State};

#[tauri::command]
pub fn app_ready(app: AppHandle, cfg_handle: State<'_, SharedConfig>) {
  let args: Vec<String> = args().collect();
  let is_autostart = args.contains(&"--auto-start".to_string());
  let cfg = cfg_handle.load();
  // Sign-in launch with "Start with my computer": stay in the tray / menu bar.
  if cfg.system.auto_start_minimised && is_autostart {
    crate::background::start_hidden(&app);
    return;
  }

  crate::background::show_main(&app);
}
