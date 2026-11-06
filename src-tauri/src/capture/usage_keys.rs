//! The usage keys a capture window action counts. The allowed keys live in `usage::keys`; this only picks them.

use super::window::CARD_LABEL;
use super::{Action, Choice};
use crate::usage::keys;

/// A Download/Options action on the Quick bar or the copied-link card: the quick pick, and `clip.accepted` for the card.
pub fn action_keys(window: &str, action: Action, choice: Option<Choice>) -> Vec<&'static str> {
  let pick = match (action, choice) {
    (Action::Options, _) => keys::QUICK_OPTIONS,
    (Action::Download, Some(Choice::Audio { kbps: None })) => keys::QUICK_AUDIO,
    (Action::Download, Some(Choice::Small)) => keys::QUICK_SMALLER,
    (Action::Download, Some(Choice::Video { .. })) => keys::CARD_FORMAT_VIDEO,
    (Action::Download, Some(Choice::Audio { kbps: Some(_) })) => keys::CARD_FORMAT_AUDIO,
    (Action::Download, Some(Choice::Best) | None) => keys::QUICK_DOWNLOAD,
    _ => return Vec::new(),
  };
  let mut out = vec![pick];
  if window == CARD_LABEL {
    out.push(keys::CLIP_ACCEPTED);
  }
  out
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::capture::window::BAR_LABEL;

  #[test]
  fn action_keys_count_the_pick_and_card_use() {
    assert_eq!(
      action_keys(BAR_LABEL, Action::Download, None),
      vec![keys::QUICK_DOWNLOAD]
    );
    assert_eq!(
      action_keys(BAR_LABEL, Action::Download, Some(Choice::Best)),
      vec![keys::QUICK_DOWNLOAD]
    );
    assert_eq!(
      action_keys(
        CARD_LABEL,
        Action::Download,
        Some(Choice::Audio { kbps: None })
      ),
      vec![keys::QUICK_AUDIO, keys::CLIP_ACCEPTED]
    );
    assert_eq!(
      action_keys(BAR_LABEL, Action::Download, Some(Choice::Small)),
      vec![keys::QUICK_SMALLER]
    );
    assert_eq!(
      action_keys(CARD_LABEL, Action::Options, None),
      vec![keys::QUICK_OPTIONS, keys::CLIP_ACCEPTED]
    );
    assert_eq!(
      action_keys(
        CARD_LABEL,
        Action::Download,
        Some(Choice::Video {
          height: 720,
          fps: Some(60)
        })
      ),
      vec![keys::CARD_FORMAT_VIDEO, keys::CLIP_ACCEPTED]
    );
    assert_eq!(
      action_keys(
        CARD_LABEL,
        Action::Download,
        Some(Choice::Audio { kbps: Some(129) })
      ),
      vec![keys::CARD_FORMAT_AUDIO, keys::CLIP_ACCEPTED]
    );
    assert!(action_keys(CARD_LABEL, Action::Dismiss, None).is_empty());
    assert!(action_keys(CARD_LABEL, Action::Report, None).is_empty());
  }

  #[test]
  fn every_action_key_is_allowed() {
    for window in [BAR_LABEL, CARD_LABEL] {
      for choice in [
        None,
        Some(Choice::Best),
        Some(Choice::Small),
        Some(Choice::Audio { kbps: None }),
        Some(Choice::Audio { kbps: Some(192) }),
        Some(Choice::Video {
          height: 720,
          fps: None,
        }),
      ] {
        for action in [Action::Download, Action::Options] {
          for key in action_keys(window, action, choice) {
            assert!(keys::allowed(key), "{key}");
          }
        }
      }
    }
  }
}
