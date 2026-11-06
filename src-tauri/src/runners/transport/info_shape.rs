use serde_json::Value;

pub fn single_direct_http(info: &Value, source: &str) -> bool {
  let host = reqwest::Url::parse(source)
    .ok()
    .and_then(|u| u.host_str().map(str::to_owned))
    .unwrap_or_default();
  if host == "youtu.be"
    || host == "youtube.com"
    || host.ends_with(".youtube.com")
    || host.ends_with(".youtube-nocookie.com")
    || info["extractor_key"]
      .as_str()
      .is_some_and(|s| s.to_ascii_lowercase().contains("youtube"))
  {
    return false;
  }
  if info["_type"].as_str().is_some_and(|s| s != "video")
    || info.get("entries").is_some()
    || info.get("requested_formats").is_some()
    || info.get("fragments").is_some()
  {
    return false;
  }
  if let Some(downloads) = info["requested_downloads"].as_array() {
    return downloads.len() == 1 && single_direct_http(&downloads[0], source);
  }
  matches!(info["protocol"].as_str(), Some("http" | "https"))
    && info["url"].as_str().is_some_and(|s| {
      reqwest::Url::parse(s)
        .ok()
        .is_some_and(|u| matches!(u.scheme(), "http" | "https"))
    })
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;
  #[test]
  fn only_proven_single_http_format_uses_aria2() {
    let direct = json!({"protocol":"https", "url":"https://cdn.example/a.mp4"});
    assert!(single_direct_http(&direct, "https://example.com/a"));
    for info in [
      json!({"protocol":"m3u8_native","url":"https://cdn.example/a.m3u8"}),
      json!({"protocol":"http_dash_segments","fragments":[{}],"url":"https://cdn.example/a"}),
      json!({"requested_formats":[direct.clone(),direct.clone()]}),
      json!({"requested_downloads":[direct.clone(),direct.clone()]}),
      json!({"_type":"playlist","entries":[direct.clone()]}),
      json!({}),
    ] {
      assert!(!single_direct_http(&info, "https://example.com/a"));
    }
    assert!(!single_direct_http(
      &direct,
      "https://www.youtube.com/watch?v=a"
    ));
    assert!(!single_direct_http(&direct, "https://youtu.be/a"));
    assert!(!single_direct_http(
      &json!({"extractor_key":"Youtube", "protocol":"https", "url":"https://cdn.example/a"}),
      "https://example.com/a"
    ));
  }
}
