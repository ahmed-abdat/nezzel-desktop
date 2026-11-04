use crate::models::TrackType;
use crate::runners::template_context::TemplateContext;
use crate::state::config_models::OutputSettings;
use crate::state::preferences_models::PathPreferences;
use std::path::{Path, PathBuf};

/// Folder inside the system Downloads folder where Nezzel saves by default (same as the Android app).
pub const DEFAULT_DOWNLOAD_SUBDIR: &str = "Nezzel";

/// Default save folder when the user has not picked one: `<system Downloads>/Nezzel`.
/// yt-dlp creates it on the first download.
pub fn default_download_dir(system_downloads: &Path) -> PathBuf {
  system_downloads.join(DEFAULT_DOWNLOAD_SUBDIR)
}

/// Folder a download of `track_type` is saved under (before the directory template and file name).
pub fn output_base_dir(
  track_type: &TrackType,
  output_settings: &OutputSettings,
  path_preferences: &PathPreferences,
  fallback_dir: PathBuf,
) -> PathBuf {
  let global_dir = output_settings
    .download_dir
    .as_ref()
    .map(PathBuf::from)
    .unwrap_or(fallback_dir);
  let preferred_dir = match track_type {
    TrackType::Both | TrackType::Video => path_preferences.video_download_dir.as_ref(),
    TrackType::Audio => path_preferences.audio_download_dir.as_ref(),
  };
  preferred_dir.map(PathBuf::from).unwrap_or(global_dir)
}

pub fn build_location_args(
  track_type: &TrackType,
  template_context: &TemplateContext,
  output_settings: &OutputSettings,
  path_preferences: &PathPreferences,
  fallback_dir: PathBuf,
) -> Vec<String> {
  let (prefix_dir, filename) = match track_type {
    TrackType::Both | TrackType::Video => (
      path_preferences.video_directory_template.as_str(),
      output_settings.file_name_template.as_str(),
    ),
    TrackType::Audio => (
      path_preferences.audio_directory_template.as_str(),
      output_settings.audio_file_name_template.as_str(),
    ),
  };
  let base_dir = output_base_dir(track_type, output_settings, path_preferences, fallback_dir);

  let output_path = base_dir.join(prefix_dir).join(filename);
  let rendered_output = template_context.render_template(output_path.to_string_lossy().as_ref());

  vec!["-o".into(), rendered_output]
}

/// A folder picked for one download wins over the per-track (video/audio) folders from Preferences.
pub fn path_preferences_for(
  path_preferences: &PathPreferences,
  download_dir_override: Option<&str>,
) -> PathPreferences {
  let mut prefs = path_preferences.clone();
  if download_dir_override.is_some_and(|dir| !dir.is_empty()) {
    prefs.video_download_dir = None;
    prefs.audio_download_dir = None;
  }
  prefs
}
