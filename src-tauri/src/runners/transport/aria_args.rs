use std::path::Path;

pub fn transfer_args(fragments: u8, aria: Option<&Path>, restart: bool) -> Vec<String> {
  let mut args = vec![
    "--concurrent-fragments".into(),
    fragments.clamp(1, 16).to_string(),
  ];
  if restart {
    args.push("--no-continue".into());
  }
  if let Some(path) = aria {
    args.extend([
      "--downloader".into(),
      // yt-dlp's "http" key covers HTTPS too; "http,https:" is not a valid scope.
      format!("http:{}", path.to_string_lossy()),
      "--downloader-args".into(),
      "aria2c:-x8 -s8 -k1M".into(),
    ]);
  }
  args
}

#[cfg(test)]
mod tests {
  use super::*;
  #[test]
  fn args_bound_fragments_and_scope_aria2_to_http() {
    assert_eq!(
      transfer_args(4, None, false),
      ["--concurrent-fragments", "4"]
    );
    assert_eq!(transfer_args(0, None, false)[1], "1");
    assert_eq!(transfer_args(255, None, false)[1], "16");
    assert_eq!(
      transfer_args(8, Some(Path::new("/app resources/aria2c")), false),
      [
        "--concurrent-fragments",
        "8",
        "--downloader",
        "http:/app resources/aria2c",
        "--downloader-args",
        "aria2c:-x8 -s8 -k1M",
      ]
    );
  }
}
