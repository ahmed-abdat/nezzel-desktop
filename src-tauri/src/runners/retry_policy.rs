//! A request has independent, single-use login, anonymous and transport retry budgets.
#[derive(Debug, PartialEq, Eq)]
pub enum RetryAction {
  Stop,
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
}
impl RetryState {
  pub fn next(
    &mut self,
    automatic: bool,
    _consented: bool,
    has_browser: bool,
    failure: RetryFailure,
  ) -> RetryAction {
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
