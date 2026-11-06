//! The one choice grammar: what a quick pick (bar) or a format row (card) asks the main window for.

use super::preview;
use serde::{Deserialize, Serialize};

/// Heights the card offers before the read and for links without their own format list (playlists).
pub(crate) const GENERIC_HEIGHTS: [u64; 3] = [1080, 720, 480];
/// Largest numbers a choice may carry: height, frame rate, audio kbps.
pub const MAX_HEIGHT: u64 = 8640;
pub const MAX_FPS: u64 = 480;
pub const MAX_KBPS: u64 = 9999;

/// A pick or a card format row. On the wire a tagged object (`{kind:'video', height:1080, fps:60}`, rendered for
/// TS by `crate::contract`); numbers outside the limits never deserialize, so every `Choice` is one a card could show.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", try_from = "Wire")]
pub enum Choice {
  Best,
  /// The Smaller pick (bar only): best at or under `preview::SMALL_MAX_HEIGHT`, else the next lower.
  Small,
  /// The best audio, or the row of that bitrate.
  Audio {
    #[serde(skip_serializing_if = "Option::is_none")]
    kbps: Option<u64>,
  },
  /// A height row (a ceiling for links without a format list); a high frame rate tier is its own row.
  Video {
    height: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    fps: Option<u64>,
  },
}

/// What arrives before the limits are checked. Best/Small are empty struct variants because serde enforces
/// `deny_unknown_fields` only on struct variants: `{kind:'best', height:1}` is refused, not read as Best.
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
enum Wire {
  Best {},
  Small {},
  Audio {
    #[serde(default)]
    kbps: Option<u64>,
  },
  Video {
    height: u64,
    #[serde(default)]
    fps: Option<u64>,
  },
}

impl TryFrom<Wire> for Choice {
  type Error = &'static str;

  fn try_from(wire: Wire) -> Result<Self, Self::Error> {
    match wire {
      Wire::Best {} => Some(Choice::Best),
      Wire::Small {} => Some(Choice::Small),
      Wire::Audio { kbps } => Choice::audio(kbps),
      Wire::Video { height, fps } => Choice::video(height, fps),
    }
    .ok_or("choice out of range")
  }
}

fn within(n: u64, max: u64) -> bool {
  (1..=max).contains(&n)
}

impl Choice {
  pub fn video(height: u64, fps: Option<u64>) -> Option<Self> {
    (within(height, MAX_HEIGHT) && fps.is_none_or(|fps| within(fps, MAX_FPS)))
      .then_some(Choice::Video { height, fps })
  }

  pub fn audio(kbps: Option<u64>) -> Option<Self> {
    kbps
      .is_none_or(|kbps| within(kbps, MAX_KBPS))
      .then_some(Choice::Audio { kbps })
  }
}

/// Only picks this offer showed cross into the main window; anything else (malformed, out of range, never shown)
/// means "the default choice". A format row must be one this offer showed: a row of its read format list, or, when
/// it has none (still reading, a playlist), one of the generic height ceilings.
pub fn quick_choice(
  choice: Option<serde_json::Value>,
  offered: &[preview::CardChoice],
) -> Option<Choice> {
  let choice: Choice = serde_json::from_value(choice?).ok()?;
  let shown = match choice {
    Choice::Best | Choice::Small | Choice::Audio { kbps: None } => true,
    Choice::Video { height, fps } if offered.is_empty() => {
      fps.is_none() && GENERIC_HEIGHTS.contains(&height)
    }
    row => offered.iter().any(|c| c.choice == row),
  };
  shown.then_some(choice)
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::{json, Value};

  #[test]
  fn only_known_quick_picks_reach_the_main_window() {
    let row = |choice| preview::CardChoice { choice, size: None };
    let offered = [
      row(Choice::Video {
        height: 2160,
        fps: Some(60),
      }),
      row(Choice::Video {
        height: 1080,
        fps: None,
      }),
      row(Choice::Audio { kbps: Some(129) }),
    ];
    let video = |height, fps| Some(Choice::Video { height, fps });
    for (input, out) in [
      (json!({"kind": "best"}), Some(Choice::Best)),
      (json!({"kind": "audio"}), Some(Choice::Audio { kbps: None })),
      (json!({"kind": "small"}), Some(Choice::Small)),
      (json!({"kind": "video", "height": 1080}), video(1080, None)),
      (
        json!({"kind": "video", "height": 2160, "fps": 60}),
        video(2160, Some(60)),
      ),
      (
        json!({"kind": "audio", "kbps": 129}),
        Some(Choice::Audio { kbps: Some(129) }),
      ),
      // Well formed, but this offer never showed them: the default choice.
      (json!({"kind": "video", "height": 8640}), None),
      (json!({"kind": "video", "height": 2160}), None),
      (json!({"kind": "video", "height": 1080, "fps": 60}), None),
      (json!({"kind": "audio", "kbps": 9999}), None),
      (json!({"kind": "video", "height": 720}), None),
      // Malformed: the default choice.
      (json!({"kind": "4k"}), None),
      (json!({"kind": "video", "height": 0}), None),
      (json!({"kind": "video", "height": 99999}), None),
      (json!({"kind": "video"}), None),
      // The old string grammar is gone: nothing persisted it, so nothing still sends it.
      (json!("video:1080"), None),
      (json!("best"), None),
      (Value::Null, None),
    ] {
      assert_eq!(quick_choice(Some(input.clone()), &offered), out, "{input}");
    }
    assert_eq!(quick_choice(None, &offered), None);
    // No format list (still reading, a playlist): only the generic ceilings the card shows.
    for (input, out) in [
      (json!({"kind": "video", "height": 720}), video(720, None)),
      (json!({"kind": "video", "height": 1080}), video(1080, None)),
      (json!({"kind": "video", "height": 1440}), None),
      (json!({"kind": "video", "height": 720, "fps": 60}), None),
      (json!({"kind": "audio", "kbps": 129}), None),
      (json!({"kind": "audio"}), Some(Choice::Audio { kbps: None })),
    ] {
      assert_eq!(
        quick_choice(Some(input.clone()), &[]),
        out,
        "{input} without a list"
      );
    }
  }

  /// The wire shape both sides use (`CaptureChoice` in src/generated/contract.ts): plain numbers within the limits.
  #[test]
  fn choices_parse_only_numbers_within_the_limits() {
    let parse = |value: Value| serde_json::from_value::<Choice>(value).ok();
    let video = |height, fps| Some(Choice::Video { height, fps });
    for (input, expected) in [
      (json!({"kind": "video", "height": 1080}), video(1080, None)),
      (
        json!({"kind": "video", "height": 1080, "fps": 60}),
        video(1080, Some(60)),
      ),
      (
        json!({"kind": "video", "height": 1080, "fps": null}),
        video(1080, None),
      ),
      (
        json!({"kind": "video", "height": 8640, "fps": 480}),
        video(8640, Some(480)),
      ),
      (json!({"kind": "video", "height": 8641}), None),
      (json!({"kind": "video", "height": 1080, "fps": 481}), None),
      (json!({"kind": "video", "height": 0}), None),
      (json!({"kind": "video", "height": 1080, "fps": 0}), None),
      (json!({"kind": "video", "height": -1}), None),
      (json!({"kind": "video", "height": 1080.5}), None),
      (json!({"kind": "video", "height": "1080"}), None),
      (json!({"kind": "video", "height": 1080, "fps": "abc"}), None),
      (json!({"kind": "video", "height": 1080, "extra": 1}), None),
      (
        json!({"kind": "audio", "kbps": 129}),
        Some(Choice::Audio { kbps: Some(129) }),
      ),
      (
        json!({"kind": "audio", "kbps": 9999}),
        Some(Choice::Audio { kbps: Some(9999) }),
      ),
      (json!({"kind": "audio", "kbps": 10000}), None),
      (json!({"kind": "audio", "kbps": 0}), None),
      (json!({"kind": "audio"}), Some(Choice::Audio { kbps: None })),
      (json!({"kind": "best"}), Some(Choice::Best)),
      (json!({"kind": "best", "height": 1}), None),
      (json!({"kind": "Video", "height": 1080}), None),
      (json!({"height": 1080}), None),
    ] {
      assert_eq!(parse(input.clone()), expected, "{input}");
    }
    // Serializing leaves absent numbers out, and reads back the same.
    for choice in [
      Choice::Best,
      Choice::Small,
      Choice::Audio { kbps: None },
      Choice::Audio { kbps: Some(129) },
      Choice::Video {
        height: 1080,
        fps: None,
      },
      Choice::Video {
        height: 1080,
        fps: Some(60),
      },
    ] {
      let wire = serde_json::to_value(choice).unwrap();
      assert_eq!(parse(wire.clone()), Some(choice), "{wire}");
    }
    assert_eq!(
      serde_json::to_value(Choice::Video {
        height: 1080,
        fps: None
      })
      .unwrap(),
      json!({"kind": "video", "height": 1080})
    );
  }
}
