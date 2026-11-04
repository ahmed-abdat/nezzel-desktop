// Adapted from Zajil Download Manager, motalim-ai contributors. GPL-3.0-or-later.
// https://github.com/motalim-ai/zajil-download-manager (sanitize.rs)
pub const FALLBACK_NAME: &str = "download";

/// Longest name we produce, in UTF-8 bytes. Common file systems allow 255;
/// we keep room for the `.zajil-part` and `.zajil-state` suffixes.
pub const MAX_NAME_BYTES: usize = 200;

/// Longest extension kept when a name is shortened, in UTF-8 bytes.
const MAX_KEPT_EXTENSION_BYTES: usize = 20;

/// Device names Windows reserves in any folder, with or without an extension.
const WINDOWS_RESERVED: &[&str] = &[
  "CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$", "COM0", "COM1", "COM2", "COM3", "COM4", "COM5",
  "COM6", "COM7", "COM8", "COM9", "COM¹", "COM²", "COM³", "LPT0", "LPT1", "LPT2", "LPT3", "LPT4",
  "LPT5", "LPT6", "LPT7", "LPT8", "LPT9", "LPT¹", "LPT²", "LPT³",
];

/// Turns an untrusted name into a single safe file name. Never fails and
/// never returns an empty string.
///
/// - Keeps only the last path segment (after `/` or `\`).
/// - Removes control characters and invisible text-direction marks such as
///   U+202E, which can disguise `exe` as `txt`.
/// - Replaces characters Windows forbids (`<>:"|?*`) with `_`.
/// - Removes leading dots and spaces (no hidden files, no `..`) and trailing
///   dots and spaces (Windows drops them silently).
/// - Prefixes Windows reserved names (`CON`, `NUL.txt`, …) with `_`.
/// - Shortens the name to [`MAX_NAME_BYTES`], keeping a short extension.
pub fn sanitize_file_name(raw: &str) -> String {
  let last_segment = raw.rsplit(['/', '\\']).next().unwrap_or_default();

  // Joiners are judged after everything else is gone, against the
  // characters they would actually sit between.
  let cleaned: String = last_segment
    .chars()
    .filter(|&c| is_joiner(c) || !is_invisible_or_control(c))
    .map(|c| if is_forbidden(c) { '_' } else { c })
    .collect();

  let mut name = tidy(cleaned);
  if name.len() > MAX_NAME_BYTES {
    name = tidy(shorten(&name));
  }
  if name.is_empty() {
    return FALLBACK_NAME.to_owned();
  }
  if is_windows_reserved(&name) {
    name.insert(0, '_');
    // The `_` may take a name at the limit past it (found with the
    // user's own names, #103): shortened again, it keeps the `_`.
    if name.len() > MAX_NAME_BYTES {
      name = tidy(shorten(&name));
    }
  }
  name
}

/// Trims the edges and drops misplaced joiners until neither changes the
/// name: removing a joiner can expose a trailing dot, and trimming can leave
/// a joiner at an edge.
fn tidy(mut name: String) -> String {
  loop {
    let next = trim_edges(&keep_allowed_joiners(trim_edges(&name))).to_owned();
    if next == name {
      return name;
    }
    name = next;
  }
}

const ZWNJ: char = '\u{200C}';
const ZWJ: char = '\u{200D}';

fn is_joiner(c: char) -> bool {
  c == ZWNJ || c == ZWJ
}

/// Keeps a ZWNJ or ZWJ only where it is part of normal writing, by the
/// ContextJ rules of RFC 5892 (Appendix A.1 and A.2), and never at an edge
/// of the name or next to a dot (where it would only hide something):
/// - ZWNJ or ZWJ right after a virama (Indic scripts);
/// - ZWNJ between a left- or dual-joining letter and a right- or
///   dual-joining letter, with only transparent marks in between (Arabic
///   script: `می‌خواهم`).
///
/// Every other joiner is removed (`invoice\u{200C}.pdf` → `invoice.pdf`).
/// Emoji sequences joined with ZWJ are not allowed by these rules and are
/// split into their parts.
fn keep_allowed_joiners(name: &str) -> String {
  let chars: Vec<char> = name.chars().collect();
  chars
    .iter()
    .enumerate()
    .filter(|&(i, &c)| !is_joiner(c) || joiner_allowed(&chars, i))
    .map(|(_, &c)| c)
    .collect()
}

fn joiner_allowed(chars: &[char], i: usize) -> bool {
  use icu_properties::props::{CanonicalCombiningClass, JoiningType};
  use icu_properties::CodePointMapData;

  let (Some(&before), Some(&after)) = (chars.get(i.wrapping_sub(1)), chars.get(i + 1)) else {
    return false; // first or last character of the name
  };
  if before == '.' || after == '.' {
    return false;
  }
  // A.1 and A.2: Canonical_Combining_Class(Before(cp)) = Virama.
  let combining = CodePointMapData::<CanonicalCombiningClass>::new();
  if combining.get(before) == CanonicalCombiningClass::Virama {
    return true;
  }
  if chars[i] != ZWNJ {
    return false; // A.2 allows ZWJ only after a virama
  }
  // A.1: (Joining_Type:{L,D})(Joining_Type:T)*\u200C(Joining_Type:T)*(Joining_Type:{R,D})
  let joining = CodePointMapData::<JoiningType>::new();
  let not_transparent = |t: &JoiningType| *t != JoiningType::Transparent;
  let left = chars[..i]
    .iter()
    .rev()
    .map(|&c| joining.get(c))
    .find(not_transparent);
  let right = chars[i + 1..]
    .iter()
    .map(|&c| joining.get(c))
    .find(not_transparent);
  matches!(
    left,
    Some(JoiningType::LeftJoining | JoiningType::DualJoining)
  ) && matches!(
    right,
    Some(JoiningType::RightJoining | JoiningType::DualJoining)
  )
}

/// Control characters (Cc) and every invisible format character (Cf), line
/// separator (Zl) and paragraph separator (Zp), as of Unicode 16. These let
/// two different names look the same (`invoice\u{200B}.pdf`), hide the real
/// extension (U+202E) or break a name across lines in a terminal (U+2028).
/// ZWNJ and ZWJ (U+200C, U+200D) are in the list too, but
/// [`keep_allowed_joiners`] keeps them where the script needs them.
fn is_invisible_or_control(c: char) -> bool {
  c.is_control() || INVISIBLE.iter().any(|range| range.contains(&c))
}

/// Unicode categories Cf, Zl and Zp.
const INVISIBLE: &[std::ops::RangeInclusive<char>] = &[
  '\u{00AD}'..='\u{00AD}',   // soft hyphen
  '\u{0600}'..='\u{0605}',   // Arabic number signs
  '\u{061C}'..='\u{061C}',   // Arabic letter mark
  '\u{06DD}'..='\u{06DD}',   // Arabic end of ayah
  '\u{070F}'..='\u{070F}',   // Syriac abbreviation mark
  '\u{0890}'..='\u{0891}',   // Arabic pound and piastre marks above
  '\u{08E2}'..='\u{08E2}',   // Arabic disputed end of ayah
  '\u{180E}'..='\u{180E}',   // Mongolian vowel separator
  '\u{200B}'..='\u{200F}',   // zero-width space, joiners, LRM, RLM
  '\u{2028}'..='\u{202E}',   // line/paragraph separators, embeddings, overrides
  '\u{2060}'..='\u{2064}',   // word joiner, invisible operators
  '\u{2066}'..='\u{206F}',   // isolates and deprecated format characters
  '\u{FEFF}'..='\u{FEFF}',   // byte order mark
  '\u{FFF9}'..='\u{FFFB}',   // interlinear annotation
  '\u{110BD}'..='\u{110BD}', // Kaithi number sign
  '\u{110CD}'..='\u{110CD}', // Kaithi number sign above
  '\u{13430}'..='\u{1343F}', // Egyptian hieroglyph format controls
  '\u{1BCA0}'..='\u{1BCA3}', // shorthand format controls
  '\u{1D173}'..='\u{1D17A}', // musical symbol format controls
  '\u{E0001}'..='\u{E0001}', // language tag
  '\u{E0020}'..='\u{E007F}', // tag characters
];

fn is_forbidden(c: char) -> bool {
  matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*')
}

fn trim_edges(name: &str) -> &str {
  name
    .trim_start_matches(|c: char| c == '.' || c.is_whitespace())
    .trim_end_matches(|c: char| c == '.' || c.is_whitespace())
}

fn is_windows_reserved(name: &str) -> bool {
  // Windows ignores everything from the first dot, and trailing spaces
  // before it: `nul .txt` is also the NUL device.
  let stem = name.split('.').next().unwrap_or_default().trim_end();
  WINDOWS_RESERVED
    .iter()
    .any(|reserved| stem.eq_ignore_ascii_case(reserved))
}

/// Cuts `name` to [`MAX_NAME_BYTES`] at a character boundary, keeping the
/// extension when it is short.
fn shorten(name: &str) -> String {
  let extension = name
    .rfind('.')
    .map(|dot| &name[dot..])
    .filter(|ext| ext.len() <= MAX_KEPT_EXTENSION_BYTES && ext.len() > 1)
    .unwrap_or("");
  let stem = &name[..name.len() - extension.len()];
  let budget = MAX_NAME_BYTES - extension.len();
  let mut cut = budget.min(stem.len());
  while !stem.is_char_boundary(cut) {
    cut -= 1;
  }
  format!("{}{}", &stem[..cut], extension)
}
