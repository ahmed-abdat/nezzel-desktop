use std::collections::HashSet;

#[derive(Clone, Debug)]
pub struct Link {
  pub site: String,
  pub label: String,
  pub expected: String,
  pub url: String,
  pub note: String,
  pub count: Option<usize>,
  pub count_is_total: bool,
}

pub fn parse_links(text: &str, seen: &mut HashSet<String>) -> Result<Vec<Link>, String> {
  let mut rows = text
    .lines()
    .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'));
  let Some(first) = rows.next() else {
    return Ok(Vec::new());
  };
  let first: Vec<_> = first.trim_end_matches('\r').split('\t').collect();
  let header = first.contains(&"url");
  let columns = if header {
    ["site", "label", "expected_type", "url"]
      .iter()
      .map(|name| {
        first
          .iter()
          .position(|v| v == name)
          .ok_or_else(|| format!("missing column {name}"))
      })
      .collect::<Result<Vec<_>, _>>()?
  } else {
    vec![0, 1, 2, 3]
  };
  let note_col = if header {
    first.iter().position(|v| *v == "note" || *v == "notes")
  } else {
    None
  };
  let mut data = Vec::new();
  if !header {
    data.push(first.join("\t"));
  }
  data.extend(rows.map(str::to_owned));
  let mut result = Vec::new();
  for row in data {
    let fields: Vec<_> = row.trim_end_matches('\r').split('\t').collect();
    let field = |i: usize| {
      fields
        .get(i)
        .map(|v| v.trim().to_string())
        .ok_or_else(|| format!("short matrix row: {row}"))
    };
    let url = field(columns[3])?;
    // Blank URLs are documented coverage gaps, never silently claimed as tested.
    if !url.is_empty() && !seen.insert(url.clone()) {
      continue;
    }
    let note = note_col
      .and_then(|i| fields.get(i))
      .unwrap_or(&"")
      .to_string();
    let words: Vec<_> = note
      .split(|c: char| !c.is_alphanumeric())
      .filter(|v| !v.is_empty())
      .collect();
    let note_count = words.windows(2).enumerate().find_map(|(i, pair)| {
      let n = pair[0].parse::<usize>().ok()?;
      let kind = if pair[1] == "feedshare" {
        *words.get(i + 2)?
      } else {
        pair[1]
      };
      (n > 0
        && matches!(
          kind,
          "photo" | "photos" | "image" | "images" | "slide" | "slides"
        ))
      .then_some((n, matches!(kind, "slide" | "slides")))
    });
    let label = field(columns[1])?;
    let expected = field(columns[2])?;
    // Only unambiguous plural/multi-photo labels carry counts. A trailing -2
    // on a singular-photo fixture means the second fixture, not two photos.
    let labels: Vec<_> = label.split('-').collect();
    let label_count = labels
      .windows(2)
      .find_map(|pair| {
        let plural = |word: &str| matches!(word, "photos" | "images");
        if plural(pair[0]) {
          pair[1].parse().ok()
        } else if plural(pair[1]) {
          pair[0].parse().ok()
        } else {
          None
        }
      })
      .or_else(|| {
        labels.windows(3).find_map(|p| {
          (p[0] == "multi" && p[1] == "photo")
            .then(|| p[2].parse().ok())
            .flatten()
        })
      });
    let (count, count_is_total) = note_count
      .map(|(n, slides)| (Some(n), slides && expected == "mixed"))
      .unwrap_or((label_count, false));
    result.push(Link {
      site: field(columns[0])?,
      label,
      expected,
      url,
      note,
      count,
      count_is_total,
    });
  }
  Ok(result)
}

pub fn matches_expected(
  expected: &str,
  count: Option<usize>,
  count_is_total: bool,
  images: usize,
  videos: usize,
  audio: usize,
) -> bool {
  if count.is_some_and(|n| {
    if count_is_total {
      images + videos != n
    } else {
      images != n
    }
  }) {
    return false;
  }
  match expected {
    "photo" | "image" => images == 1 && videos == 0,
    "photos" | "carousel" => images >= 2 && videos == 0,
    "mixed" => images > 0 && videos > 0,
    "video" => videos > 0,
    "audio" => audio > 0 && videos == 0,
    "gif" => images + videos > 0,
    "unknown" => images + videos + audio > 0,
    _ => false,
  }
}

pub fn classify(codes: &[String], error: &str) -> &'static str {
  let lower = error.to_ascii_lowercase();
  if codes.iter().any(|c| {
    matches!(
      c.as_str(),
      "signInRequired" | "signInRequiredForBotDetection" | "membersOnly"
    )
  }) || [
    "sign in",
    "log in",
    "login required",
    "loginrequired",
    "authenticationerror",
    "authorizationerror",
    "cookies are required",
    "redirect to login",
  ]
  .iter()
  .any(|s| lower.contains(s))
  {
    "LOGIN"
  } else if codes.iter().any(|c| c == "notFound404")
    || [
      "http error 404",
      "404 not found",
      "404: not found",
      "status code 404",
      "410 gone",
      "has been removed",
      "has been deleted",
      "notfounderror",
      "no longer available",
    ]
    .iter()
    .any(|s| lower.contains(s))
  {
    "GONE"
  } else {
    "FAIL"
  }
}

// Called only for a failed pipeline, using its terminal app code. Engine prose
// and earlier fallback diagnostics cannot prove an expected refusal.
pub fn matches_error(expected: &str, codes: &[&str], error: &str) -> bool {
  if error.starts_with("Harness per-link deadline:") {
    return false;
  }
  let code = codes.last().copied().unwrap_or_default();
  match expected {
    "gone" => matches!(code, "notFound404" | "geoBlocked"),
    "none" => code == "noMediaFound",
    _ => false,
  }
}

pub fn verify_file(path: &std::path::Path, checksum: Option<&str>) -> Result<(), String> {
  use std::io::Read;
  let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
  if !file.metadata().map_err(|e| e.to_string())?.is_file() {
    return Err("Not a regular output file".into());
  }
  if let Some(expected) = checksum {
    use sha2::{Digest, Sha256};
    if expected.len() != 64 || !expected.bytes().all(|b| b.is_ascii_hexdigit()) {
      return Err("Invalid fixture SHA-256".into());
    }
    let mut hash = Sha256::new();
    let mut buffer = [0; 65536];
    let mut size = 0;
    loop {
      let count = file.read(&mut buffer).map_err(|e| e.to_string())?;
      if count == 0 {
        break;
      }
      size += count;
      hash.update(&buffer[..count]);
    }
    return if size > 0 && hex::encode(hash.finalize()).eq_ignore_ascii_case(expected) {
      Ok(())
    } else {
      Err("Output differs from fixture SHA-256".into())
    };
  }
  let mut bytes = Vec::new();
  file
    .by_ref()
    .take(512)
    .read_to_end(&mut bytes)
    .map_err(|e| e.to_string())?;
  let extension = path
    .extension()
    .and_then(|v| v.to_str())
    .unwrap_or_default()
    .to_ascii_lowercase();
  let matches = match extension.as_str() {
    "pdf" => bytes.starts_with(b"%PDF-"),
    "zip" => [b"PK\x03\x04", b"PK\x05\x06", b"PK\x07\x08"]
      .iter()
      .any(|m| bytes.starts_with(*m)),
    "exe" | "dll" => bytes.starts_with(b"MZ"),
    "msi" => bytes.starts_with(b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1"),
    "7z" => bytes.starts_with(b"7z\xbc\xaf\x27\x1c"),
    "rar" => bytes.starts_with(b"Rar!\x1a\x07\x00") || bytes.starts_with(b"Rar!\x1a\x07\x01\x00"),
    "gz" | "tgz" => bytes.starts_with(b"\x1f\x8b\x08"),
    "bz2" => bytes.starts_with(b"BZh"),
    "xz" => bytes.starts_with(b"\xfd7zXZ\0"),
    "tar" => bytes.get(257..263) == Some(b"ustar\0") || bytes.get(257..265) == Some(b"ustar  \0"),
    _ => false,
  };
  if !matches {
    return Err(format!(
      "Empty, unknown or mismatched file signature: {}",
      path.display()
    ));
  }
  Ok(())
}

pub fn matches_selection(
  expected_images: usize,
  expected_videos: usize,
  images: usize,
  videos: usize,
  audio: usize,
) -> bool {
  images + videos + audio > 0 && images >= expected_images && videos >= expected_videos
}

pub fn last_engine_error(text: &str) -> String {
  text
    .lines()
    .rev()
    .find(|line| {
      let lower = line.to_ascii_lowercase();
      lower.contains("error:")
        || lower.contains(":error]")
        || lower.contains("][error]")
        || lower.contains("exception:")
    })
    .or_else(|| text.lines().rev().find(|line| !line.trim().is_empty()))
    .unwrap_or_default()
    .into()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn matrix_headers_comments_legacy_rows_and_duplicate_annotations() {
    let mut seen = HashSet::new();
    let links = parse_links("# comment\nurl\texpected_type\tlabel\tsite\tnote\nhttps://example.com/1\tphotos\towner\tlinkedin\t4 feedshare images\n", &mut seen).unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].count, Some(4));
    assert_eq!(links[0].site, "linkedin");
    let legacy = parse_links("# no header\nyoutube\tclip\tvideo\thttps://example.com/2\nyoutube\tduplicate\tvideo\thttps://example.com/1\n", &mut seen).unwrap();
    assert_eq!(legacy.len(), 1);
    assert_eq!(legacy[0].url, "https://example.com/2");
    let counts = parse_links("x\tstatus-multi-photo-4\tphotos\thttps://example.com/3\nbluesky\tpost-images-3\tphotos\thttps://example.com/4\nx\tstatus-video-with-2-photos\tmixed\thttps://example.com/5\n", &mut seen).unwrap();
    assert_eq!(
      counts.iter().map(|r| r.count).collect::<Vec<_>>(),
      [Some(4), Some(3), Some(2)]
    );

    assert!(parse_links(
      "site\tlabel\turl\nx\tbroken\thttps://example.com\n",
      &mut seen
    )
    .is_err());
  }

  #[test]
  fn verdict_requires_files_of_every_expected_kind_and_explicit_count() {
    for (expected, count, images, videos, audio, want) in [
      ("video", None, 1, 0, 0, false),
      ("video", None, 0, 1, 1, true),
      ("audio", None, 0, 0, 1, true),
      ("photo", None, 1, 0, 0, true),
      ("photos", Some(4), 3, 0, 0, false),
      ("photos", Some(4), 4, 0, 0, true),
      ("mixed", None, 1, 0, 0, false),
      ("mixed", None, 1, 1, 1, true),
      ("unknown", None, 0, 0, 0, false),
    ] {
      assert_eq!(
        matches_expected(expected, count, expected == "mixed", images, videos, audio),
        want,
        "{expected}"
      );
    }
  }

  #[test]
  fn expected_absence_requires_the_terminal_honest_code() {
    for (expected, code, error, want) in [
      ("gone", "notFound404", "fetch failed", true),
      ("gone", "geoBlocked", "fetch failed", true),
      ("none", "noMediaFound", "fetch failed", true),
      ("gone", "videoNotFoundOrPrivate", "fetch failed", false),
      ("gone", "mediaUnavailable", "fetch failed", false),
      ("gone", "noMediaFound", "fetch failed", false),
      ("none", "notFound404", "fetch failed", false),
      ("none", "geoBlocked", "fetch failed", false),
      ("none", "unknown", "No media found", false),
      ("gone", "", "HTTP Error 404: Not Found", false),
      ("none", "", "fetch cancelled/no metadata", false),
      (
        "none",
        "noMediaFound",
        "Harness per-link deadline: 180 seconds",
        false,
      ),
      (
        "gone",
        "geoBlocked",
        "Harness per-link deadline: 180 seconds",
        false,
      ),
      ("video", "notFound404", "fetch failed", false),
    ] {
      assert_eq!(
        matches_error(expected, &[code], error),
        want,
        "{expected}/{code}/{error}"
      );
    }
    for codes in [
      vec!["noMediaFound", "unknown"],
      vec!["noMediaFound", ""],
      vec![],
    ] {
      assert!(!matches_error("none", &codes, "download failed"));
    }
  }

  #[test]
  fn generic_files_require_nonempty_matching_magic_not_a_media_stream() {
    let root = std::env::temp_dir().join(format!(
      "matrix-magic-{}-{}",
      std::process::id(),
      std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
    ));
    std::fs::create_dir(&root).unwrap();
    let mut tar = vec![0; 512];
    tar[257..263].copy_from_slice(b"ustar\0");
    for (index, (extension, bytes, want)) in [
      ("pdf", b"%PDF-1.7\n".as_slice(), true),
      ("PDF", b"%PDF-1.7\n", true),
      ("zip", b"PK\x03\x04payload", true),
      ("zip", b"PK\x05\x06empty archive", true),
      ("exe", b"MZpayload", true),
      ("msi", b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1", true),
      ("7z", b"7z\xbc\xaf\x27\x1c", true),
      ("rar", b"Rar!\x1a\x07\x01\x00", true),
      ("gz", b"\x1f\x8b\x08payload", true),
      ("bz2", b"BZh9payload", true),
      ("xz", b"\xfd7zXZ\0payload", true),
      ("tar", tar.as_slice(), true),
      ("pdf", b"", false),
      ("pdf", b"<html>not a PDF</html>", false),
      ("pdf", b"PK\x03\x04wrong kind", false),
      ("zip", b"PK\x03", false),
      ("tar", b"ustar\0", false),
      ("bin", b"unknown signature", false),
    ]
    .into_iter()
    .enumerate()
    {
      let path = root.join(format!("{index}.{extension}"));
      std::fs::write(&path, bytes).unwrap();
      let got = verify_file(&path, None).is_ok();
      std::fs::remove_file(path).unwrap();
      assert_eq!(got, want, "{extension}, fixture {index}");
    }
    assert!(verify_file(&root, None).is_err());
    assert!(verify_file(&root.join("missing.pdf"), None).is_err());
    let text = root.join("test.txt");
    std::fs::write(&text, b"hello").unwrap();
    assert!(verify_file(
      &text,
      Some("2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824")
    )
    .is_ok());
    assert!(verify_file(
      &text,
      Some("0000000000000000000000000000000000000000000000000000000000000000")
    )
    .is_err());
    assert!(verify_file(&text, None).is_err());
    std::fs::remove_file(&text).unwrap();
    std::fs::remove_dir(root).unwrap();
  }

  #[test]
  fn each_request_requires_new_files_for_all_selected_attachments() {
    assert!(!matches_selection(0, 0, 0, 0, 0));
    assert!(!matches_selection(4, 1, 3, 1, 0));
    assert!(matches_selection(4, 1, 4, 1, 0));
    assert_eq!(
      classify(&["membersOnly".into()], "Members-only content"),
      "LOGIN"
    );
    assert_eq!(classify(&["videoNotFoundOrPrivate".into()], ""), "FAIL");
    assert_eq!(classify(&[], "ERROR: Unknown item video404"), "FAIL");
    assert!(matches_expected("mixed", Some(3), true, 2, 1, 0));
    assert!(matches_expected("mixed", Some(2), false, 2, 1, 0));
  }

  #[test]
  fn engine_error_survives_a_trailing_loader_warning() {
    let output = "[PYI-1:ERROR] Failed to initialize sync semaphore!\nsemctl: Operation not permitted\n[PYI-1:WARNING] LOADER: failed to destroy sync semaphore";
    assert_eq!(
      last_engine_error(output),
      "[PYI-1:ERROR] Failed to initialize sync semaphore!"
    );
    assert_eq!(
      last_engine_error("WARNING: retry\nERROR: HTTP Error 403: Forbidden\n"),
      "ERROR: HTTP Error 403: Forbidden"
    );
  }

  #[test]
  fn classification_never_turns_a_generic_forbidden_or_rate_limit_into_login() {
    assert_eq!(classify(&[], "HTTP Error 403: Forbidden"), "FAIL");
    assert_eq!(classify(&[], "HTTP Error 429: Too Many Requests"), "FAIL");
    assert_eq!(classify(&["signInRequired".into()], ""), "LOGIN");
    assert_eq!(classify(&[], "ERROR: HTTP Error 404: Not Found"), "GONE");
    assert_eq!(
      classify(&[], "ERROR: Sign in to confirm you are not a bot"),
      "LOGIN"
    );
  }
}
