//! A request has independent, single-use login, anonymous and transport retry budgets.
#[derive(Debug, PartialEq, Eq)]
pub enum RetryAction {
  Stop,
  Refetch,
  AskLogin,
  /// An automatic source/sign-in task owns the parked row; no user consent is needed.
  Pending,
  Browser,
  Native,
  Anonymous,
}
#[derive(Default)]
pub struct RetryFailure {
  pub cancelled: bool,
  pub login: bool,
  pub aria_denied: bool,
  pub used_cookies: bool,
  pub network_down: bool,
}
#[derive(Default, Clone)]
pub struct RetryState {
  pub browser_attempted: bool,
  pub native_fallback: bool,
  pub anonymous_attempted: bool,
  pub refetched: bool,
}
pub(crate) fn youtube_url(url: &str) -> bool {
  reqwest::Url::parse(url).ok().is_some_and(|url| {
    url.host_str().is_some_and(|host| {
      [
        "youtube.com",
        "youtu.be",
        "googlevideo.com",
        "youtube-nocookie.com",
      ]
      .iter()
      .any(|domain| crate::sites::host_is(host, domain))
    })
  })
}

impl RetryState {
  /// Fresh extraction has its own single-use budget; cookie/aria recovery runs first.
  pub(crate) fn refetch(
    &mut self,
    url: &str,
    code: &str,
    message: &str,
    aria: bool,
    cookies: bool,
  ) -> RetryAction {
    let bilibili = reqwest::Url::parse(url).ok().is_some_and(|url| {
      url.host_str().and_then(crate::sites::site_of) == Some(crate::sites::SiteId::Bilibili)
    });
    let eligible = (youtube_url(url) && code == "accessForbidden403" && !aria && !cookies)
      || (bilibili
        && message
          .to_ascii_lowercase()
          .contains("no video formats found"));
    if !eligible || self.refetched {
      return RetryAction::Stop;
    }
    self.refetched = true;
    RetryAction::Refetch
  }

  pub fn next(
    &mut self,
    automatic: bool,
    _consented: bool,
    has_browser: bool,
    failure: RetryFailure,
  ) -> RetryAction {
    // Download transport recovery owns offline waits before this login-only decision.
    if failure.cancelled || failure.network_down {
      return RetryAction::Stop;
    }
    if failure.aria_denied && !self.native_fallback {
      self.native_fallback = true;
      return RetryAction::Native;
    }
    if failure.used_cookies && !failure.login && !self.anonymous_attempted {
      self.anonymous_attempted = true;
      return RetryAction::Anonymous;
    }
    if failure.login && automatic && !self.browser_attempted {
      if has_browser {
        self.browser_attempted = true;
        return RetryAction::Browser;
      }
      return RetryAction::Stop;
    }
    RetryAction::Stop
  }
}
#[cfg(test)]
mod tests {
  use super::*;
  #[test]
  fn source_refetch_is_scoped_and_once() {
    for (url, code, message, aria, cookies, expected) in [
      (
        "https://youtube.com/watch?v=1",
        "accessForbidden403",
        "",
        false,
        false,
        RetryAction::Refetch,
      ),
      (
        "https://rr1.googlevideo.com/video",
        "accessForbidden403",
        "",
        false,
        false,
        RetryAction::Refetch,
      ),
      (
        "https://youtu.be/1",
        "accessForbidden403",
        "",
        false,
        false,
        RetryAction::Refetch,
      ),
      (
        "https://youtube.com.evil.test/1",
        "accessForbidden403",
        "",
        false,
        false,
        RetryAction::Stop,
      ),
      (
        "https://youtube.com/1",
        "accessForbidden403",
        "",
        true,
        false,
        RetryAction::Stop,
      ),
      (
        "https://youtube.com/1",
        "accessForbidden403",
        "",
        false,
        true,
        RetryAction::Stop,
      ),
      (
        "https://youtube.com/1",
        "mediaUnavailable",
        "",
        false,
        false,
        RetryAction::Stop,
      ),
      (
        "https://bilibili.com/video/1",
        "mediaUnavailable",
        "No video formats found!",
        false,
        false,
        RetryAction::Refetch,
      ),
      (
        "https://b23.tv/1",
        "mediaUnavailable",
        "No video formats found!",
        false,
        true,
        RetryAction::Refetch,
      ),
      (
        "https://bilibili.com/video/1",
        "mediaUnavailable",
        "Not found",
        false,
        false,
        RetryAction::Stop,
      ),
      (
        "https://example.com/1",
        "mediaUnavailable",
        "No video formats found!",
        false,
        false,
        RetryAction::Stop,
      ),
    ] {
      let mut state = RetryState::default();
      assert_eq!(
        state.refetch(url, code, message, aria, cookies),
        expected,
        "{url}"
      );
      assert_eq!(
        state.refetch(url, code, message, aria, cookies),
        RetryAction::Stop
      );
    }
  }

  #[test]
  fn cookie_failures_have_one_anonymous_retry_independent_of_login_and_transport_budgets() {
    let mut state = RetryState {
      browser_attempted: true,
      native_fallback: true,
      ..Default::default()
    };
    let failure = || RetryFailure {
      used_cookies: true,
      ..Default::default()
    };
    assert_eq!(
      state.next(false, false, false, failure()),
      RetryAction::Anonymous
    );
    assert_eq!(
      state.next(false, false, false, failure()),
      RetryAction::Stop
    );
  }

  #[test]
  fn anonymous_retry_eligibility_respects_the_failed_attempt() {
    for (used_cookies, login, cancelled, network_down, expected) in [
      (true, false, false, false, RetryAction::Anonymous),
      (true, true, false, false, RetryAction::Browser),
      (false, false, false, false, RetryAction::Stop),
      (true, false, true, false, RetryAction::Stop),
      (true, false, false, true, RetryAction::Stop),
    ] {
      assert_eq!(
        RetryState::default().next(
          true,
          true,
          true,
          RetryFailure {
            used_cookies,
            login,
            cancelled,
            network_down,
            ..Default::default()
          }
        ),
        expected
      );
    }
  }

  #[test]
  fn first_login_refusal_retries_without_consent() {
    let mut state = RetryState::default();
    assert_eq!(
      state.next(
        true,
        false,
        true,
        RetryFailure {
          cancelled: false,
          login: true,
          aria_denied: false,
          ..Default::default()
        }
      ),
      RetryAction::Browser
    );
    assert!(state.browser_attempted);
  }
  #[test]
  fn browser_retry_is_once_and_only_for_automatic_mode() {
    let mut state = RetryState::default();
    assert_eq!(
      state.next(
        true,
        true,
        true,
        RetryFailure {
          cancelled: false,
          login: true,
          aria_denied: false,
          ..Default::default()
        }
      ),
      RetryAction::Browser
    );
    assert_eq!(
      state.next(
        true,
        true,
        true,
        RetryFailure {
          cancelled: false,
          login: true,
          aria_denied: false,
          ..Default::default()
        }
      ),
      RetryAction::Stop
    );
    assert_eq!(
      RetryState::default().next(
        false,
        true,
        true,
        RetryFailure {
          cancelled: false,
          login: true,
          aria_denied: false,
          ..Default::default()
        }
      ),
      RetryAction::Stop
    );
  }
  #[test]
  fn aria_denial_falls_back_before_cookie_access_and_only_once() {
    let mut state = RetryState::default();
    assert_eq!(
      state.next(
        true,
        true,
        true,
        RetryFailure {
          cancelled: false,
          login: true,
          aria_denied: true,
          ..Default::default()
        }
      ),
      RetryAction::Native
    );
    assert_eq!(
      state.next(
        true,
        true,
        true,
        RetryFailure {
          cancelled: false,
          login: false,
          aria_denied: true,
          ..Default::default()
        }
      ),
      RetryAction::Stop
    );
    assert!(!state.browser_attempted);
  }
  #[test]
  fn cancel_and_unrelated_errors_never_retry() {
    let mut state = RetryState::default();
    assert_eq!(
      state.next(
        true,
        true,
        true,
        RetryFailure {
          cancelled: true,
          login: true,
          aria_denied: true,
          ..Default::default()
        }
      ),
      RetryAction::Stop
    );
    assert_eq!(
      state.next(
        true,
        true,
        true,
        RetryFailure {
          cancelled: false,
          login: false,
          aria_denied: false,
          ..Default::default()
        }
      ),
      RetryAction::Stop
    );
    assert_eq!(
      state.next(
        true,
        true,
        false,
        RetryFailure {
          cancelled: false,
          login: true,
          aria_denied: false,
          ..Default::default()
        }
      ),
      RetryAction::Stop
    );
  }
}
