use crate::state::json_handle::JsonStoreHandle;
use crate::state::json_state::{json_merge, JsonBackedState};
use crate::state::preferences_models::Preferences;
use serde_json::{json, Value};
use std::error::Error;

impl JsonBackedState for Preferences {
  const STORE_FILE: &'static str = "preferences.store.json";
  const ROOT_KEY: &'static str = "preferences";

  fn default_value() -> Self {
    Self::default()
  }

  fn materialize(raw: &Value) -> Result<Self, Box<dyn Error>> {
    let mut merged = serde_json::to_value(Self::default_value())?;
    json_merge(&mut merged, &with_existing_user_onboarding(raw));
    Ok(serde_json::from_value(merged)?)
  }
}

/// Preferences saved before onboarding existed belong to someone who already used Nezzel:
/// they skip the welcome sheet. A fresh install has no saved preferences at all.
fn with_existing_user_onboarding(raw: &Value) -> Value {
  let mut raw = raw.clone();
  if let Value::Object(map) = &mut raw {
    if !map.is_empty() && !map.contains_key("onboarding") {
      map.insert("onboarding".into(), json!({ "welcomeDone": true }));
    }
  }
  raw
}

pub type PreferencesHandle = JsonStoreHandle<Preferences>;

#[cfg(test)]
mod tests {
  use super::*;

  fn onboarding(raw: Value) -> (bool, bool) {
    let p = Preferences::materialize(&raw).unwrap();
    (p.onboarding.welcome_done, p.onboarding.close_note_done)
  }

  #[test]
  fn first_run_detection_from_saved_preferences() {
    // Fresh install: nothing saved yet, so the welcome shows.
    assert_eq!(onboarding(json!({})), (false, false));
    // Saved before onboarding existed: an existing user skips the welcome, still gets the note once.
    assert_eq!(
      onboarding(json!({ "window": { "width": 800, "height": 900 } })),
      (true, false)
    );
    // Saved after onboarding existed: stored flags win.
    assert_eq!(
      onboarding(json!({ "window": { "width": 800 }, "onboarding": { "welcomeDone": false } })),
      (false, false)
    );
    assert_eq!(
      onboarding(json!({ "onboarding": { "welcomeDone": true, "closeNoteDone": true } })),
      (true, true)
    );
  }
}
