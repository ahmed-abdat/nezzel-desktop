//! Download transport: info-shape analysis, aria2 args, 403 parsing, bundled aria2 lookup.

mod aria_args;
mod denial;
mod info_shape;
mod sidecar;

pub use aria_args::transfer_args;
pub use denial::aria_denied;
pub use info_shape::single_direct_http;
pub use sidecar::*;

#[cfg(test)]
mod tests {
  use super::*;
  #[test]
  fn aria_denial_restarts_with_native_args_and_retries_only_once() {
    use crate::runners::retry_policy::{RetryAction, RetryFailure, RetryState};
    let mut state = RetryState::default();
    let denied = aria_denied("ERROR: HTTP Error 403: Forbidden");
    assert_eq!(
      state.next(
        false,
        false,
        false,
        RetryFailure {
          aria_denied: denied,
          ..Default::default()
        }
      ),
      RetryAction::Native
    );
    assert_eq!(
      transfer_args(4, None, state.native_fallback),
      ["--concurrent-fragments", "4", "--no-continue"]
    );
    assert_eq!(
      state.next(
        false,
        false,
        false,
        RetryFailure {
          aria_denied: denied,
          ..Default::default()
        }
      ),
      RetryAction::Stop
    );
  }
}
