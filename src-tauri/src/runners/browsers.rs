//! The browser model: one table of the browsers Nezzel knows, the `--cookies-from-browser` value parsed once
//! into a [`BrowserSource`], and the [`CookieSource`] a run's login comes from.
use super::browser_logins::Os;
use crate::state::config_models::AuthSettings;

/// A browser a login (cookies) or a captured link can come from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BrowserKind {
  Chrome,
  Firefox,
  Safari,
  Edge,
  Brave,
  Opera,
  Vivaldi,
  Chromium,
  Whale,
  Arc,
}

/// How a browser keeps its cookies: decides the reader and the files a profile holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CookieStore {
  Chromium,
  Firefox,
  Safari,
}

struct Entry {
  #[cfg_attr(not(test), allow(dead_code))]
  kind: BrowserKind,
  /// yt-dlp's `--cookies-from-browser` name; also the config and wire key.
  name: &'static str,
  store: CookieStore,
  /// yt-dlp reads its cookies, so it can give a login.
  logins: bool,
  /// The profile root is itself the default profile (Opera Stable).
  root_profile: bool,
  /// Substrings of the OS default-browser id (macOS LaunchServices, Windows ProgId, xdg desktop file).
  default_ids: &'static [&'static str],
  /// macOS bundle that answers AppleScript for the front tab's URL (capture).
  mac_bundle: Option<&'static str>,
  /// Windows process whose address bar a capture reads.
  windows_process: Option<&'static str>,
}

const fn entry(kind: BrowserKind, name: &'static str, store: CookieStore) -> Entry {
  Entry {
    kind,
    name,
    store,
    logins: true,
    root_profile: false,
    default_ids: &[],
    mac_bundle: None,
    windows_process: None,
  }
}

/// In `BrowserKind` order, which is also the order Settings lists them.
const TABLE: [Entry; 10] = [
  Entry {
    default_ids: &["chrome"],
    mac_bundle: Some("com.google.Chrome"),
    windows_process: Some("chrome"),
    ..entry(BrowserKind::Chrome, "chrome", CookieStore::Chromium)
  },
  Entry {
    default_ids: &["firefox"],
    windows_process: Some("firefox"),
    ..entry(BrowserKind::Firefox, "firefox", CookieStore::Firefox)
  },
  Entry {
    default_ids: &["safari"],
    mac_bundle: Some("com.apple.Safari"),
    ..entry(BrowserKind::Safari, "safari", CookieStore::Safari)
  },
  Entry {
    default_ids: &["msedge", "microsoft-edge"],
    mac_bundle: Some("com.microsoft.edgemac"),
    windows_process: Some("msedge"),
    ..entry(BrowserKind::Edge, "edge", CookieStore::Chromium)
  },
  Entry {
    default_ids: &["brave"],
    mac_bundle: Some("com.brave.Browser"),
    windows_process: Some("brave"),
    ..entry(BrowserKind::Brave, "brave", CookieStore::Chromium)
  },
  Entry {
    root_profile: true,
    default_ids: &["opera"],
    windows_process: Some("opera"),
    ..entry(BrowserKind::Opera, "opera", CookieStore::Chromium)
  },
  Entry {
    default_ids: &["vivaldi"],
    windows_process: Some("vivaldi"),
    ..entry(BrowserKind::Vivaldi, "vivaldi", CookieStore::Chromium)
  },
  Entry {
    default_ids: &["chromium"],
    ..entry(BrowserKind::Chromium, "chromium", CookieStore::Chromium)
  },
  Entry {
    default_ids: &["whale"],
    ..entry(BrowserKind::Whale, "whale", CookieStore::Chromium)
  },
  // Captures read Arc's link; yt-dlp cannot read its cookies.
  Entry {
    logins: false,
    mac_bundle: Some("company.thebrowser.Browser"),
    windows_process: Some("arc"),
    ..entry(BrowserKind::Arc, "arc", CookieStore::Chromium)
  },
];

impl BrowserKind {
  pub const ALL: [BrowserKind; 10] = [
    Self::Chrome,
    Self::Firefox,
    Self::Safari,
    Self::Edge,
    Self::Brave,
    Self::Opera,
    Self::Vivaldi,
    Self::Chromium,
    Self::Whale,
    Self::Arc,
  ];

  fn entry(self) -> &'static Entry {
    &TABLE[self as usize]
  }

  pub fn name(self) -> &'static str {
    self.entry().name
  }

  /// A yt-dlp browser name, any case (yt-dlp lowercases it too).
  pub fn from_name(name: &str) -> Option<Self> {
    Self::ALL
      .into_iter()
      .find(|kind| kind.name().eq_ignore_ascii_case(name))
  }

  pub fn store(self) -> CookieStore {
    self.entry().store
  }

  /// yt-dlp can read this browser's cookies.
  pub fn logins(self) -> bool {
    self.entry().logins
  }

  pub fn root_profile(self) -> bool {
    self.entry().root_profile
  }

  /// Windows Chromium jars may use app-bound encryption yt-dlp cannot read.
  pub fn app_bound_on(self, os: Os) -> bool {
    matches!(os, Os::Windows) && self.store() == CookieStore::Chromium && self.logins()
  }

  /// The browser behind an OS default-browser id ("com.google.chrome", "MSEdgeHTM", "firefox.desktop").
  pub fn from_default_id(value: &str) -> Option<Self> {
    let value = value.to_ascii_lowercase();
    Self::ALL.into_iter().find(|kind| {
      kind
        .entry()
        .default_ids
        .iter()
        .any(|needle| value.contains(needle))
    })
  }

  #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
  pub fn from_mac_bundle(bundle: &str) -> Option<Self> {
    Self::ALL
      .into_iter()
      .find(|kind| kind.entry().mac_bundle == Some(bundle))
  }

  #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
  pub fn mac_bundle(self) -> Option<&'static str> {
    self.entry().mac_bundle
  }

  /// Process names of the browsers whose address bar a Windows capture reads.
  #[cfg_attr(not(any(windows, test)), allow(dead_code))]
  pub fn windows_processes() -> impl Iterator<Item = &'static str> {
    TABLE.iter().filter_map(|entry| entry.windows_process)
  }
}

impl serde::Serialize for BrowserKind {
  fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(self.name())
  }
}

/// Browsers a login can come from, as yt-dlp names them, in the order Settings lists them (`crate::contract`).
#[cfg_attr(not(test), allow(dead_code))]
pub fn cookie_browsers() -> impl Iterator<Item = &'static str> {
  BrowserKind::ALL
    .into_iter()
    .filter(|kind| kind.logins())
    .map(BrowserKind::name)
}

/// A `--cookies-from-browser` value: `BROWSER[+KEYRING][:PROFILE][::CONTAINER]`. The profile is a name or
/// an absolute profile directory; the container is a Firefox container name (`none`: no container).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrowserSource {
  pub kind: BrowserKind,
  pub keyring: Option<String>,
  pub profile: Option<String>,
  pub container: Option<String>,
}

impl BrowserSource {
  /// Parses as yt-dlp does (`yt_dlp/__init__.py`); `None` when yt-dlp would reject it.
  pub fn parse(spec: &str) -> Option<Self> {
    let name_end = spec.find(['+', ':']).unwrap_or(spec.len());
    let kind = BrowserKind::from_name(&spec[..name_end]).filter(|kind| kind.logins())?;
    let mut rest = &spec[name_end..];
    let mut keyring = None;
    if let Some(after) = rest.strip_prefix('+') {
      let end = after.find(':').unwrap_or(after.len());
      let name = after[..end].trim_start();
      if name.is_empty() {
        return None;
      }
      keyring = Some(name.to_owned());
      rest = &after[end..];
    }
    let (mut profile, mut container) = (None, None);
    if let Some(after) = rest.strip_prefix("::") {
      let after = after.trim_start();
      if after.is_empty() {
        return None;
      }
      container = Some(after);
    } else if let Some(after) = rest.strip_prefix(':') {
      let after = after.trim_start();
      if after.is_empty() || after.starts_with(':') {
        return None;
      }
      // The profile is the shortest prefix the rest of which is `::CONTAINER`.
      let split = after.match_indices("::").find_map(|(at, _)| {
        let tail = after[at + 2..].trim_start();
        (!tail.is_empty()).then_some((after[..at].trim_end(), tail))
      });
      match split {
        Some((name, tail)) => (profile, container) = (Some(name), Some(tail)),
        None => profile = Some(after),
      }
    } else if !rest.is_empty() {
      return None;
    }
    Some(Self {
      kind,
      keyring,
      profile: profile.map(str::to_owned),
      container: container.map(str::to_owned),
    })
  }

  /// Back to the value yt-dlp takes.
  pub fn spec(&self) -> String {
    let mut spec = self.kind.name().to_owned();
    for (separator, part) in [
      ("+", &self.keyring),
      (":", &self.profile),
      ("::", &self.container),
    ] {
      if let Some(part) = part {
        spec.push_str(separator);
        spec.push_str(part);
      }
    }
    spec
  }
}

/// `AuthSettings.cookie_browser`: find a signed-in profile when a site refuses.
pub const AUTOMATIC: &str = "automatic";
/// `AuthSettings.cookie_browser`: never use browser logins.
pub const NEVER: &str = "none";

/// What `AuthSettings.cookie_browser` asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrowserSetting<'a> {
  Automatic,
  Never,
  /// A browser (yt-dlp value) picked by hand.
  Fixed(&'a str),
}

impl<'a> BrowserSetting<'a> {
  pub fn of(auth: &'a AuthSettings) -> Self {
    match auth.cookie_browser.as_str() {
      AUTOMATIC => Self::Automatic,
      NEVER | "" => Self::Never,
      browser => Self::Fixed(browser),
    }
  }
}

/// Automatic browser logins and no manual cookie file.
pub fn automatic(auth: &AuthSettings) -> bool {
  BrowserSetting::of(auth) == BrowserSetting::Automatic && manual_file(auth).is_none()
}

fn manual_file(auth: &AuthSettings) -> Option<&str> {
  auth.cookie_file.as_deref().filter(|file| !file.is_empty())
}

/// Where a run's cookies come from, decided once.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CookieSource {
  Anonymous,
  /// A cookies.txt the user picked; engines read it as is.
  File(String),
  /// Exported, scoped to the site, into a private jar.
  Browser(BrowserSource),
}

impl CookieSource {
  /// A fixed Settings browser is implicit: export only for known login sites. An explicit row
  /// choice (including a login retry) and a manual cookie file retain their existing consent.
  pub fn for_site(
    auth: &AuthSettings,
    browser: Option<&str>,
    host: &str,
  ) -> Result<Self, &'static str> {
    if browser.is_none()
      && manual_file(auth).is_none()
      && !super::browser_cookies::known_login_host(host)
    {
      return Ok(Self::Anonymous);
    }
    Self::select(auth, browser)
  }

  /// The row's login (`browser`, consented) wins, then a manual file, then a browser picked in Settings.
  /// `Err` when the browser is one yt-dlp cannot read.
  pub fn select(auth: &AuthSettings, browser: Option<&str>) -> Result<Self, &'static str> {
    let browser = match browser {
      Some(super::login::ANONYMOUS_BROWSER) => return Ok(Self::Anonymous),
      Some(browser) => browser,
      None => {
        if let Some(file) = manual_file(auth) {
          return Ok(Self::File(file.to_owned()));
        }
        match BrowserSetting::of(auth) {
          BrowserSetting::Fixed(browser) => browser,
          BrowserSetting::Automatic | BrowserSetting::Never => return Ok(Self::Anonymous),
        }
      }
    };
    BrowserSource::parse(browser)
      .map(Self::Browser)
      .ok_or(super::browser_cookies::COOKIE_READ_ERROR)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn table_rows_follow_the_enum_and_names_are_unique() {
    for (index, kind) in BrowserKind::ALL.into_iter().enumerate() {
      assert_eq!(TABLE[index].kind, kind);
      assert_eq!(BrowserKind::from_name(kind.name()), Some(kind));
      assert_eq!(
        serde_json::to_value(kind).unwrap(),
        serde_json::json!(kind.name())
      );
    }
    assert_eq!(
      BrowserKind::from_name("FireFox"),
      Some(BrowserKind::Firefox)
    );
    assert_eq!(BrowserKind::from_name("netscape"), None);
    assert_eq!(
      cookie_browsers().collect::<Vec<_>>(),
      ["chrome", "firefox", "safari", "edge", "brave", "opera", "vivaldi", "chromium", "whale"]
    );
  }

  #[test]
  fn app_bound_is_windows_chromium_family_only() {
    let bound: Vec<_> = BrowserKind::ALL
      .into_iter()
      .filter(|kind| kind.app_bound_on(Os::Windows))
      .map(BrowserKind::name)
      .collect();
    assert_eq!(
      bound,
      ["chrome", "edge", "brave", "opera", "vivaldi", "chromium", "whale"]
    );
    assert!(BrowserKind::ALL
      .into_iter()
      .all(|kind| !kind.app_bound_on(Os::Mac) && !kind.app_bound_on(Os::Linux)));
  }

  #[test]
  fn capture_browsers_are_the_ones_answering_for_their_url() {
    assert_eq!(
      BrowserKind::windows_processes().collect::<Vec<_>>(),
      ["chrome", "firefox", "msedge", "brave", "opera", "vivaldi", "arc"]
    );
    for bundle in [
      "com.apple.Safari",
      "com.google.Chrome",
      "com.microsoft.edgemac",
      "com.brave.Browser",
      "company.thebrowser.Browser",
    ] {
      let kind = BrowserKind::from_mac_bundle(bundle).unwrap();
      assert_eq!(kind.mac_bundle(), Some(bundle));
    }
    assert_eq!(BrowserKind::from_mac_bundle("org.mozilla.firefox"), None);
  }

  #[test]
  fn browser_sources_parse_like_yt_dlp_and_round_trip() {
    let source = |kind, keyring: Option<&str>, profile: Option<&str>, container: Option<&str>| {
      Some(BrowserSource {
        kind,
        keyring: keyring.map(str::to_owned),
        profile: profile.map(str::to_owned),
        container: container.map(str::to_owned),
      })
    };
    use BrowserKind::{Chrome, Firefox};
    for (spec, expected) in [
      ("chrome", source(Chrome, None, None, None)),
      ("Chrome", source(Chrome, None, None, None)),
      (
        "chrome:/tmp/Profile 1",
        source(Chrome, None, Some("/tmp/Profile 1"), None),
      ),
      (
        r"chrome:C:\Users\me\AppData\Local\Google\Chrome\User Data\Profile 1",
        source(
          Chrome,
          None,
          Some(r"C:\Users\me\AppData\Local\Google\Chrome\User Data\Profile 1"),
          None,
        ),
      ),
      (
        "chrome+gnomekeyring:Profile 2",
        source(Chrome, Some("gnomekeyring"), Some("Profile 2"), None),
      ),
      (
        "firefox:abc.default",
        source(Firefox, None, Some("abc.default"), None),
      ),
      (
        "firefox:/home/me/.mozilla/firefox/abc.default::Work",
        source(
          Firefox,
          None,
          Some("/home/me/.mozilla/firefox/abc.default"),
          Some("Work"),
        ),
      ),
      ("firefox::none", source(Firefox, None, None, Some("none"))),
      (
        "firefox:p :: Shopping",
        source(Firefox, None, Some("p"), Some("Shopping")),
      ),
      ("", None),
      ("netscape", None),
      ("arc", None),
      ("none", None),
      ("chrome:", None),
      ("chrome+", None),
      ("firefox::", None),
    ] {
      let parsed = BrowserSource::parse(spec);
      assert_eq!(parsed, expected, "{spec:?}");
      if let Some(parsed) = parsed {
        assert_eq!(
          BrowserSource::parse(&parsed.spec()),
          Some(parsed),
          "{spec:?}"
        );
      }
    }
    assert_eq!(
      BrowserSource::parse("Firefox:/p::Work").unwrap().spec(),
      "firefox:/p::Work"
    );
  }

  #[test]
  fn browser_setting_words_match_the_config_default() {
    let mut auth = AuthSettings::default();
    assert_eq!(auth.cookie_browser, AUTOMATIC);
    assert_eq!(BrowserSetting::of(&auth), BrowserSetting::Automatic);
    assert!(automatic(&auth));
    auth.cookie_file = Some(String::new());
    assert!(automatic(&auth));
    auth.cookie_file = Some("/tmp/manual.txt".into());
    assert!(!automatic(&auth));
    for (word, setting) in [
      (NEVER, BrowserSetting::Never),
      ("", BrowserSetting::Never),
      ("firefox", BrowserSetting::Fixed("firefox")),
    ] {
      auth.cookie_browser = word.into();
      assert_eq!(BrowserSetting::of(&auth), setting, "{word:?}");
      assert!(!automatic(&auth));
    }
  }

  // Moved from ytdlp_runner (`cookie_source_args`).
  #[test]
  fn cookie_source_requires_consent_but_preserves_manual_overrides() {
    let browser = |spec| CookieSource::Browser(BrowserSource::parse(spec).unwrap());
    let mut settings = AuthSettings::default();
    assert_eq!(
      CookieSource::select(&settings, None),
      Ok(CookieSource::Anonymous)
    );
    settings.cookie_browser = NEVER.into();
    assert_eq!(
      CookieSource::select(&settings, None),
      Ok(CookieSource::Anonymous)
    );
    settings.cookie_browser = "firefox".into();
    assert_eq!(
      CookieSource::select(&settings, None),
      Ok(browser("firefox"))
    );
    settings.cookie_file = Some("/tmp/manual.txt".into());
    assert_eq!(
      CookieSource::select(&settings, None),
      Ok(CookieSource::File("/tmp/manual.txt".into()))
    );
    assert_eq!(
      CookieSource::select(&settings, Some("chrome:/tmp/Profile 1")),
      Ok(browser("chrome:/tmp/Profile 1"))
    );
    assert_eq!(
      CookieSource::select(&settings, Some(super::super::login::ANONYMOUS_BROWSER)),
      Ok(CookieSource::Anonymous)
    );
    assert!(CookieSource::select(&settings, Some("netscape")).is_err());
    settings.cookie_file = None;
    settings.cookie_browser = "netscape".into();
    assert!(CookieSource::select(&settings, None).is_err());
  }
}
