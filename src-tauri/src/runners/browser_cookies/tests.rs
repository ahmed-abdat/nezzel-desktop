//! Cookie export tests: scoping, consent, cleanup and the exporter process.
use super::jar::{
  filter_export, private_directory, private_file, sweep_directory, sweep_named_directories,
  COOKIE_HEADER,
};
use super::*;
use crate::runners::gallery_files::Staging;
use crate::state::config_models::AuthSettings;
use rusqlite::Connection;
use std::io::Write;
use std::path::{Path, PathBuf};
use tokio::sync::watch;

fn context(root: &Path) -> CookieContext {
  CookieContext {
    exporter: root.join("must-not-run"),
    directory: root.join("scoped"),
  }
}
fn firefox_fixture(root: &Path) -> String {
  let profile = root.join("profile");
  std::fs::create_dir(&profile).unwrap();
  let db = Connection::open(profile.join("cookies.sqlite")).unwrap();
  db.execute_batch("CREATE TABLE moz_cookies (host TEXT, name TEXT, value TEXT, path TEXT, expiry INTEGER, isSecure INTEGER, isHttpOnly INTEGER, originAttributes TEXT);").unwrap();
  for host in [
    ".reddit.com",
    "www.reddit.com",
    "google.com",
    "bank.example",
    "reddit.com.evil.example",
    "notreddit.com",
    ".instagram.com",
    "threads.net",
    "threads.com",
    "www.instagram.com.evil.example",
    ".x.com",
    ".twitter.com",
    "eviltwitter.com",
  ] {
    db.execute(
      "INSERT INTO moz_cookies VALUES (?1, 'fixture', 'synthetic', '/', 4102444800, 1, 1, '')",
      [host],
    )
    .unwrap();
  }
  format!("firefox:{}", profile.display())
}
fn domains(path: &Path) -> Vec<String> {
  let contents = std::fs::read_to_string(path).unwrap();
  let mut domains: Vec<_> = contents
    .lines()
    .filter_map(|line| {
      line
        .strip_prefix("#HttpOnly_")
        .or_else(|| (!line.starts_with('#') && !line.is_empty()).then_some(line))
    })
    .map(|line| line.split('\t').next().unwrap().to_owned())
    .collect();
  domains.sort();
  domains
}

#[tokio::test]
async fn scoped_cookie_files_restrict_both_engines_and_are_removed_after_use() {
  let root = Staging::new().unwrap();
  let browser = firefox_fixture(&root.path);
  let context = context(&root.path);
  let (_tx, cancel) = watch::channel(true);
  for (host, expected) in [
    ("www.reddit.com", vec![".reddit.com", "www.reddit.com"]),
    (
      "threads.com",
      vec![".instagram.com", "threads.com", "threads.net"],
    ),
    ("x.com", vec![".twitter.com", ".x.com"]),
    ("twitter.com", vec![".twitter.com", ".x.com"]),
  ] {
    let access = CookieAccess::prepare(
      &AuthSettings::default(),
      Some(&browser),
      &format!("https://{host}/post"),
      &context,
      cancel.clone(),
    )
    .await
    .unwrap();
    assert_eq!(access.args[0], "--cookies");
    assert!(!access
      .args
      .iter()
      .any(|arg| arg == "--cookies-from-browser"));
    let path = PathBuf::from(&access.args[1]);
    assert_eq!(domains(&path), expected);
    assert_eq!(
      std::fs::read_dir(path.parent().unwrap()).unwrap().count(),
      1,
      "only the scoped jar survives preparation"
    );
    #[cfg(unix)]
    {
      use std::os::unix::fs::PermissionsExt;
      assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
      );
    }
    let gallery = crate::runners::gallery::arguments(&access.args, Path::new("config.json"));
    assert!(gallery.windows(2).any(|args| args == access.args));
    assert!(!gallery.iter().any(|arg| arg == "--cookies-from-browser"));
    drop(access);
    assert!(!path.exists());
  }
  assert_eq!(std::fs::read_dir(context.directory).unwrap().count(), 0);
}

#[tokio::test]
async fn declined_consent_creates_no_file_and_manual_cookie_file_is_unchanged() {
  let root = Staging::new().unwrap();
  let context = context(&root.path);
  let settings = AuthSettings::default();
  let (_tx, cancel) = watch::channel(true);
  for browser in [None, Some(super::super::login::ANONYMOUS_BROWSER)] {
    let args = CookieAccess::prepare(
      &settings,
      browser,
      "https://reddit.com/post",
      &context,
      cancel.clone(),
    )
    .await
    .unwrap();
    assert!(args.args.is_empty());
    assert!(!context.directory.exists());
  }
  let manual = root.path.join("manual.txt");
  std::fs::write(&manual, b"user-managed jar").unwrap();
  let settings = AuthSettings {
    cookie_file: Some(manual.to_string_lossy().into_owned()),
    ..Default::default()
  };
  let args = CookieAccess::prepare(&settings, None, "https://reddit.com/post", &context, cancel)
    .await
    .unwrap();
  assert_eq!(args.args, ["--cookies", manual.to_str().unwrap()]);
  drop(args);
  assert_eq!(std::fs::read(manual).unwrap(), b"user-managed jar");
  assert!(!context.directory.exists());
}

#[tokio::test]
async fn unreadable_firefox_never_falls_back_and_removes_partial_exports() {
  let root = Staging::new().unwrap();
  let context = context(&root.path);
  let browser = format!("firefox:{}", root.path.display());
  let (_tx, cancel) = watch::channel(true);
  assert!(CookieAccess::prepare(
    &AuthSettings::default(),
    Some(&browser),
    "https://reddit.com/post",
    &context,
    cancel
  )
  .await
  .is_err());
  assert_eq!(std::fs::read_dir(context.directory).unwrap().count(), 0);
}

#[test]
fn filtering_removes_the_full_export_on_success_and_parse_error_and_sweep_removes_stale_jars() {
  let root = Staging::new().unwrap();
  for invalid in [false, true] {
    let full = root.path.join("full.txt");
    std::fs::write(&full, if invalid { "malformed jar\n" } else { "# Netscape HTTP Cookie File\n#HttpOnly_.reddit.com\tTRUE\t/\tTRUE\t4102444800\tfixture\tsynthetic\ngoogle.com\tFALSE\t/\tTRUE\t4102444800\tfixture\tsynthetic\n" }).unwrap();
    let path = root.path.join(format!("scoped-{invalid}"));
    let mut scoped = private_file(&path).unwrap();
    scoped.write_all(COOKIE_HEADER.as_bytes()).unwrap();
    assert_eq!(
      filter_export(&full, "reddit.com", &mut scoped).is_err(),
      invalid
    );
    assert!(!full.exists());
    if !invalid {
      assert_eq!(domains(&path), [".reddit.com"]);
    }
  }
  let stale = root.path.join(format!("attempt-{}", uuid::Uuid::new_v4()));
  private_directory(&stale).unwrap();
  std::fs::write(stale.join("full.txt"), b"synthetic").unwrap();
  let snapshot = root
    .path
    .join(format!("nezzel-images-{}", uuid::Uuid::new_v4()));
  private_directory(&snapshot).unwrap();
  std::fs::write(snapshot.join("cookies.sqlite"), b"synthetic snapshot").unwrap();
  let keep = root.path.join("unrelated.txt");
  std::fs::write(&keep, b"keep").unwrap();
  sweep_directory(&root.path).unwrap();
  assert!(!stale.exists());
  sweep_named_directories(&root.path, "nezzel-images-").unwrap();
  assert!(
    !snapshot.exists(),
    "aborted discovery snapshots must be swept"
  );
  assert!(keep.exists());
  #[cfg(unix)]
  {
    use std::os::unix::fs::PermissionsExt;
    let other = root
      .path
      .join(format!("nezzel-images-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&other).unwrap();
    std::fs::set_permissions(&other, std::fs::Permissions::from_mode(0o755)).unwrap();
    sweep_named_directories(&root.path, "nezzel-images-").unwrap();
    assert!(
      other.exists(),
      "shared temp entries that are not private must be left alone"
    );
  }
}

#[cfg(unix)]
fn exporter_fixture(root: &Path, mode: &str) -> CookieContext {
  use std::os::unix::fs::PermissionsExt;
  let exporter = root.join("exporter");
  std::fs::write(&exporter, format!(r#"#!/bin/sh
set -eu
full=''
info=''
while [ "$#" -gt 0 ]; do
  case "$1" in
    --cookies) full="$2"; shift 2 ;;
    --cookies-from-browser) shift 2 ;;
    --load-info-json) info="$2"; shift 2 ;;
    --ignore-config|--no-cache-dir|--skip-download|--no-clean-info-json) shift ;;
    *) exit 90 ;;
  esac
done
[ -f "$info" ]
printf '%s\n' '# Netscape HTTP Cookie File' '.reddit.com	TRUE	/	TRUE	4102444800	fixture	synthetic' 'google.com	FALSE	/	TRUE	4102444800	fixture	synthetic' > "$full"
case '{mode}' in
  error) exit 1 ;;
  wait) exec /usr/bin/tail -f /dev/null ;;
esac
"#)).unwrap();
  std::fs::set_permissions(&exporter, std::fs::Permissions::from_mode(0o700)).unwrap();
  CookieContext {
    exporter,
    directory: root.join("scoped"),
  }
}

#[cfg(unix)]
#[tokio::test]
async fn chromium_exporter_passes_only_scoped_args_and_erases_full_jar() {
  let root = Staging::new().unwrap();
  let context = exporter_fixture(&root.path, "success");
  let (_tx, cancel) = watch::channel(true);
  let access = CookieAccess::prepare(
    &AuthSettings::default(),
    Some("chrome:/synthetic"),
    "https://reddit.com/post",
    &context,
    cancel,
  )
  .await
  .unwrap();
  assert_eq!(access.args[0], "--cookies");
  let path = PathBuf::from(&access.args[1]);
  assert_eq!(domains(&path), [".reddit.com"]);
  assert!(!path.with_file_name("full.txt").exists());
  assert_eq!(
    std::fs::read_dir(path.parent().unwrap()).unwrap().count(),
    1
  );
  let child = access.clone();
  assert_eq!(access.args, child.args);
  drop(access);
  assert!(path.exists());
  drop(child);
  assert!(!path.exists());
}

#[cfg(unix)]
#[tokio::test]
async fn cancelling_an_active_export_removes_the_full_and_scoped_files() {
  let root = Staging::new().unwrap();
  let context = exporter_fixture(&root.path, "wait");
  let (tx, cancel) = watch::channel(true);
  let settings = AuthSettings::default();
  let prepare = CookieAccess::prepare(
    &settings,
    Some("chrome:/synthetic"),
    "https://reddit.com/post",
    &context,
    cancel,
  );
  tokio::pin!(prepare);
  let cancel_after_write = async {
    loop {
      let exported = std::fs::read_dir(&context.directory)
        .into_iter()
        .flatten()
        .flatten()
        .any(|entry| {
          std::fs::metadata(entry.path().join("full.txt"))
            .is_ok_and(|metadata| metadata.len() > COOKIE_HEADER.len() as u64)
        });
      if exported {
        tx.send(false).unwrap();
        break;
      }
      tokio::task::yield_now().await;
    }
  };
  tokio::select! {
    result = &mut prepare => { assert!(result.is_ok(), "export returned before cancellation"); panic!("export must wait for cancellation"); },
    () = cancel_after_write => { assert!(matches!(prepare.await, Err("cancelled"))); },
  }
  assert_eq!(std::fs::read_dir(&context.directory).unwrap().count(), 0);
}

#[cfg(unix)]
#[tokio::test]
async fn exporter_failure_removes_every_cookie_file() {
  let root = Staging::new().unwrap();
  let context = exporter_fixture(&root.path, "error");
  let (_tx, cancel) = watch::channel(true);
  let result = CookieAccess::prepare(
    &AuthSettings::default(),
    Some("chrome:/synthetic"),
    "https://reddit.com/post",
    &context,
    cancel,
  )
  .await;
  assert!(matches!(result, Err(COOKIE_READ_ERROR)));
  assert_eq!(std::fs::read_dir(&context.directory).unwrap().count(), 0);
}

#[cfg(unix)]
#[tokio::test(start_paused = true)]
async fn exporter_timeout_removes_every_cookie_file() {
  let root = Staging::new().unwrap();
  let context = exporter_fixture(&root.path, "wait");
  let (_tx, cancel) = watch::channel(true);
  let result = CookieAccess::prepare(
    &AuthSettings::default(),
    Some("chrome:/synthetic"),
    "https://reddit.com/post",
    &context,
    cancel,
  )
  .await;
  assert!(matches!(
    result,
    Err(crate::extract::deadline::FETCH_TIMEOUT)
  ));
  assert_eq!(std::fs::read_dir(&context.directory).unwrap().count(), 0);
}
