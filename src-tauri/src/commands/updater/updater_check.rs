use crate::commands::UpdateStore;
use crate::paths::PathsManager;
use tauri::{AppHandle, State};
use tauri_plugin_updater::{Config, Error, Result as UpdaterResult, Update, UpdaterExt};
use tokio::sync::Mutex;

#[derive(serde::Serialize)]
pub struct UpdateCheck {
  pub available: bool,
  pub current_version: String,
  pub available_version: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
enum ManifestStatus {
  Missing,
  Check,
  Failed,
}

fn manifest_status(status: u16) -> ManifestStatus {
  match status {
    404 => ManifestStatus::Missing,
    200..=299 => ManifestStatus::Check,
    _ => ManifestStatus::Failed,
  }
}

async fn check_published_release(app: &AppHandle) -> UpdaterResult<Option<Update>> {
  let config: Config = app
    .config()
    .plugins
    .0
    .get("updater")
    .cloned()
    .map(serde_json::from_value)
    .transpose()?
    .unwrap_or_default();
  if config.endpoints.is_empty() {
    return Ok(None);
  }
  let client = reqwest::Client::builder()
    .timeout(std::time::Duration::from_secs(10))
    .build()?;
  for endpoint in config.endpoints {
    // The plugin erases all non-2xx statuses into ReleaseNotFound. Only a confirmed 404
    // means "not published"; transport, permission and server failures must remain errors.
    let status = client.get(endpoint.clone()).send().await?.status().as_u16();
    match manifest_status(status) {
      ManifestStatus::Missing => continue,
      ManifestStatus::Failed => return Err(Error::ReleaseNotFound),
      ManifestStatus::Check => {
        // Probe and check the same endpoint. A later failure is still an error (including
        // ReleaseNotFound if the response changes between the two GETs).
        return app
          .updater_builder()
          .endpoints(vec![endpoint])?
          .build()?
          .check()
          .await;
      }
    }
  }
  Ok(None)
}

#[tauri::command]
pub async fn updater_check(
  app: AppHandle,
  store: State<'_, Mutex<UpdateStore>>,
  paths: State<'_, PathsManager>,
) -> UpdaterResult<UpdateCheck> {
  let current = app.package_info().version.to_string();
  // Portable app does not auto-update, nor does the microsoft/snap store version.
  if paths.is_microsoft_store_app() || paths.is_portable_app() || paths.is_snap_app() {
    return Ok(UpdateCheck {
      available: false,
      current_version: current,
      available_version: None,
    });
  }
  let checked = check_published_release(&app).await?;
  if let Some(update) = checked {
    tracing::info!("[updater] Update available: {}", update.version);
    crate::usage::update_offered(&app, &update.version);
    let mut s = store.lock().await;
    s.bytes = None;
    s.update = Some(update.clone());
    Ok(UpdateCheck {
      available: true,
      current_version: current,
      available_version: Some(update.version),
    })
  } else {
    let mut s = store.lock().await;
    s.bytes = None;
    s.update = None;
    Ok(UpdateCheck {
      available: false,
      current_version: current,
      available_version: None,
    })
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn only_missing_manifests_are_latest_and_successes_reach_the_plugin() {
    for (status, expected) in [
      (200, ManifestStatus::Check),
      (204, ManifestStatus::Check),
      (302, ManifestStatus::Failed),
      (403, ManifestStatus::Failed),
      (404, ManifestStatus::Missing),
      (500, ManifestStatus::Failed),
      (503, ManifestStatus::Failed),
    ] {
      assert_eq!(manifest_status(status), expected, "HTTP {status}");
    }
  }
}
