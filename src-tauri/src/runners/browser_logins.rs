//! Profile names and site-session metadata. Cookie values are never projected or decrypted.
#[cfg(test)]
use super::browser_cookies::cookie_hint;
use super::browser_cookies::{cookie_hint_for_container, firefox_containers, CookieHint};
use super::browsers::{BrowserKind, BrowserSource, CookieStore};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Clone, Debug, PartialEq)]
pub struct BrowserCandidate {
  pub id: String,
  pub browser: BrowserKind,
  pub profile: String,
  pub modified: SystemTime,
  pub signed_in: bool,
  pub app_bound: bool,
  pub active_time: f64,
}
#[derive(Clone, Copy)]
pub enum Os {
  Mac,
  Windows,
  Linux,
}

/// The machine discovery reads: the real one in the app, a fixture in tests.
#[derive(Clone)]
pub(super) struct BrowserEnv {
  pub os: Os,
  pub home: PathBuf,
  pub local: PathBuf,
  pub roaming: PathBuf,
  pub default: Option<BrowserKind>,
  /// Snapshots a cookie database (and its WAL) before a sign-in hint reads it.
  pub copy_file: fn(&Path, &Path) -> std::io::Result<u64>,
}

impl BrowserEnv {
  fn current() -> Self {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
      .map(PathBuf::from)
      .unwrap_or_default();
    let local = std::env::var_os("LOCALAPPDATA")
      .map(PathBuf::from)
      .unwrap_or_default();
    let roaming = std::env::var_os("APPDATA")
      .map(PathBuf::from)
      .unwrap_or_default();
    let os = if cfg!(target_os = "macos") {
      Os::Mac
    } else if cfg!(windows) {
      Os::Windows
    } else {
      Os::Linux
    };
    let default = default_browser(&home);
    Self {
      os,
      home,
      local,
      roaming,
      default,
      copy_file: |from, to| std::fs::copy(from, to),
    }
  }

  /// No profile roots, no default browser, real file copies.
  #[cfg(test)]
  pub(super) fn test(os: Os) -> Self {
    Self {
      os,
      home: PathBuf::new(),
      local: PathBuf::new(),
      roaming: PathBuf::new(),
      default: None,
      copy_file: |from, to| std::fs::copy(from, to),
    }
  }

  /// Every profile root under `root`.
  #[cfg(test)]
  fn at(self, root: &Path) -> Self {
    Self {
      home: root.into(),
      local: root.into(),
      roaming: root.into(),
      ..self
    }
  }
}

fn discover_in(
  env: &BrowserEnv,
  remembered: Option<&str>,
  host: Option<&str>,
) -> Vec<BrowserCandidate> {
  let (os, home, local, roaming, default) =
    (env.os, &env.home, &env.local, &env.roaming, env.default);
  let roots = match os {
    Os::Mac => vec![
      (
        BrowserKind::Firefox,
        home.join("Library/Application Support/Firefox/Profiles"),
      ),
      (
        BrowserKind::Chrome,
        home.join("Library/Application Support/Google/Chrome"),
      ),
      (
        BrowserKind::Edge,
        home.join("Library/Application Support/Microsoft Edge"),
      ),
      (
        BrowserKind::Brave,
        home.join("Library/Application Support/BraveSoftware/Brave-Browser"),
      ),
    ],
    Os::Windows => vec![
      (
        BrowserKind::Firefox,
        roaming.join("Mozilla/Firefox/Profiles"),
      ),
      (BrowserKind::Chrome, local.join("Google/Chrome/User Data")),
      (BrowserKind::Edge, local.join("Microsoft/Edge/User Data")),
      (
        BrowserKind::Brave,
        local.join("BraveSoftware/Brave-Browser/User Data"),
      ),
    ],
    Os::Linux => vec![
      (BrowserKind::Firefox, home.join(".mozilla/firefox")),
      (BrowserKind::Firefox, home.join(".config/mozilla/firefox")),
      (
        BrowserKind::Firefox,
        home.join("snap/firefox/common/.mozilla/firefox"),
      ),
      (
        BrowserKind::Firefox,
        home.join(".var/app/org.mozilla.firefox/.mozilla/firefox"),
      ),
      (BrowserKind::Chrome, home.join(".config/google-chrome")),
      (BrowserKind::Edge, home.join(".config/microsoft-edge")),
      (
        BrowserKind::Brave,
        home.join(".config/BraveSoftware/Brave-Browser"),
      ),
    ],
  };
  let mut roots = roots;
  roots.extend(match os {
    Os::Mac => vec![
      (
        BrowserKind::Chromium,
        home.join("Library/Application Support/Chromium"),
      ),
      (
        BrowserKind::Vivaldi,
        home.join("Library/Application Support/Vivaldi"),
      ),
      (
        BrowserKind::Opera,
        home.join("Library/Application Support/com.operasoftware.Opera"),
      ),
      (
        BrowserKind::Opera,
        home.join("Library/Application Support/com.operasoftware.OperaGX"),
      ),
      (
        BrowserKind::Whale,
        home.join("Library/Application Support/Naver/Whale"),
      ),
    ],
    Os::Windows => vec![
      (BrowserKind::Chromium, local.join("Chromium/User Data")),
      (BrowserKind::Vivaldi, local.join("Vivaldi/User Data")),
      (
        BrowserKind::Opera,
        roaming.join("Opera Software/Opera Stable"),
      ),
      (
        BrowserKind::Opera,
        roaming.join("Opera Software/Opera GX Stable"),
      ),
      (
        BrowserKind::Whale,
        local.join("Naver/Naver Whale/User Data"),
      ),
    ],
    Os::Linux => vec![
      (BrowserKind::Chromium, home.join(".config/chromium")),
      (BrowserKind::Vivaldi, home.join(".config/vivaldi")),
      (BrowserKind::Opera, home.join(".config/opera")),
      (BrowserKind::Whale, home.join(".config/naver-whale")),
    ],
  });
  if matches!(os, Os::Linux) {
    roots.extend([
      (
        BrowserKind::Chromium,
        home.join("snap/chromium/common/chromium"),
      ),
      (
        BrowserKind::Chromium,
        home.join("snap/chromium/common/.config/chromium"),
      ),
      (
        BrowserKind::Chrome,
        home.join("snap/google-chrome/common/.config/google-chrome"),
      ),
      (
        BrowserKind::Brave,
        home.join("snap/brave/current/.config/BraveSoftware/Brave-Browser"),
      ),
      (
        BrowserKind::Brave,
        home.join("snap/brave/common/.config/BraveSoftware/Brave-Browser"),
      ),
      (
        BrowserKind::Chrome,
        home.join(".var/app/com.google.Chrome/config/google-chrome"),
      ),
      (
        BrowserKind::Chromium,
        home.join(".var/app/org.chromium.Chromium/config/chromium"),
      ),
      (
        BrowserKind::Edge,
        home.join(".var/app/com.microsoft.Edge/config/microsoft-edge"),
      ),
      (
        BrowserKind::Brave,
        home.join(".var/app/com.brave.Browser/config/BraveSoftware/Brave-Browser"),
      ),
      (
        BrowserKind::Vivaldi,
        home.join(".var/app/com.vivaldi.Vivaldi/config/vivaldi"),
      ),
      (
        BrowserKind::Opera,
        home.join(".var/app/com.opera.Opera/config/opera"),
      ),
    ]);
  }
  let now = SystemTime::now()
    .duration_since(SystemTime::UNIX_EPOCH)
    .map(|d| d.as_secs() as i64)
    .unwrap_or(0);
  let mut found = Vec::new();
  for (browser, root) in roots.into_iter().filter(|(_, root)| root.is_absolute()) {
    let state: serde_json::Value = std::fs::read(root.join("Local State"))
      .ok()
      .and_then(|bytes| serde_json::from_slice(&bytes).ok())
      .unwrap_or_default();
    let firefox_names = firefox_profiles(&root);
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&root)
      .into_iter()
      .flatten()
      .flatten()
      .map(|e| e.path())
      .filter(|p| p.is_dir())
      .collect();
    paths.extend(firefox_names.keys().cloned());
    if browser.root_profile() {
      paths.push(root.clone());
    }
    paths.sort();
    paths.dedup();
    for path in paths {
      let cookie_paths = if browser.store() == CookieStore::Firefox {
        vec![path.join("cookies.sqlite")]
      } else {
        vec![path.join("Network/Cookies"), path.join("Cookies")]
      };
      let modified = cookie_paths
        .iter()
        .filter_map(|p| std::fs::metadata(p).ok())
        .filter(|m| m.is_file())
        .filter_map(|m| m.modified().ok())
        .max();
      if let Some(modified) = modified {
        let directory = path.file_name().unwrap_or_default().to_string_lossy();
        let key = if browser.root_profile() && path == root {
          "Default"
        } else {
          directory.as_ref()
        };
        let info = &state["profile"]["info_cache"][key];
        let hint = host
          .map(|host| {
            cookie_paths
              .iter()
              .filter(|p| p.is_file())
              .map(|p| {
                cookie_hint_for_container(
                  env,
                  p,
                  browser,
                  host,
                  now,
                  (browser.store() == CookieStore::Firefox).then_some(0),
                )
              })
              .fold(CookieHint::default(), |best, hint| {
                // An old root Cookies file must not hide the active Network database's v20.
                let app_bound = best.app_bound || hint.app_bound;
                let mut selected =
                  if (hint.signed_in, hint.last_access) > (best.signed_in, best.last_access) {
                    hint
                  } else {
                    best
                  };
                selected.app_bound = app_bound;
                selected
              })
          })
          .unwrap_or_default();
        // Windows Chromium without a known site session has no login to offer.
        if host.is_some_and(super::browser_cookies::known_login_host)
          && browser.app_bound_on(os)
          && !hint.signed_in
          && !hint.app_bound
        {
          continue;
        }
        let candidate = BrowserCandidate {
          id: format!("{}:{}", browser.name(), path.to_string_lossy()),
          browser,
          profile: if browser.store() == CookieStore::Firefox {
            firefox_names
              .get(&path)
              .cloned()
              .unwrap_or_else(|| directory.into_owned())
          } else {
            chromium_profile_name(info, &directory)
          },
          modified,
          signed_in: hint.signed_in,
          app_bound: hint.app_bound,
          active_time: if hint.signed_in {
            hint.last_access as f64
          } else {
            info["active_time"]
              .as_f64()
              .filter(|v| v.is_finite())
              .unwrap_or(0.0)
          },
        };
        if browser == BrowserKind::Firefox {
          let containers = firefox_containers(&path);
          for (name, id) in containers {
            let hint = host
              .map(|host| {
                cookie_hint_for_container(
                  env,
                  &path.join("cookies.sqlite"),
                  browser,
                  host,
                  now,
                  Some(id),
                )
              })
              .unwrap_or_default();
            let source = BrowserSource {
              kind: browser,
              keyring: None,
              profile: Some(path.to_string_lossy().into_owned()),
              container: Some(name.clone()),
            };
            if BrowserSource::parse(&source.spec()).as_ref() != Some(&source) {
              continue;
            }
            found.push(BrowserCandidate {
              id: source.spec(),
              profile: format!("{} · {}", candidate.profile, name),
              signed_in: hint.signed_in,
              active_time: hint.last_access as f64,
              ..candidate.clone()
            });
          }
        }
        found.push(candidate);
      }
    }
  }

  if matches!(os, Os::Mac) {
    for path in [
      home.join("Library/Cookies/Cookies.binarycookies"),
      home.join("Library/Containers/com.apple.Safari/Data/Library/Cookies/Cookies.binarycookies"),
    ] {
      if let Ok(m) = std::fs::metadata(path) {
        if m.is_file() {
          found.push(BrowserCandidate {
            id: BrowserKind::Safari.name().into(),
            browser: BrowserKind::Safari,
            profile: "Default".into(),
            modified: m.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            signed_in: false,
            app_bound: false,
            active_time: 0.0,
          });
          break;
        }
      }
    }
  }
  // Disambiguate only within a browser, preserving the discovery/ranking order.
  let mut counts = std::collections::HashMap::new();
  for c in &found {
    *counts.entry((c.browser, c.profile.clone())).or_insert(0) += 1;
  }
  for c in &mut found {
    if counts[&(c.browser, c.profile.clone())] > 1 {
      let directory = c
        .id
        .split_once(':')
        .and_then(|(_, path)| Path::new(path).file_name())
        .unwrap_or_default()
        .to_string_lossy();
      c.profile = format!("{} · {}", c.profile, directory);
    }
  }
  let order = {
    use BrowserKind::*;
    [
      Firefox, Chrome, Edge, Brave, Chromium, Vivaldi, Opera, Whale, Safari,
    ]
  };
  found.sort_by(|a, b| {
    let tier = |candidate: &BrowserCandidate| {
      if candidate.app_bound {
        5
      } else if candidate.signed_in && candidate.browser != BrowserKind::Firefox {
        0
      } else if candidate.signed_in {
        1
      } else if candidate.browser == BrowserKind::Safari {
        2
      } else if remembered == Some(candidate.id.as_str()) {
        3
      } else {
        4
      }
    };
    tier(a)
      .cmp(&tier(b))
      .then_with(|| b.active_time.total_cmp(&a.active_time))
      .then_with(|| (default != Some(a.browser)).cmp(&(default != Some(b.browser))))
      .then_with(|| {
        order
          .iter()
          .position(|x| *x == a.browser)
          .cmp(&order.iter().position(|x| *x == b.browser))
      })
      .then_with(|| b.modified.cmp(&a.modified))
      .then_with(|| a.id.cmp(&b.id))
  });
  found
}

pub fn discover(remembered: Option<&str>, host: &str) -> Vec<BrowserCandidate> {
  discover_in(&BrowserEnv::current(), remembered, Some(host))
}

fn chromium_profile_name(info: &serde_json::Value, directory: &str) -> String {
  let text = |key| info[key].as_str().map(str::trim).filter(|s| !s.is_empty());
  let name = text("name").unwrap_or(directory);
  let given = text("gaia_given_name");
  if info["is_using_default_name"].as_bool() == Some(true) {
    given.or_else(|| text("gaia_name")).unwrap_or(name).into()
  } else if let Some(given) = given.filter(|given| *given != name) {
    format!("{given} ({name})")
  } else {
    name.into()
  }
}

fn firefox_profiles(root: &Path) -> std::collections::HashMap<PathBuf, String> {
  let mut names = std::collections::HashMap::new();
  let ini_root = if root.file_name().is_some_and(|s| s == "Profiles") {
    root.parent().unwrap_or(root)
  } else {
    root
  };
  let Ok(ini) = std::fs::read_to_string(ini_root.join("profiles.ini")) else {
    return names;
  };
  for section in ini.split('[').filter(|s| s.starts_with("Profile")) {
    let fields: std::collections::HashMap<_, _> = section
      .lines()
      .filter_map(|line| line.trim().split_once('='))
      .collect();
    if let (Some(path), Some(name)) = (fields.get("Path"), fields.get("Name")) {
      if !name.is_empty() {
        let path = if fields.get("IsRelative") == Some(&"0") {
          PathBuf::from(path)
        } else {
          ini_root.join(path)
        };
        names.insert(path, (*name).into());
      }
    }
  }
  names
}

#[cfg(test)]
fn has_host_cookies(path: &Path, browser: BrowserKind, host: &str) -> bool {
  cookie_hint(&BrowserEnv::test(Os::Mac), path, browser, host, 1).signed_in
}

/// Names only, for displaying an already selected browser. Does not inspect cookies.
pub fn describe(id: &str) -> Option<BrowserCandidate> {
  let source = BrowserSource::parse(id)?;
  let browser = source.kind;
  let path = Path::new(source.profile.as_deref().unwrap_or_default());
  let directory = path.file_name().unwrap_or_default().to_string_lossy();
  let profile = if browser.store() == CookieStore::Safari {
    "Default".into()
  } else if browser.store() == CookieStore::Firefox {
    let root = path.parent().unwrap_or(path);
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
      .map(PathBuf::from)
      .unwrap_or_default();
    let roaming = std::env::var_os("APPDATA")
      .map(PathBuf::from)
      .unwrap_or_default();
    [
      root.to_path_buf(),
      home.join("Library/Application Support/Firefox"),
      roaming.join("Mozilla/Firefox"),
      home.join(".mozilla/firefox"),
      home.join(".config/mozilla/firefox"),
      home.join("snap/firefox/common/.mozilla/firefox"),
      home.join(".var/app/org.mozilla.firefox/.mozilla/firefox"),
    ]
    .into_iter()
    .filter(|root| root.is_absolute())
    .find_map(|root| firefox_profiles(&root).remove(path))
    .unwrap_or_else(|| directory.to_string())
  } else {
    let root = if browser.root_profile() && path.join("Local State").is_file() {
      path
    } else {
      path.parent().unwrap_or(path)
    };
    let state: serde_json::Value = std::fs::read(root.join("Local State"))
      .ok()
      .and_then(|v| serde_json::from_slice(&v).ok())
      .unwrap_or_default();
    chromium_profile_name(
      &state["profile"]["info_cache"][if path == root {
        "Default"
      } else {
        directory.as_ref()
      }],
      &directory,
    )
  };
  let profile = match source.container.as_deref() {
    Some(container) if container != "none" => format!("{profile} · {container}"),
    _ => profile,
  };
  Some(BrowserCandidate {
    id: id.into(),
    browser,
    profile,
    modified: SystemTime::UNIX_EPOCH,
    signed_in: false,
    app_bound: false,
    active_time: 0.0,
  })
}

fn default_browser(home: &Path) -> Option<BrowserKind> {
  #[cfg(target_os = "macos")]
  {
    let output =
      crate::runners::tool_process::command("/usr/bin/plutil")
        .ok()?
        .args(["-extract", "LSHandlers", "json", "-o", "-"])
        .arg(home.join(
          "Library/Preferences/com.apple.LaunchServices/com.apple.launchservices.secure.plist",
        ))
        .output()
        .ok()?;
    let handlers: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
    handlers
      .as_array()?
      .iter()
      .find(|h| h["LSHandlerURLScheme"] == "http")
      .and_then(|h| h["LSHandlerRoleAll"].as_str())
      .and_then(BrowserKind::from_default_id)
  }
  #[cfg(target_os = "windows")]
  {
    let _ = home;
    let output = crate::runners::tool_process::command("reg")
      .ok()?
      .args([
        "query",
        r"HKCU\Software\Microsoft\Windows\Shell\Associations\UrlAssociations\http\UserChoice",
        "/v",
        "ProgId",
      ])
      .output()
      .ok()?;
    BrowserKind::from_default_id(&String::from_utf8_lossy(&output.stdout))
  }
  #[cfg(target_os = "linux")]
  {
    let _ = home;
    let output = crate::runners::tool_process::command("xdg-settings")
      .ok()?
      .args(["get", "default-web-browser"])
      .output()
      .ok()?;
    BrowserKind::from_default_id(&String::from_utf8_lossy(&output.stdout))
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  fn root() -> PathBuf {
    let p = std::env::temp_dir().join(format!("nezzel-browser-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&p).unwrap();
    p
  }
  #[test]
  fn firefox_container_candidates_keep_profile_and_session_separate() {
    let temp = crate::runners::gallery_files::Staging::new().unwrap();
    let profile = temp.path.join(".mozilla/firefox/main");
    std::fs::create_dir_all(&profile).unwrap();
    std::fs::write(
      profile.join("containers.json"),
      r#"{"identities":[
      {"userContextId":1,"name":"Work","public":true},
      {"userContextId":2,"l10nID":"userContextPersonal.label","public":true},
      {"userContextId":3,"name":"Hidden","public":false}
    ]}"#,
    )
    .unwrap();
    let db = rusqlite::Connection::open(profile.join("cookies.sqlite")).unwrap();
    db.execute_batch("CREATE TABLE moz_cookies (host TEXT, name TEXT, expiry INTEGER, lastAccessed INTEGER, originAttributes TEXT);
      INSERT INTO moz_cookies VALUES ('.instagram.com', 'sessionid', 4102444800, 42, '^userContextId=1&privateBrowsingId=0');").unwrap();
    let found = discover_in(
      &BrowserEnv::test(Os::Linux).at(&temp.path),
      None,
      Some("instagram.com"),
    );
    let specs: Vec<_> = found
      .iter()
      .map(|c| {
        (
          c.id
            .strip_prefix(&format!("firefox:{}", profile.display()))
            .unwrap(),
          c.signed_in,
        )
      })
      .collect();
    assert_eq!(
      specs,
      [
        ("::Work", true),
        ("", false),
        ("::userContextPersonal.label", false)
      ]
    );
    assert_eq!(found[0].profile, "main · Work");
    assert_eq!(describe(&found[0].id).unwrap().profile, "main · Work");
    for contents in [
      "invalid json",
      r#"{"identities":[]}"#,
      r#"{"identities":[{"name":"none","userContextId":1},{"name":" Work","userContextId":2},{"name":"Duplicate","userContextId":3},{"name":"Duplicate","userContextId":4}]}"#,
    ] {
      std::fs::write(profile.join("containers.json"), contents).unwrap();
      let found = discover_in(
        &BrowserEnv::test(Os::Linux).at(&temp.path),
        None,
        Some("instagram.com"),
      );
      assert_eq!(found.len(), 1, "{contents}");
      assert!(
        !found[0].signed_in,
        "container session must not leak into the default: {contents}"
      );
    }
  }

  fn touch(root: &Path, rel: &str, seconds: u64) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    let f = std::fs::File::create(p).unwrap();
    f.set_times(
      std::fs::FileTimes::new()
        .set_modified(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(seconds)),
    )
    .unwrap();
  }
  /// Every browser key a default-browser id maps to can give a login.
  #[test]
  fn default_browser_ids_map_to_known_browser_keys() {
    for (raw, expected) in [
      ("com.google.chrome", Some("chrome")),
      ("org.mozilla.firefox", Some("firefox")),
      ("MSEdgeHTM", Some("edge")),
      ("microsoft-edge.desktop", Some("edge")),
      ("com.apple.Safari", Some("safari")),
      ("BraveHTML", Some("brave")),
      ("org.chromium.Chromium", Some("chromium")),
      ("Vivaldi.xyz", Some("vivaldi")),
      ("com.operasoftware.Opera", Some("opera")),
      ("com.naver.Whale", Some("whale")),
      ("naver-whale.desktop", Some("whale")),
      ("company.thebrowser.Browser", None),
      ("", None),
    ] {
      let kind = BrowserKind::from_default_id(raw);
      assert_eq!(kind.map(BrowserKind::name), expected, "{raw}");
      assert!(kind.is_none_or(BrowserKind::logins), "{raw}");
    }
  }

  #[test]
  fn consent_identity_folds_site_aliases() {
    use crate::runners::browser_cookies::site_key;
    for (host, key) in [
      ("youtu.be", Some("youtube.com")),
      ("www.youtube.com", Some("youtube.com")),
      ("v.redd.it", Some("reddit.com")),
      ("old.reddit.com", Some("reddit.com")),
      ("twitter.com", Some("x.com")),
      ("mobile.twitter.com", Some("x.com")),
      ("threads.com", Some("threads.net")),
      ("fb.watch", Some("fb.watch")),
      ("music.youtube.com", Some("youtube.com")),
      ("youtube-nocookie.com", Some("youtube.com")),
      ("YouTube.COM.", Some("youtube.com")),
      ("192.168.1.2", None),
      ("[::1]", None),
      ("", None),
    ] {
      assert_eq!(site_key(host).as_deref(), key, "{host}");
    }
  }

  #[test]
  fn site_session_cookie_names_and_registrable_domains() {
    use crate::runners::browser_cookies::{same_site, session_present};
    for (host, names, expected) in [
      ("www.instagram.com", "sessionid", true),
      ("instagram.com", "mid,ds_user_id,csrftoken", false),
      ("m.facebook.com", "c_user,xs", true),
      ("facebook.com", "c_user", false),
      ("x.com", "auth_token", true),
      ("twitter.com", "auth_token", true),
      ("tiktok.com", "sessionid", true),
      ("youtube.com", "SAPISID", true),
      ("youtu.be", "__Secure-3PSID", true),
      ("youtube.com", "LOGIN_INFO", true),
      ("reddit.com", "reddit_session", true),
      ("redd.it", "reddit_session", true),
      ("v.redd.it", "reddit_session", true),
      ("old.reddit.com", "reddit_session", true),
      ("linkedin.com", "li_at", true),
      ("pinterest.com", "_pinterest_sess", true),
      ("threads.net", "sessionid", true),
      ("threads.com", "sessionid", true),
      ("vimeo.com", "vimeo", true),
      ("snapchat.com", "sc-a-session", true),
      ("other.co.uk", "anything", false),
      ("other.co.uk", "", false),
    ] {
      let names = names
        .split(',')
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();
      assert_eq!(session_present(host, &names), expected, "{host}");
    }
    for (cookie, host, expected) in [
      (".instagram.com", "m.instagram.com", true),
      ("i.instagram.com", "www.instagram.com", true),
      ("instagram.com.attacker.test", "instagram.com", false),
      ("notinstagram.com", "instagram.com", false),
      ("media.example.co.uk", "www.example.co.uk", true),
      ("other.co.uk", "example.co.uk", false),
      ("one.github.io", "two.github.io", false),
      ("127.0.0.1", "127.0.0.1", false),
    ] {
      assert_eq!(same_site(cookie, host), expected, "{cookie}, {host}");
    }
  }

  fn metadata_db(path: &Path, firefox: bool, rows: &[(&str, &str, i64, i64, bool)]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let connection = rusqlite::Connection::open(path).unwrap();
    let (table, host, expiry, access) = if firefox {
      ("moz_cookies", "host", "expiry", "lastAccessed")
    } else {
      ("cookies", "host_key", "expires_utc", "last_access_utc")
    };
    // No value column. The encrypted prefix is only exposed by SQL as a boolean.
    connection.execute_batch(&format!("CREATE TABLE {table} ({host} TEXT, name TEXT, {expiry} INTEGER, {access} INTEGER, encrypted_value BLOB)")).unwrap();
    for (host, name, expiry, access, v20) in rows {
      connection
        .execute(
          &format!("INSERT INTO {table} VALUES (?1, ?2, ?3, ?4, ?5)"),
          rusqlite::params![
            host,
            name,
            expiry,
            access,
            if *v20 {
              b"v20-metadata-fixture".as_slice()
            } else {
              b"v10-metadata-fixture".as_slice()
            }
          ],
        )
        .unwrap();
    }
  }

  #[test]
  fn metadata_queries_ignore_expired_tracking_cookies_and_flag_windows_v20() {
    let now = 2_000_000_000_i64;
    for (firefox, os, name, future, session, v20, signed, blocked) in [
      (false, Os::Mac, "sessionid", true, false, false, true, false),
      (
        false,
        Os::Mac,
        "sessionid",
        false,
        false,
        false,
        false,
        false,
      ),
      (false, Os::Mac, "sessionid", false, true, false, true, false),
      (
        true,
        Os::Linux,
        "sessionid",
        true,
        false,
        false,
        true,
        false,
      ),
      (
        true,
        Os::Linux,
        "sessionid",
        false,
        false,
        false,
        false,
        false,
      ),
      (
        true,
        Os::Linux,
        "sessionid",
        false,
        true,
        false,
        false,
        false,
      ),
      (
        false,
        Os::Windows,
        "sessionid",
        true,
        false,
        true,
        true,
        true,
      ),
      (
        false,
        Os::Windows,
        "sessionid",
        true,
        false,
        false,
        true,
        false,
      ),
      (
        false,
        Os::Windows,
        "csrftoken",
        true,
        false,
        true,
        false,
        false,
      ),
      (false, Os::Windows, "mid", true, false, true, false, false),
      (false, Os::Mac, "sessionid", true, false, true, true, false),
    ] {
      let root = crate::runners::gallery_files::Staging::new().unwrap();
      let path = root.path.join("Cookies");
      let unix_expiry = now + if future { 100 } else { -100 };
      let expires = if session {
        0
      } else if firefox {
        unix_expiry
      } else {
        (unix_expiry + 11_644_473_600) * 1_000_000
      };
      let access = if firefox {
        900_000
      } else {
        11_644_473_600_900_000
      };
      metadata_db(
        &path,
        firefox,
        &[(".instagram.com", name, expires, access, v20)],
      );
      let before = std::fs::read(&path).unwrap();
      let hint = cookie_hint(
        &BrowserEnv::test(os),
        &path,
        if firefox {
          BrowserKind::Firefox
        } else {
          BrowserKind::Chrome
        },
        "www.instagram.com",
        now,
      );
      assert_eq!((hint.signed_in, hint.app_bound), (signed, blocked));
      if signed {
        assert_eq!(hint.last_access, 900_000);
      }
      assert_eq!(std::fs::read(&path).unwrap(), before);
    }
    for (expires, signed) in [
      (now + 100, true),
      (now - 100, false),
      ((now + 100) * 1000, true),
      ((now - 100) * 1000, false),
    ] {
      let root = crate::runners::gallery_files::Staging::new().unwrap();
      let path = root.path.join("cookies.sqlite");
      metadata_db(
        &path,
        true,
        &[(".instagram.com", "sessionid", expires, 900_000, false)],
      );
      assert_eq!(
        cookie_hint(
          &BrowserEnv::test(Os::Linux),
          &path,
          BrowserKind::Firefox,
          "instagram.com",
          now
        )
        .signed_in,
        signed,
        "expiry {expires}"
      );
    }
    let root = crate::runners::gallery_files::Staging::new().unwrap();
    let path = root.path.join("Cookies");
    metadata_db(
      &path,
      false,
      &[
        (".threads.net", "mid", 0, 0, false),
        (".instagram.com", "sessionid", 0, 0, false),
      ],
    );
    assert!(
      cookie_hint(
        &BrowserEnv::test(Os::Mac),
        &path,
        BrowserKind::Chrome,
        "www.threads.com",
        now
      )
      .signed_in
    );
    let live_path = root.path.join("LiveCookies");
    metadata_db(&live_path, false, &[(".instagram.com", "mid", 0, 0, false)]);
    let live = rusqlite::Connection::open(&live_path).unwrap();
    live.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; UPDATE cookies SET name='sessionid';").unwrap();
    let before = std::fs::read(&live_path).unwrap();
    assert!(
      cookie_hint(
        &BrowserEnv::test(Os::Mac),
        &live_path,
        BrowserKind::Chrome,
        "instagram.com",
        now
      )
      .signed_in
    );
    assert_eq!(std::fs::read(&live_path).unwrap(), before);
  }

  #[test]
  fn reddit_alias_discovery_finds_the_signed_in_profile_without_reading_cookie_values() {
    let now = 2_000_000_000_i64;
    for firefox in [false, true] {
      let root = crate::runners::gallery_files::Staging::new().unwrap();
      let path = root.path.join("Cookies");
      metadata_db(
        &path,
        firefox,
        &[(
          ".reddit.com",
          "reddit_session",
          if firefox { now + 100 } else { 0 },
          900_000,
          false,
        )],
      );
      let before = std::fs::read(&path).unwrap();
      for host in ["redd.it", "v.redd.it", "old.reddit.com", "www.reddit.com"] {
        assert!(
          cookie_hint(
            &BrowserEnv::test(Os::Mac),
            &path,
            if firefox {
              BrowserKind::Firefox
            } else {
              BrowserKind::Chrome
            },
            host,
            now
          )
          .signed_in,
          "{host}"
        );
      }
      assert!(
        !cookie_hint(
          &BrowserEnv::test(Os::Mac),
          &path,
          BrowserKind::Chrome,
          "reddit.com.attacker.test",
          now
        )
        .signed_in
      );
      assert_eq!(std::fs::read(&path).unwrap(), before);
    }
  }

  #[test]
  fn locked_windows_cookie_copy_uses_immutable_or_reports_browser_protection() {
    use std::cell::RefCell;
    thread_local! {
      static SNAPSHOTS: RefCell<Vec<PathBuf>> = const { RefCell::new(Vec::new()) };
    }
    let root = crate::runners::gallery_files::Staging::new().unwrap();
    let path = root.path.join("Profile space #1").join("Cookies");
    metadata_db(&path, false, &[(".instagram.com", "sessionid", 0, 0, true)]);
    let before = std::fs::read(&path).unwrap();
    let recording = BrowserEnv {
      copy_file: |from, to| {
        SNAPSHOTS.with_borrow_mut(|snapshots| snapshots.push(to.parent().unwrap().to_owned()));
        std::fs::copy(from, to)
      },
      ..BrowserEnv::test(Os::Windows)
    };
    let denied = BrowserEnv {
      copy_file: |_, _| Err(std::io::ErrorKind::PermissionDenied.into()),
      ..BrowserEnv::test(Os::Windows)
    };
    let hint = cookie_hint(
      &recording,
      &path,
      BrowserKind::Chrome,
      "instagram.com",
      2_000_000_000,
    );
    let snapshots = SNAPSHOTS.take();
    assert!(hint.signed_in && hint.app_bound);
    assert!(!snapshots.is_empty());
    assert!(snapshots.iter().all(|snapshot| !snapshot.exists()));
    let hint = cookie_hint(
      &denied,
      &path,
      BrowserKind::Chrome,
      "instagram.com",
      2_000_000_000,
    );
    assert!(hint.signed_in && hint.app_bound);
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(
      std::fs::read_dir(path.parent().unwrap()).unwrap().count(),
      1
    );
    std::fs::write(&path, b"unreadable database fixture").unwrap();
    let hint = cookie_hint(
      &denied,
      &path,
      BrowserKind::Chrome,
      "instagram.com",
      2_000_000_000,
    );
    assert!(!hint.signed_in && !hint.app_bound);
  }

  #[test]
  fn windows_visitor_cookies_do_not_offer_a_login() {
    for (host, rows, signed, blocked) in [
      (
        "reddit.com",
        vec![("loid", true), ("csrf_token", true)],
        false,
        false,
      ),
      (
        "instagram.com",
        vec![("csrftoken", true), ("mid", true)],
        false,
        false,
      ),
      ("reddit.com", vec![("reddit_session", true)], true, true),
      (
        "reddit.com",
        vec![("reddit_session", false), ("loid", true)],
        true,
        false,
      ),
      (
        "instagram.com",
        vec![("sessionid", false), ("mid", true)],
        true,
        false,
      ),
    ] {
      let root = crate::runners::gallery_files::Staging::new().unwrap();
      let path = root
        .path
        .join("local/Microsoft/Edge/User Data/Default/Network/Cookies");
      let rows: Vec<_> = rows
        .iter()
        .map(|(name, v20)| (host, *name, 0, 0, *v20))
        .collect();
      metadata_db(&path, false, &rows);
      let env = BrowserEnv {
        local: root.path.join("local"),
        ..BrowserEnv::test(Os::Windows).at(&root.path)
      };
      let found = discover_in(&env, None, Some(host));
      assert_eq!(found.len(), usize::from(signed), "{host}: {rows:?}");
      if signed {
        assert_eq!((found[0].signed_in, found[0].app_bound), (signed, blocked));
      }
    }
    // A site without known session names keeps the old rule: any v20 row marks the jar unreadable.
    let root = crate::runners::gallery_files::Staging::new().unwrap();
    let path = root
      .path
      .join("local/Microsoft/Edge/User Data/Default/Network/Cookies");
    metadata_db(&path, false, &[("twitch.tv", "auth-token", 0, 0, true)]);
    let env = BrowserEnv {
      local: root.path.join("local"),
      ..BrowserEnv::test(Os::Windows).at(&root.path)
    };
    let found = discover_in(&env, None, Some("twitch.tv"));
    assert!(
      !found.is_empty() && found.iter().all(|c| c.app_bound),
      "{found:?}"
    );
  }

  #[test]
  fn signed_chromium_access_time_precedes_firefox_and_windows_v20_is_last() {
    let root = crate::runners::gallery_files::Staging::new().unwrap();
    let local = root.path.join("local");
    let roaming = root.path.join("roaming");
    for (relative, firefox, access, v20) in [
      (
        "local/Google/Chrome/User Data/Default/Network/Cookies",
        false,
        100,
        false,
      ),
      (
        "local/BraveSoftware/Brave-Browser/User Data/Default/Cookies",
        false,
        200,
        false,
      ),
      (
        "roaming/Mozilla/Firefox/Profiles/main/cookies.sqlite",
        true,
        500,
        false,
      ),
      (
        "local/Microsoft/Edge/User Data/Default/Network/Cookies",
        false,
        600,
        true,
      ),
    ] {
      let access = if firefox {
        access
      } else {
        access + 11_644_473_600_000_000
      };
      metadata_db(
        &root.path.join(relative),
        firefox,
        &[(
          ".instagram.com",
          "sessionid",
          if firefox { 4_102_444_800 } else { 0 },
          access,
          v20,
        )],
      );
    }
    metadata_db(
      &root
        .path
        .join("local/Microsoft/Edge/User Data/Default/Cookies"),
      false,
      &[(
        ".instagram.com",
        "sessionid",
        0,
        11_644_473_600_000_900,
        false,
      )],
    );
    let env = BrowserEnv {
      local: local.clone(),
      roaming: roaming.clone(),
      default: Some(BrowserKind::Firefox),
      ..BrowserEnv::test(Os::Windows).at(&root.path)
    };
    let found = discover_in(&env, None, Some("instagram.com"));
    assert_eq!(
      found.iter().map(|c| c.browser.name()).collect::<Vec<_>>(),
      ["brave", "chrome", "firefox", "edge"]
    );
    assert!(found.last().unwrap().app_bound);
    assert!(found.iter().all(|c| c.signed_in));
  }

  #[test]
  fn chromium_menu_names_and_collisions() {
    let p = root();
    let base = "Library/Application Support/Google/Chrome";
    let cases = [
      (
        "Default",
        serde_json::json!({"name":"Your Chrome", "gaia_given_name":"Ahmed", "is_using_default_name":true, "active_time":10, "user_name":"private@example.com"}),
        "Ahmed · Default",
      ),
      (
        "Profile 1",
        serde_json::json!({"name":"souvi", "gaia_given_name":""}),
        "souvi",
      ),
      (
        "Profile 4",
        serde_json::json!({"name":"Ahmed", "gaia_given_name":""}),
        "Ahmed · Profile 4",
      ),
      (
        "Profile 5",
        serde_json::json!({"name":"groupmbi and cacmauritania", "gaia_given_name":"MBI"}),
        "MBI (groupmbi and cacmauritania)",
      ),
      (
        "Profile 6",
        serde_json::json!({"name":"hapa", "gaia_given_name":"hapa"}),
        "hapa",
      ),
      (
        "Profile 7",
        serde_json::json!({"name":"Person 1", "gaia_given_name":"chat", "is_using_default_name":true}),
        "chat",
      ),
      (
        "Profile 8",
        serde_json::json!({"name":"Person 2", "gaia_given_name":"", "gaia_name":"Full Name", "is_using_default_name":true}),
        "Full Name",
      ),
      (
        "Profile 9",
        serde_json::json!({"name":"Person 3", "is_using_default_name":true}),
        "Person 3",
      ),
      ("Profile 10", serde_json::json!({}), "Profile 10"),
    ];
    let mut cache = serde_json::Map::new();
    for (dir, metadata, _) in &cases {
      touch(&p, &format!("{base}/{dir}/Cookies"), 1);
      cache.insert((*dir).into(), metadata.clone());
    }
    std::fs::write(
      p.join(base).join("Local State"),
      serde_json::json!({"profile":{"info_cache":cache}}).to_string(),
    )
    .unwrap();
    let found = discover_in(&BrowserEnv::test(Os::Mac).at(&p), None, None);
    for (dir, _, expected) in cases {
      let candidate = found
        .iter()
        .find(|c| c.id.ends_with(&format!("/{dir}")))
        .unwrap();
      assert_eq!(candidate.profile, expected);
    }
    std::fs::remove_dir_all(p).unwrap();
  }
  fn cookie_db(path: &Path, table: &str, column: &str, host: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    // No plaintext value column: the detector must only project session metadata.
    let connection = rusqlite::Connection::open(path).unwrap();
    let (expiry, access) = if table == "moz_cookies" {
      ("expiry", "lastAccessed")
    } else {
      ("expires_utc", "last_access_utc")
    };
    connection
      .execute_batch(&format!(
        "CREATE TABLE {table} ({column} TEXT, name TEXT, {expiry} INTEGER, {access} INTEGER);"
      ))
      .unwrap();
    connection
      .execute(
        &format!(
          "INSERT INTO {table} VALUES (?1, 'sessionid', {}, 0)",
          if table == "moz_cookies" {
            4_102_444_800_i64
          } else {
            0
          }
        ),
        [host],
      )
      .unwrap();
  }
  #[test]
  fn host_cookie_hint_ranks_before_remembered_then_active_time() {
    let p = root();
    let base = p.join("Library/Application Support/Google/Chrome");
    let cache = serde_json::json!({"profile":{"info_cache":{
      "Default":{"name":"Recent", "active_time":90},
      "Profile 1":{"name":"Signed in", "active_time":1},
      "Profile 2":{"name":"Remembered", "active_time":100},
      "Profile 3":{"name":"Older", "active_time":2}
    }}});
    for (dir, host) in [
      ("Default", "unrelated.test"),
      ("Profile 1", ".instagram.com"),
      ("Profile 2", "notinstagram.com"),
      ("Profile 3", "instagram.com.attacker.test"),
    ] {
      cookie_db(
        &base.join(dir).join("Network/Cookies"),
        "cookies",
        "host_key",
        host,
      );
    }
    std::fs::write(base.join("Local State"), cache.to_string()).unwrap();
    let remembered = format!("chrome:{}", base.join("Profile 2").display());
    let db = base.join("Profile 1/Network/Cookies");
    let before = std::fs::read(&db).unwrap();
    let env = BrowserEnv {
      default: Some(BrowserKind::Firefox),
      ..BrowserEnv::test(Os::Mac).at(&p)
    };
    let found = discover_in(&env, Some(&remembered), Some("www.instagram.com"));
    assert_eq!(
      found
        .iter()
        .map(|c| (c.profile.as_str(), c.signed_in))
        .collect::<Vec<_>>(),
      [
        ("Signed in", true),
        ("Remembered", false),
        ("Recent", false),
        ("Older", false)
      ]
    );
    assert_eq!(std::fs::read(db).unwrap(), before);
    assert!(!has_host_cookies(
      &base.join("absent"),
      BrowserKind::Chrome,
      "instagram.com"
    ));
    std::fs::write(base.join("broken"), "not sqlite").unwrap();
    assert!(!has_host_cookies(
      &base.join("broken"),
      BrowserKind::Chrome,
      "instagram.com"
    ));
    assert!(!has_host_cookies(
      &base.join("Default/Network/Cookies"),
      BrowserKind::Chrome,
      "%' OR 1=1;--"
    ));
    std::fs::remove_dir_all(p).unwrap();
  }
  #[test]
  fn firefox_ini_names_and_host_hints_include_custom_profile_paths() {
    for (os, relative, profile) in [
      (
        Os::Mac,
        "Library/Application Support/Firefox",
        "Profiles/random.default",
      ),
      (
        Os::Windows,
        "roaming/Mozilla/Firefox",
        "Profiles/random.default",
      ),
      (Os::Linux, ".mozilla/firefox", "random.default"),
      (
        Os::Linux,
        "snap/firefox/common/.mozilla/firefox",
        "random.default",
      ),
      (
        Os::Linux,
        ".var/app/org.mozilla.firefox/.mozilla/firefox",
        "random.default",
      ),
    ] {
      let root = crate::runners::gallery_files::Staging::new().unwrap();
      let p = &root.path;
      let base = p.join(relative);
      cookie_db(
        &base.join(profile).join("cookies.sqlite"),
        "moz_cookies",
        "host",
        ".instagram.com",
      );
      let custom = p.join("external-custom");
      cookie_db(
        &custom.join("cookies.sqlite"),
        "moz_cookies",
        "host",
        "other.test",
      );
      std::fs::write(base.join("profiles.ini"), format!("[Profile0]\nName=Ahmed.work\nIsRelative=1\nPath={profile}\n\n[Profile1]\nName=Custom\nIsRelative=0\nPath={}\n", custom.display())).unwrap();
      let env = BrowserEnv {
        local: p.join("local"),
        roaming: p.join("roaming"),
        ..BrowserEnv::test(os).at(p)
      };
      let found = discover_in(&env, None, Some("instagram.com"));
      assert_eq!(
        found
          .iter()
          .map(|c| (c.profile.as_str(), c.signed_in))
          .collect::<Vec<_>>(),
        [("Ahmed.work", true), ("Custom", false)]
      );
      assert!(found.iter().all(|c| c
        .id
        .split_once(':')
        .is_some_and(|(_, path)| Path::new(path).is_absolute())));
    }
  }
  #[test]
  fn mac_default_then_browser_order_newest_profile_and_remembered() {
    let p = root();
    touch(
      &p,
      "Library/Application Support/Firefox/Profiles/a/cookies.sqlite",
      1,
    );
    touch(
      &p,
      "Library/Application Support/Google/Chrome/Default/Cookies",
      2,
    );
    touch(
      &p,
      "Library/Application Support/Google/Chrome/Profile 1/Network/Cookies",
      3,
    );
    let env = BrowserEnv {
      default: Some(BrowserKind::Chrome),
      ..BrowserEnv::test(Os::Mac).at(&p)
    };
    let found = discover_in(&env, None, None);
    assert_eq!(
      found.iter().map(|c| c.profile.as_str()).collect::<Vec<_>>(),
      ["Profile 1", "Default", "a"]
    );
    let remembered = &found[2].id;
    assert_eq!(
      discover_in(&env, Some(remembered), None)[0].browser,
      BrowserKind::Firefox
    );
    std::fs::remove_dir_all(p).unwrap();
  }
  #[test]
  fn chromium_family_roots_use_local_state_names() {
    for (os, base, browser, profile) in [
      (
        Os::Mac,
        "Library/Application Support/Microsoft Edge",
        "edge",
        "Default",
      ),
      (
        Os::Mac,
        "Library/Application Support/BraveSoftware/Brave-Browser",
        "brave",
        "Default",
      ),
      (
        Os::Mac,
        "Library/Application Support/Google/Chrome",
        "chrome",
        "Default",
      ),
      (
        Os::Mac,
        "Library/Application Support/Chromium",
        "chromium",
        "Default",
      ),
      (
        Os::Mac,
        "Library/Application Support/Vivaldi",
        "vivaldi",
        "Default",
      ),
      (
        Os::Mac,
        "Library/Application Support/com.operasoftware.Opera",
        "opera",
        "",
      ),
      (Os::Windows, "Google/Chrome/User Data", "chrome", "Default"),
      (Os::Windows, "Microsoft/Edge/User Data", "edge", "Default"),
      (
        Os::Windows,
        "BraveSoftware/Brave-Browser/User Data",
        "brave",
        "Default",
      ),
      (Os::Windows, "Chromium/User Data", "chromium", "Default"),
      (Os::Linux, ".config/google-chrome", "chrome", "Default"),
      (Os::Linux, ".config/microsoft-edge", "edge", "Default"),
      (
        Os::Linux,
        ".config/BraveSoftware/Brave-Browser",
        "brave",
        "Default",
      ),
      (Os::Linux, ".config/vivaldi", "vivaldi", "Default"),
      (Os::Linux, ".config/opera", "opera", ""),
      (
        Os::Linux,
        "snap/chromium/common/chromium",
        "chromium",
        "Default",
      ),
      (
        Os::Linux,
        "snap/chromium/common/.config/chromium",
        "chromium",
        "Default",
      ),
      (
        Os::Linux,
        ".var/app/org.chromium.Chromium/config/chromium",
        "chromium",
        "Default",
      ),
      (
        Os::Linux,
        ".var/app/com.google.Chrome/config/google-chrome",
        "chrome",
        "Default",
      ),
      (
        Os::Linux,
        ".var/app/com.microsoft.Edge/config/microsoft-edge",
        "edge",
        "Default",
      ),
      (
        Os::Linux,
        ".var/app/com.brave.Browser/config/BraveSoftware/Brave-Browser",
        "brave",
        "Default",
      ),
      (
        Os::Linux,
        ".var/app/com.vivaldi.Vivaldi/config/vivaldi",
        "vivaldi",
        "Default",
      ),
      (
        Os::Linux,
        ".var/app/com.opera.Opera/config/opera",
        "opera",
        "",
      ),
      (Os::Linux, ".config/chromium", "chromium", "Default"),
      (Os::Windows, "Vivaldi/User Data", "vivaldi", "Default"),
      (Os::Windows, "Opera Software/Opera Stable", "opera", ""),
      (Os::Windows, "Opera Software/Opera GX Stable", "opera", ""),
      (
        Os::Mac,
        "Library/Application Support/com.operasoftware.OperaGX",
        "opera",
        "",
      ),
      (
        Os::Mac,
        "Library/Application Support/Naver/Whale",
        "whale",
        "Default",
      ),
      (
        Os::Windows,
        "Naver/Naver Whale/User Data",
        "whale",
        "Default",
      ),
      (Os::Linux, ".config/naver-whale", "whale", "Default"),
    ] {
      let p = root();
      let base = p.join(base);
      let profile_dir = if profile.is_empty() {
        base.clone()
      } else {
        base.join(profile)
      };
      std::fs::create_dir_all(&profile_dir).unwrap();
      std::fs::write(profile_dir.join("Cookies"), "").unwrap();
      std::fs::write(base.join("Local State"), r#"{"profile":{"info_cache":{"Default":{"name":"Person 1","gaia_given_name":"chat","is_using_default_name":true}}}}"#).unwrap();
      let found = discover_in(&BrowserEnv::test(os).at(&p), None, None);
      assert_eq!(
        found
          .iter()
          .find(|c| c.id == format!("{browser}:{}", profile_dir.display()))
          .unwrap()
          .profile,
        "chat"
      );
      std::fs::remove_dir_all(p).unwrap();
    }
  }
  #[test]
  fn windows_and_linux_find_only_existing_cookie_databases() {
    let p = root();
    touch(
      &p,
      "local/Microsoft/Edge/User Data/Default/Network/Cookies",
      1,
    );
    touch(&p, "roaming/Mozilla/Firefox/Profiles/p/cookies.sqlite", 1);
    let env = BrowserEnv {
      local: p.join("local"),
      roaming: p.join("roaming"),
      default: Some(BrowserKind::Edge),
      ..BrowserEnv::test(Os::Windows).at(&p)
    };
    let found = discover_in(&env, None, None);
    assert_eq!(
      found.iter().map(|c| c.browser.name()).collect::<Vec<_>>(),
      ["edge", "firefox"]
    );
    touch(&p, ".config/google-chrome/Default/Cookies", 1);
    touch(&p, ".mozilla/firefox/default/cookies.sqlite", 2);
    assert_eq!(
      discover_in(&BrowserEnv::test(Os::Linux).at(&p), None, None)
        .iter()
        .map(|c| c.browser.name())
        .collect::<Vec<_>>(),
      ["firefox", "chrome"]
    );
    assert!(discover_in(
      &BrowserEnv {
        home: p.join("missing"),
        ..BrowserEnv::test(Os::Linux).at(&p)
      },
      None,
      None
    )
    .is_empty());
    std::fs::remove_dir_all(p).unwrap();
  }
}
