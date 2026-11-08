use crate::commands::NotificationKind;
use crate::models::download::{
  AudioFormat, AudioPostprocessPreset, TranscodePolicy, VideoContainer, VideoPostprocessMode,
  VideoPostprocessPreset,
};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, thread};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AppearanceSettings {
  pub theme: String,
  pub language: String,
  pub expanded_options: String,
}

impl Default for AppearanceSettings {
  fn default() -> Self {
    Self {
      theme: "system".into(),
      language: "system".into(),
      expanded_options: "none".into(),
    }
  }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AuthSettings {
  pub cookie_file: Option<String>,
  pub cookie_browser: String,
  pub always_use_browser_logins: bool,
  pub site_browsers: HashMap<String, String>,
}

impl Default for AuthSettings {
  fn default() -> Self {
    Self {
      cookie_file: None,
      cookie_browser: "automatic".into(),
      always_use_browser_logins: false,
      site_browsers: HashMap::new(),
    }
  }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct NetworkSettings {
  pub enable_proxy: Option<bool>,
  pub proxy: Option<String>,
  pub impersonate: String,
  pub extractor_args: String,
}

impl Default for NetworkSettings {
  fn default() -> Self {
    Self {
      enable_proxy: None,
      proxy: None,
      impersonate: "none".into(),
      extractor_args: String::new(),
    }
  }
}

/// Logical screen corner for the copied-link card; Automatic preserves the platform default.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CardPosition {
  #[default]
  Automatic,
  TopStart,
  TopEnd,
  BottomStart,
  BottomEnd,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct InputSettings {
  pub auto_fill_clipboard: bool,
  pub watch_clipboard: bool,
  pub card_keep_open: bool,
  pub card_position: CardPosition,
  pub capture_shortcut: String,
  pub prefer_video_in_mixed_links: bool,
  pub global_shortcuts: bool,
}

/// Quick download default: ⌥⌘N on macOS, Ctrl+Alt+N on Windows/Linux (owner decision 2026-11-04).
pub const DEFAULT_CAPTURE_SHORTCUT: &str = "CommandOrControl+Alt+N";
/// Registered instead of the default when the OS or another app owns it, or when Ctrl+Alt+N types a
/// character on the Windows keyboard layout (AltGr+N, e.g. Polish ń). Never stored in the config.
pub const FALLBACK_CAPTURE_SHORTCUT: &str = "CommandOrControl+Alt+Shift+D";
/// Earlier defaults: ⌘⇧D / Ctrl+Shift+D (collides with browsers), then ⌥⇧⌘D / Ctrl+Alt+Shift+D.
pub const LEGACY_CAPTURE_SHORTCUTS: [&str; 2] =
  ["CommandOrControl+Shift+D", "CommandOrControl+Alt+Shift+D"];

impl InputSettings {
  /// Moves a stored shortcut that is still one of the earlier defaults to the current default.
  pub fn migrate_capture_shortcut(&mut self) {
    if LEGACY_CAPTURE_SHORTCUTS.contains(&self.capture_shortcut.as_str()) {
      self.capture_shortcut = DEFAULT_CAPTURE_SHORTCUT.into();
    }
  }
}

impl Default for InputSettings {
  fn default() -> Self {
    Self {
      auto_fill_clipboard: true,
      watch_clipboard: true,
      card_keep_open: false,
      card_position: CardPosition::Automatic,
      capture_shortcut: DEFAULT_CAPTURE_SHORTCUT.into(),
      prefer_video_in_mixed_links: false,
      global_shortcuts: true,
    }
  }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "UPPERCASE")]
pub enum InputFilterSizeUnit {
  #[default]
  B,
  KB,
  MB,
  GB,
  TB,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct InputFilterSizeFilter {
  pub value: Option<f64>,
  pub unit: Option<InputFilterSizeUnit>,
}

impl Default for InputFilterSizeFilter {
  fn default() -> Self {
    Self {
      value: None,
      unit: Some(InputFilterSizeUnit::MB),
    }
  }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum InputFilterDateMode {
  #[default]
  Exact,
  Before,
  After,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct InputFilterDateFilter {
  pub mode: Option<InputFilterDateMode>,
  pub value: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct InputFilterSettings {
  pub min_size: InputFilterSizeFilter,
  pub max_size: InputFilterSizeFilter,
  pub date_filter: InputFilterDateFilter,
  pub match_filters: Option<String>,
  pub break_match_filters: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoOutputSettings {
  pub container: VideoContainer,
  #[serde(default)]
  pub keep_original_codec: bool,
  pub policy: TranscodePolicy,
  pub postprocess_preset: VideoPostprocessPreset,
  pub custom_postprocess_mode: VideoPostprocessMode,
  pub postprocess_args: String,
}

impl Default for VideoOutputSettings {
  fn default() -> Self {
    Self {
      policy: TranscodePolicy::AllowReencode,
      container: VideoContainer::Mp4,
      keep_original_codec: false,
      postprocess_preset: VideoPostprocessPreset::None,
      custom_postprocess_mode: VideoPostprocessMode::Remux,
      postprocess_args: String::new(),
    }
  }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioOutputSettings {
  pub format: AudioFormat,
  pub policy: TranscodePolicy,
  pub postprocess_preset: AudioPostprocessPreset,
  pub postprocess_args: String,
}

impl Default for AudioOutputSettings {
  fn default() -> Self {
    Self {
      policy: TranscodePolicy::AllowReencode,
      format: AudioFormat::Mp3,
      postprocess_preset: AudioPostprocessPreset::None,
      postprocess_args: String::new(),
    }
  }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct OutputSettings {
  pub video: VideoOutputSettings,
  pub audio: AudioOutputSettings,
  pub add_metadata: bool,
  pub add_thumbnail: bool,
  pub save_thumbnail: bool,
  pub precise_cuts: bool,
  pub reverse_playlist_numbering: bool,
  pub download_dir: Option<String>,
  pub file_name_template: String,
  pub audio_file_name_template: String,
  pub restrict_filenames: bool,
}

impl Default for OutputSettings {
  fn default() -> Self {
    Self {
      video: VideoOutputSettings::default(),
      audio: AudioOutputSettings::default(),
      add_metadata: true,
      add_thumbnail: true,
      save_thumbnail: false,
      precise_cuts: false,
      reverse_playlist_numbering: false,
      download_dir: None,
      file_name_template: "%(title).200s-(%(height)sp%(fps).0d).%(ext)s".into(),
      audio_file_name_template: "%(title).200s-(%(abr)dk).%(ext)s".into(),
      restrict_filenames: false,
    }
  }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PerformanceSettings {
  pub max_concurrency: usize,
  pub split_playlist_threshold: usize,
  pub auto_load_size: bool,
  #[serde(deserialize_with = "deserialize_fragments")]
  pub concurrent_fragments: u8,
  pub aria2_enabled: bool,
}

impl Default for PerformanceSettings {
  fn default() -> Self {
    Self {
      max_concurrency: thread::available_parallelism()
        .map(|n| n.get().div_ceil(2))
        .unwrap_or(1),
      split_playlist_threshold: 50,
      auto_load_size: true,
      concurrent_fragments: 4,
      aria2_enabled: false,
    }
  }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SponsorBlockSettings {
  pub api_url: Option<String>,
  pub remove_parts: Vec<String>,
  pub mark_parts: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SubtitleSettings {
  pub enabled: bool,
  pub include_auto_generated: bool,
  pub languages: Vec<String>,
  pub format_preference: Vec<String>,
  pub embed_subtitles: bool,
}

impl Default for SubtitleSettings {
  fn default() -> Self {
    Self {
      enabled: false,
      include_auto_generated: false,
      languages: vec!["en".into()],
      embed_subtitles: true,
      format_preference: vec![
        "srt".into(),
        "vtt".into(),
        "ass".into(),
        "ttml".into(),
        "json3".into(),
      ],
    }
  }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct UpdateSettings {
  pub update_binaries: bool,
  pub update_app: bool,
}

impl Default for UpdateSettings {
  fn default() -> Self {
    Self {
      update_binaries: true,
      update_app: true,
    }
  }
}

/// Crash and error reports to our own Sentry (build-time DSN). On by default, owner decision 2026-11-04.
/// Usage counts ("Help improve Nezzel", `usage/`): on by default, owner decision 2026-11-05.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PrivacySettings {
  pub send_reports: bool,
  pub send_usage: bool,
}

impl Default for PrivacySettings {
  fn default() -> Self {
    Self {
      send_reports: true,
      send_usage: true,
    }
  }
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CloseBehavior {
  Exit,
  Hide,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SystemConfig {
  pub tray_enabled: bool,
  pub auto_start_enabled: bool,
  pub auto_start_minimised: bool,
  pub close_behavior: CloseBehavior,
}

impl Default for SystemConfig {
  fn default() -> Self {
    #[cfg(target_os = "windows")]
    {
      Self {
        tray_enabled: true,
        auto_start_enabled: false,
        auto_start_minimised: false,
        close_behavior: CloseBehavior::Hide,
      }
    }

    #[cfg(target_os = "macos")]
    {
      Self {
        tray_enabled: true,
        auto_start_enabled: false,
        auto_start_minimised: false,
        close_behavior: CloseBehavior::Hide,
      }
    }

    #[cfg(target_os = "linux")]
    {
      Self {
        tray_enabled: true,
        auto_start_enabled: false,
        auto_start_minimised: false,
        close_behavior: CloseBehavior::Hide,
      }
    }
  }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NotificationBehavior {
  Always,
  OnBackground,
  Never,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct NotificationConfig {
  pub notification_behavior: NotificationBehavior,
  pub disabled_notifications: Vec<NotificationKind>,
}

impl Default for NotificationConfig {
  fn default() -> Self {
    Self {
      // Background mode: a "Saved" notification while the window is hidden, nothing while it shows.
      notification_behavior: NotificationBehavior::OnBackground,
      disabled_notifications: vec![
        NotificationKind::QueueAdded,
        NotificationKind::QueueDownloading,
        NotificationKind::QueueFinished,
        NotificationKind::VideoReady,
        NotificationKind::PlaylistReady,
      ],
    }
  }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Config {
  pub appearance: AppearanceSettings,
  pub auth: AuthSettings,
  pub network: NetworkSettings,
  pub input: InputSettings,
  pub input_filters: InputFilterSettings,
  pub output: OutputSettings,
  pub performance: PerformanceSettings,
  pub sponsor_block: SponsorBlockSettings,
  pub subtitles: SubtitleSettings,
  pub update: UpdateSettings,
  pub system: SystemConfig,
  pub notifications: NotificationConfig,
  pub privacy: PrivacySettings,
}

fn deserialize_fragments<'de, D: serde::Deserializer<'de>>(d: D) -> Result<u8, D::Error> {
  let value = u8::deserialize(d)?;
  if !(1..=16).contains(&value) {
    return Err(serde::de::Error::custom("concurrentFragments must be 1–16"));
  }
  Ok(value)
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;
  #[test]
  fn engine_fragment_config_defaults_and_rejects_out_of_bounds() {
    assert_eq!(AuthSettings::default().cookie_browser, "automatic");
    assert_eq!(
      serde_json::from_value::<Config>(json!({}))
        .unwrap()
        .performance
        .concurrent_fragments,
      4
    );
    for value in [1, 16] {
      assert_eq!(
        serde_json::from_value::<PerformanceSettings>(json!({"concurrentFragments":value}))
          .unwrap()
          .concurrent_fragments,
        value
      );
    }
    for value in [0, 17, 256, -1] {
      assert!(
        serde_json::from_value::<PerformanceSettings>(json!({"concurrentFragments":value}))
          .is_err()
      );
    }
    let old = serde_json::from_value::<Config>(
      json!({"auth":{"cookieBrowser":"firefox","cookieFile":"/tmp/manual.txt"}}),
    )
    .unwrap();
    assert_eq!(old.auth.cookie_browser, "firefox");
    assert_eq!(old.auth.cookie_file.as_deref(), Some("/tmp/manual.txt"));
    assert!(!old.auth.always_use_browser_logins);
    assert!(old.auth.site_browsers.is_empty());
  }
}

#[cfg(test)]
mod capture_shortcut_tests {
  use super::*;

  #[test]
  fn earlier_defaults_move_to_the_current_default() {
    let cases = [
      ("CommandOrControl+Shift+D", DEFAULT_CAPTURE_SHORTCUT),
      ("CommandOrControl+Alt+Shift+D", DEFAULT_CAPTURE_SHORTCUT),
      ("CommandOrControl+Alt+N", DEFAULT_CAPTURE_SHORTCUT),
      ("CommandOrControl+Alt+K", "CommandOrControl+Alt+K"),
      ("", ""),
    ];
    for (stored, expected) in cases {
      let mut input = InputSettings {
        capture_shortcut: stored.into(),
        ..InputSettings::default()
      };
      input.migrate_capture_shortcut();
      assert_eq!(input.capture_shortcut, expected, "stored {stored:?}");
    }
  }
}
