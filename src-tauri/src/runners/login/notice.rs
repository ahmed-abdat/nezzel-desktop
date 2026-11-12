//! One nonblocking notice after actual use; persist only after the webview acknowledges display.
use crate::{state::config_models::AuthSettings, SharedConfig};
use serde::Serialize;
use std::sync::{LazyLock, Mutex};
use tauri::{AppHandle, Manager};

#[derive(Clone)]
pub(in crate::runners) struct LoginUsage {
  pub browser: &'static str,
  pub host: String,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LoginNotice {
  token: String,
  browser: String,
  host: String,
}
#[derive(Default)]
struct Delivery {
  pending: Option<LoginNotice>,
}
impl Delivery {
  fn used(&mut self, shown: bool, usage: &LoginUsage) -> Option<LoginNotice> {
    if shown {
      return None;
    }
    Some(
      self
        .pending
        .get_or_insert_with(|| LoginNotice {
          token: uuid::Uuid::new_v4().to_string(),
          browser: usage.browser.to_owned(),
          host: usage.host.clone(),
        })
        .clone(),
    )
  }
  fn confirm(&self, auth: &mut AuthSettings, token: &str) -> bool {
    if !self
      .pending
      .as_ref()
      .is_some_and(|notice| notice.token == token)
    {
      return false;
    }
    auth.browser_login_notice_shown = true;
    true
  }
}
static DELIVERY: LazyLock<Mutex<Delivery>> = LazyLock::new(|| Mutex::new(Delivery::default()));

pub(in crate::runners) fn used(app: &AppHandle, usage: &LoginUsage) {
  // The current flags and pending delivery share the acknowledgement's lock order.
  let mut delivery = DELIVERY.lock().unwrap();
  let config = app.state::<SharedConfig>();
  let snapshot = config.load();
  let auth = &snapshot.auth;
  if super::global_consent(auth) != Some(true) {
    return;
  }
  let notice = delivery.used(auth.browser_login_notice_shown, usage);
  drop(delivery);
  if let Some(notice) = notice {
    crate::events::emit(app, crate::events::MEDIA_LOGIN_NOTICE, notice);
  }
}

/// Query replays a notice missed during listener setup; a token acknowledges its first view.
#[tauri::command]
pub(crate) fn media_login_notice(
  app: AppHandle,
  seen_token: Option<String>,
) -> Result<Option<LoginNotice>, String> {
  let mut delivery = DELIVERY.lock().unwrap();
  let config = app.state::<SharedConfig>();
  if config.load().auth.browser_login_notice_shown {
    delivery.pending = None;
    return Ok(None);
  }
  if let Some(token) = seen_token {
    if !delivery
      .pending
      .as_ref()
      .is_some_and(|notice| notice.token == token)
    {
      return Err("Login notice expired".into());
    }
    config
      .modify(&app, |cfg| {
        delivery.confirm(&mut cfg.auth, &token);
      })
      .map_err(|_| "Could not save login notice state".to_owned())?;
    delivery.pending = None;
    Ok(None)
  } else {
    Ok(delivery.pending.clone())
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  #[test]
  fn browser_notice_is_replayed_until_viewed_then_persistently_suppressed() {
    let mut auth = AuthSettings::default();
    let mut delivery = Delivery::default();
    let usage = LoginUsage {
      browser: "chrome",
      host: "facebook.com".into(),
    };
    assert!(!auth.browser_login_notice_shown);
    assert!(delivery.pending.is_none());
    let first = delivery
      .used(auth.browser_login_notice_shown, &usage)
      .unwrap();
    assert!(!auth.browser_login_notice_shown, "use is not proof of view");
    let payload = serde_json::to_value(&first).unwrap();
    let mut keys: Vec<_> = payload
      .as_object()
      .unwrap()
      .keys()
      .map(String::as_str)
      .collect();
    keys.sort();
    assert_eq!(keys, ["browser", "host", "token"]);
    let second = delivery
      .used(
        false,
        &LoginUsage {
          browser: "firefox",
          host: "instagram.com".into(),
        },
      )
      .unwrap();
    assert_eq!(first.token, second.token);
    assert_eq!(second.browser, "chrome");
    assert!(!delivery.confirm(&mut auth, "unoffered-token"));
    assert!(!auth.browser_login_notice_shown);
    assert!(delivery.confirm(&mut auth, &first.token));
    let saved: AuthSettings = serde_json::from_value(serde_json::to_value(auth).unwrap()).unwrap();
    assert!(Delivery::default()
      .used(saved.browser_login_notice_shown, &usage)
      .is_none());
  }
}
