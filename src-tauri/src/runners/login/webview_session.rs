//! App-owned, per-site persistent sessions. External pages receive no capabilities.
use super::automatic::enabled;
use crate::runners::browser_cookies::{proof_cookies, site_key, CookieAccess, CookieContext};
use std::{
  sync::LazyLock,
  time::{Duration, Instant},
};
use tauri::{
  webview::NewWindowResponse, AppHandle, Manager, Url, WebviewUrl, WebviewWindowBuilder,
};

static WINDOWS: LazyLock<[tokio::sync::Mutex<()>; 3]> =
  LazyLock::new(|| std::array::from_fn(|_| tokio::sync::Mutex::new(())));
const COOKIE_TIMEOUT: Duration = Duration::from_secs(5);
const INSECURE_BROWSER: &str = "This browser or app may not be secure";
fn site(host: &str) -> Option<(usize, &'static str, &'static str)> {
  match site_key(host)?.as_str() {
    "facebook.com" => Some((0, "facebook.com", "https://www.facebook.com/login/")),
    "instagram.com" => Some((
      1,
      "instagram.com",
      "https://www.instagram.com/accounts/login/",
    )),
    "youtube.com" | "google.com" => {
      Some((2, "youtube.com", "https://accounts.google.com/ServiceLogin"))
    }
    _ => None,
  }
}
pub(crate) fn allowed_url(host: &str, url: &Url) -> bool {
  if url.scheme() != "https"
    || !url.username().is_empty()
    || url.password().is_some()
    || url.port_or_known_default() != Some(443)
  {
    return false;
  }
  let Some((index, domain, _)) = site(host) else {
    return false;
  };
  let Some(target) = url.host_str() else {
    return false;
  };
  [domain, if index == 2 { "google.com" } else { domain }]
    .iter()
    .any(|domain| target == *domain || target.ends_with(&format!(".{domain}")))
}
pub(crate) fn rejected_page(host: &str, url: &Url, text: &str) -> bool {
  site(host).is_some_and(|(index, _, _)| index == 2)
    && (url.path().contains("/signin/rejected")
      || text
        .to_ascii_lowercase()
        .contains(&INSECURE_BROWSER.to_ascii_lowercase()))
}
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum CloseDecision {
  Wait,
  Retry,
  Remedy,
  Cancelled,
}
pub(crate) fn close_decision(
  proof: bool,
  cancelled: bool,
  consent: bool,
  rejected: bool,
  expired: bool,
) -> CloseDecision {
  if cancelled || !consent {
    CloseDecision::Cancelled
  } else if rejected || expired {
    CloseDecision::Remedy
  } else if proof {
    CloseDecision::Retry
  } else {
    CloseDecision::Wait
  }
}
/// Compare only proof cookies: changing a tracking cookie cannot authenticate a stale session.
fn proof_signature(
  host: &str,
  cookies: &[tauri::webview::Cookie<'static>],
) -> Vec<(String, String, String)> {
  let mut signature: Vec<_> = cookies
    .iter()
    .filter(|c| {
      crate::runners::browser_cookies::is_session_cookie(host, c.name())
        && crate::runners::browser_cookies::cookie_host_allowed(c.domain().unwrap_or(""), host)
    })
    .map(|c| {
      (
        c.domain().unwrap_or("").into(),
        c.name().into(),
        c.value().into(),
      )
    })
    .collect();
  signature.sort();
  signature.dedup();
  signature
}
fn fresh_proof(
  interactive: bool,
  before: &[(String, String, String)],
  after: &[(String, String, String)],
) -> bool {
  !interactive || before != after
}
struct CloseOnDrop<F: FnOnce()>(Option<F>);
impl<F: FnOnce()> Drop for CloseOnDrop<F> {
  fn drop(&mut self) {
    if let Some(close) = self.0.take() {
      close();
    }
  }
}

pub(in crate::runners) async fn read(
  app: &AppHandle,
  host: &str,
  interactive: bool,
) -> Result<Option<CookieAccess>, &'static str> {
  let Some((index, domain, login)) = site(host) else {
    return if interactive {
      Err("browserLoginRemedy")
    } else {
      Ok(None)
    };
  };
  let root = app
    .path()
    .app_data_dir()
    .map_err(|_| "browserCookiesUnreadable")?
    .join("login-sessions")
    .join(domain);
  let marker = root.join("session-present");
  if !interactive && !marker.is_file() {
    return Ok(None);
  }
  let _site = WINDOWS[index].lock().await;
  if !enabled(app) {
    return Err("browserLoginDisabled");
  }
  // Restrict the directory even on platforms that keep the native store elsewhere (WKWebView).
  std::fs::create_dir_all(root.parent().ok_or("browserCookiesUnreadable")?)
    .map_err(|_| "browserCookiesUnreadable")?;
  crate::runners::browser_cookies::session_directory(&root)?;
  let (tx, rx) = tokio::sync::oneshot::channel();
  let handle = app.clone();
  app
    .run_on_main_thread(move || {
      let build = || {
        if !enabled(&handle) {
          return Err("browserLoginDisabled");
        }
        let builder = WebviewWindowBuilder::new(
          &handle,
          format!("site-login-{index}"),
          WebviewUrl::External(login.parse().unwrap()),
        )
        .title("Nezzel")
        .inner_size(900.0, 750.0)
        .visible(false)
        .user_agent(crate::net::CHROME_UA)
        .on_navigation(move |url| allowed_url(domain, url))
        .on_new_window(|_, _| NewWindowResponse::Deny);
        #[cfg(target_os = "macos")]
        let builder = builder.data_store_identifier([0x40 + index as u8; 16]);
        #[cfg(not(target_os = "macos"))]
        let builder = builder.data_directory(root);
        builder.build().map_err(|_| "browserCookiesUnreadable")
      };
      if let Err(Ok(window)) = tx.send(build()) {
        let _ = window.close();
      }
    })
    .map_err(|_| "browserCookiesUnreadable")?;
  let window = tokio::time::timeout(COOKIE_TIMEOUT, rx)
    .await
    .map_err(|_| "browserLoginTimeout")?
    .map_err(|_| "browserCookiesUnreadable")??;
  let close = window.clone();
  let _close = CloseOnDrop(Some(move || {
    let _ = close.destroy();
  }));
  let context = CookieContext::for_app(app)?;
  let before = if interactive {
    let reset_window = window.clone();
    tokio::time::timeout(
      COOKIE_TIMEOUT,
      tauri::async_runtime::spawn_blocking(move || {
        // WKWebView cookies_for_url currently compares domains literally and misses
        // .facebook.com and host-only login-subdomain cookies. This store is app-owned
        // and site-isolated; read it off-thread, then scope before proof/export/delete.
        let cookies = reset_window
          .cookies()
          .map_err(|_| "browserCookiesUnreadable")?;
        let before = proof_signature(domain, &cookies);
        // The Cookie API normalizes Domain's leading dot, which can also break
        // native delete_cookie matching. Reset this site's dedicated app-owned
        // store, including its local sign-in state, without touching browser data.
        reset_window
          .clear_all_browsing_data()
          .map_err(|_| "browserCookiesUnreadable")?;
        Ok::<_, &'static str>(before)
      }),
    )
    .await
    .map_err(|_| "browserLoginTimeout")?
    .map_err(|_| "browserCookiesUnreadable")??
  } else {
    vec![]
  };
  if interactive {
    // WKWebView clears asynchronously. Wait for old proof to disappear before
    // loading the form, rather than navigating with the rejected session again.
    tokio::time::timeout(COOKIE_TIMEOUT, async {
      loop {
        let probe = window.clone();
        let cookies = tauri::async_runtime::spawn_blocking(move || {
          probe.cookies().map_err(|_| "browserCookiesUnreadable")
        })
        .await
        .map_err(|_| "browserCookiesUnreadable")??;
        if proof_signature(domain, &cookies).is_empty() {
          break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
      }
      Ok::<_, &'static str>(())
    })
    .await
    .map_err(|_| "browserLoginTimeout")??;
    if !enabled(app) {
      return Err("browserLoginDisabled");
    }
    window
      .navigate(login.parse().unwrap())
      .map_err(|_| "browserCookiesUnreadable")?;
    window.show().map_err(|_| "browserCookiesUnreadable")?;
    let _ = window.set_focus();
  }
  let start = Instant::now();
  loop {
    if !enabled(app) {
      return Err("browserLoginDisabled");
    }
    if app.get_webview_window(window.label()).is_none() {
      return Err("cancelled");
    }
    let read_window = window.clone();
    let host_owned = domain.to_owned();
    let result = tokio::time::timeout(
      COOKIE_TIMEOUT,
      tauri::async_runtime::spawn_blocking(move || {
        let current = read_window.url().map_err(|_| "browserCookiesUnreadable")?;
        let mut cookies = read_window
          .cookies()
          .map_err(|_| "browserCookiesUnreadable")?;
        cookies.retain(|cookie| {
          crate::runners::browser_cookies::cookie_host_allowed(
            cookie.domain().unwrap_or(""),
            &host_owned,
          )
        });
        Ok::<_, &'static str>((current, cookies))
      }),
    )
    .await
    .map_err(|_| "browserLoginTimeout")?
    .map_err(|_| "browserCookiesUnreadable")??;
    let (current, cookies) = result;
    let mut rejected = rejected_page(domain, &current, "");
    if index == 2 && !rejected {
      let (tx, rx) = tokio::sync::oneshot::channel();
      let tx = std::sync::Mutex::new(Some(tx));
      window.eval_with_callback("Boolean(document.body && document.body.innerText.toLowerCase().includes('this browser or app may not be secure'))", move |value| { if let Some(tx) = tx.lock().unwrap().take() { let _ = tx.send(value); } }).map_err(|_| "browserCookiesUnreadable")?;
      let value = tokio::time::timeout(COOKIE_TIMEOUT, rx)
        .await
        .map_err(|_| "browserLoginTimeout")?
        .map_err(|_| "browserCookiesUnreadable")?;
      if value == "true" {
        rejected = rejected_page(domain, &current, INSECURE_BROWSER);
      }
    }
    let now = std::time::SystemTime::now()
      .duration_since(std::time::UNIX_EPOCH)
      .map_or(0, |t| t.as_secs() as i64);
    let proof = proof_cookies(
      domain,
      cookies.iter().map(|c| {
        (
          c.domain().unwrap_or(""),
          c.name(),
          c.value(),
          c.expires_datetime().map_or(0, |t| t.unix_timestamp()),
        )
      }),
      now,
    );
    let proof = proof && fresh_proof(interactive, &before, &proof_signature(domain, &cookies));
    match close_decision(
      proof,
      false,
      enabled(app),
      rejected,
      start.elapsed() >= super::LOGIN_TTL,
    ) {
      CloseDecision::Cancelled => return Err("cancelled"),
      CloseDecision::Remedy => return Err("browserLoginRemedy"),
      CloseDecision::Retry => {
        let access = CookieAccess::from_webview(&context, domain, &cookies, crate::net::CHROME_UA)?;
        // The marker contains no session data. The native per-site store is authoritative.
        if !marker.exists() {
          crate::runners::browser_cookies::session_marker(&marker)?;
        }
        return Ok(Some(access));
      }
      CloseDecision::Wait if !interactive => return Ok(None),
      CloseDecision::Wait => tokio::time::sleep(Duration::from_millis(500)).await,
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  #[tokio::test]
  async fn dropping_a_parked_login_future_closes_its_window_once() {
    let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let closed = count.clone();
    let (ready, wait) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
      let _guard = CloseOnDrop(Some(move || {
        closed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
      }));
      let _ = ready.send(());
      std::future::pending::<()>().await;
    });
    wait.await.unwrap();
    task.abort();
    let _ = task.await;
    assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 1);
  }
  #[test]
  fn unchanged_stale_proof_does_not_close_the_recovery_window() {
    let cookie = |name, value| {
      tauri::webview::Cookie::build((name, value))
        .domain("facebook.com")
        .build()
    };
    let stale = proof_signature(
      "facebook.com",
      &[cookie("c_user", "account"), cookie("xs", "old-session")],
    );
    let tracking_changed = proof_signature(
      "facebook.com",
      &[
        cookie("c_user", "account"),
        cookie("xs", "old-session"),
        cookie("tracking", "new"),
      ],
    );
    let refreshed = proof_signature(
      "facebook.com",
      &[cookie("c_user", "account"), cookie("xs", "new-session")],
    );
    let subdomain = tauri::webview::Cookie::build(("xs", "unchanged"))
      .domain("web.facebook.com")
      .build();
    let before = proof_signature("facebook.com", std::slice::from_ref(&subdomain));
    assert!(!before.is_empty());
    assert!(!fresh_proof(
      true,
      &before,
      &proof_signature("facebook.com", &[subdomain])
    ));
    assert!(!fresh_proof(true, &stale, &tracking_changed));
    assert!(fresh_proof(true, &stale, &refreshed));
    assert!(fresh_proof(false, &stale, &stale));
    assert_eq!(
      close_decision(
        fresh_proof(true, &stale, &tracking_changed),
        false,
        true,
        false,
        false
      ),
      CloseDecision::Wait
    );
  }
  #[test]
  fn sign_in_allow_list_rejects_cross_site_and_untrusted_schemes() {
    for (site, url, allowed) in [
      ("facebook.com", "https://web.facebook.com/login/", true),
      ("facebook.com", "https://m.facebook.com/", true),
      (
        "instagram.com",
        "https://instagram.com/accounts/login/",
        true,
      ),
      (
        "youtube.com",
        "https://accounts.google.com/v3/signin/identifier",
        true,
      ),
      ("youtube.com", "https://www.youtube.com/", true),
      ("facebook.com", "https://facebook.com.evil.example/", false),
      ("facebook.com", "https://evilfacebook.com/", false),
      ("facebook.com", "https://instagram.com/", false),
      ("facebook.com", "http://facebook.com/", false),
      ("facebook.com", "https://facebook.com:444/", false),
      ("facebook.com", "https://user:pass@facebook.com/", false),
      ("facebook.com", "data:text/html,hi", false),
      ("facebook.com", "javascript:alert(1)", false),
    ] {
      assert_eq!(allowed_url(site, &url.parse().unwrap()), allowed, "{url}");
    }
  }
  #[test]
  fn google_rejection_and_close_decisions_precede_stale_proof() {
    for (path, text, expected) in [
      ("/v3/signin/identifier", "", false),
      ("/signin/rejected", "", true),
      (
        "/v3/signin/identifier",
        "This browser or app may not be secure",
        true,
      ),
    ] {
      assert_eq!(
        rejected_page(
          "youtube.com",
          &format!("https://accounts.google.com{path}")
            .parse()
            .unwrap(),
          text
        ),
        expected
      );
    }
    for (proof, cancel, consent, reject, expiry, expected) in [
      (false, false, true, false, false, CloseDecision::Wait),
      (true, false, true, false, false, CloseDecision::Retry),
      (true, false, true, true, false, CloseDecision::Remedy),
      (true, true, true, false, false, CloseDecision::Cancelled),
      (true, false, false, false, false, CloseDecision::Cancelled),
      (true, false, true, false, true, CloseDecision::Remedy),
    ] {
      assert_eq!(
        close_decision(proof, cancel, consent, reject, expiry),
        expected
      );
    }
  }
}
