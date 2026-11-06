pub fn aria_denied(line: &str) -> bool {
  let line = line.to_ascii_lowercase();
  if line.contains("http error 403") {
    return true;
  }
  let error = line.trim_start().starts_with("error:")
    || line.contains("[error]")
    || line.contains("errorcode=");
  error
    && (line.contains("authorization failed")
      || line
        .split(|c: char| !c.is_ascii_digit())
        .any(|token| token == "403"))
}

#[cfg(test)]
mod tests {
  use super::*;
  #[test]
  fn aria_denials_do_not_match_progress_or_non_error_status_text() {
    for line in [
      "RAW|403|403.21MiB",
      "[download] 403 bytes",
      "HTTP status 403",
      "ERROR: HTTP Error 404",
      "Authorization failed",
    ] {
      assert!(!aria_denied(line), "{line}");
    }
    assert!(aria_denied("ERROR: HTTP Error 403: Forbidden"));
    assert!(aria_denied("[ERROR] errorCode=24 Authorization failed"));
  }
}
