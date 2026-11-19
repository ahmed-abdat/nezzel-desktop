//! App-command access for each window. Plugin commands use Tauri's capability checks separately.

use crate::background::NOTE_LABEL;
use crate::capture::{BAR_LABEL, CARD_LABEL};

pub(crate) fn allowed_for_window(label: &str, command: &str) -> bool {
  // Keep these lists exact: a new capture_* command is not automatically safe for an aux window.
  match label {
    "main" => true,
    CARD_LABEL => matches!(
      command,
      "config_get" | "capture_snapshot" | "capture_action" | "capture_retry" | "capture_fit"
    ),
    BAR_LABEL => matches!(
      command,
      "config_get"
        | "capture_snapshot"
        | "capture_action"
        | "capture_retry"
        | "capture_edit"
        | "get_platform"
        | "capture_open_automation_settings"
    ),
    NOTE_LABEL => matches!(
      command,
      "config_get" | "close_note_ready" | "close_note_done"
    ),
    // Includes site-login-* and any future window until its access is explicitly defined.
    _ => false,
  }
}

#[cfg(test)]
mod tests {
  use super::allowed_for_window;
  use crate::background::NOTE_LABEL;
  use crate::capture::{BAR_LABEL, CARD_LABEL};

  #[test]
  fn allowed_for_window_limits_auxiliary_commands() {
    let cases = [
      ("main", "stronghold_get", true),
      ("main", "config_set", true),
      ("main", "media_trash_file", true),
      (CARD_LABEL, "config_get", true),
      (CARD_LABEL, "capture_snapshot", true),
      (CARD_LABEL, "capture_action", true),
      (CARD_LABEL, "capture_retry", true),
      (CARD_LABEL, "capture_fit", true),
      (CARD_LABEL, "stronghold_get", false),
      (CARD_LABEL, "config_set", false),
      (CARD_LABEL, "media_trash_file", false),
      (CARD_LABEL, "capture_edit", false),
      (CARD_LABEL, "capture_pull", false),
      (CARD_LABEL, "capture_ready", false),
      (CARD_LABEL, "capture_future_command", false),
      (BAR_LABEL, "config_get", true),
      (BAR_LABEL, "capture_snapshot", true),
      (BAR_LABEL, "capture_action", true),
      (BAR_LABEL, "capture_retry", true),
      (BAR_LABEL, "capture_edit", true),
      (BAR_LABEL, "get_platform", true),
      (BAR_LABEL, "capture_open_automation_settings", true),
      (BAR_LABEL, "stronghold_get", false),
      (BAR_LABEL, "config_set", false),
      (BAR_LABEL, "media_trash_file", false),
      (BAR_LABEL, "capture_fit", false),
      (BAR_LABEL, "capture_future_command", false),
      (NOTE_LABEL, "config_get", true),
      (NOTE_LABEL, "close_note_ready", true),
      (NOTE_LABEL, "close_note_done", true),
      (NOTE_LABEL, "stronghold_get", false),
      (NOTE_LABEL, "config_set", false),
      (NOTE_LABEL, "media_trash_file", false),
      (NOTE_LABEL, "capture_action", false),
      (NOTE_LABEL, "close_note_future_command", false),
      ("site-login-0", "config_get", false),
      ("site-login-1", "stronghold_get", false),
      ("unknown", "config_get", false),
      ("capture-card-extra", "capture_action", false),
      ("", "close_note_done", false),
    ];
    for (label, command, expected) in cases {
      assert_eq!(
        allowed_for_window(label, command),
        expected,
        "{label}: {command}"
      );
    }
  }
}
