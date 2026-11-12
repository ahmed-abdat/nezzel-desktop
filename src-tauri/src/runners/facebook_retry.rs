//! One anonymous Facebook retry before requesting a session.
use crate::{resolve::ResolvedInput, runners::ytdlp_runner::YtdlpRunner};
use tauri::AppHandle;

pub(crate) fn chrome_target_available(output: &str) -> bool {
  output.lines().any(|line| {
    let lower = line.to_ascii_lowercase();
    !lower.contains("not available")
      && !lower.contains("unavailable")
      && lower
        .split_whitespace()
        .next()
        .is_some_and(|target| target == "chrome" || target.starts_with("chrome-"))
  })
}
pub(crate) async fn retry(
  app: &AppHandle,
  group: &str,
  input: &mut ResolvedInput,
  used_cookies: bool,
  reason: Option<&str>,
  stderr: &str,
) -> bool {
  if !input.can_retry_facebook(used_cookies)
    || (reason.is_none() && !input.can_retry_facebook_share(stderr))
  {
    return false;
  }
  let supported = YtdlpRunner::new(app)
    .with_args(["--list-impersonate-targets"])
    .output_with_timeout(group, std::time::Duration::from_secs(5))
    .await
    .ok()
    .flatten()
    .is_some_and(|output| {
      output.code == 0 && chrome_target_available(&String::from_utf8_lossy(&output.stdout))
    });
  input.retry_facebook(used_cookies, supported) || input.retry_facebook_share(stderr).await
}
#[cfg(test)]
mod tests {
  use super::*;
  #[test]
  fn available_chrome_targets_and_one_anonymous_retry() {
    for (output, expected) in [
      (
        "Target Client OS Source\nchrome-136 Chrome macOS curl_cffi",
        true,
      ),
      ("chrome-136 Chrome macOS curl_cffi (not available)", false),
      ("safari-17 Safari macOS curl_cffi", false),
    ] {
      assert_eq!(chrome_target_available(output), expected);
    }
    for (url, cookies, support, expected) in [
      ("https://facebook.com/watch?v=1", false, true, true),
      ("https://web.facebook.com/watch?v=1", true, true, false),
      ("https://facebook.com/watch?v=1", false, false, false),
      ("https://facebook.com.evil.example/", false, true, false),
    ] {
      let mut input = ResolvedInput::new(url);
      assert_eq!(input.retry_facebook(cookies, support), expected);
      if expected {
        assert!(!input.retry_facebook(false, true));
        assert!(input
          .args()
          .windows(2)
          .any(|args| args == ["--impersonate", "chrome"]));
      }
    }
  }

  #[tokio::test(start_paused = true)]
  async fn share_backoff_and_impersonation_share_one_budget_and_keep_the_target() {
    let stderr = "ERROR: [facebook] abc: Cannot parse data; please report this issue";
    for impersonate in [false, true] {
      let mut input = ResolvedInput::new("https://www.facebook.com/share/v/abc/");
      input.rewrite("https://cdn.example/selected.mp4".into());
      let referer = input.source_page.clone();
      assert!(!input.can_retry_facebook(true));
      if impersonate {
        assert!(input.retry_facebook(false, true));
        assert!(!input.retry_facebook_share(stderr).await);
      } else {
        assert!(!input.retry_facebook(false, false));
        assert!(input.retry_facebook_share(stderr).await);
        assert!(!input.retry_facebook(false, true));
      }
      assert!(!input.can_retry_facebook(false));
      assert_eq!(input.url, "https://cdn.example/selected.mp4");
      assert_eq!(input.source_page, referer);
      assert_eq!(input.args().contains(&"--impersonate"), impersonate);
    }
  }
}
