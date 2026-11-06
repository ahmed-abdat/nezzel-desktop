use crate::paths::PathsManager;
use crate::state::json_state::{json_merge, JsonBackedState};
use arc_swap::ArcSwap;
use serde_json::{Map, Value};
use std::{
  error::Error,
  path::PathBuf,
  sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
  },
};
use tauri::{AppHandle, Manager, Runtime, Wry};
use tauri_plugin_store::{Store, StoreExt};

pub struct JsonStoreHandle<T: JsonBackedState, R: Runtime = Wry> {
  swap: Arc<ArcSwap<T>>,
  store: Arc<Store<R>>,
  writes: Mutex<()>,
  isolated: bool,
  store_path: PathBuf,
  /// Set when a damaged store file could not be backed up: nothing may overwrite it until a
  /// backup succeeds, so changes live in memory only.
  persist_blocked: AtomicBool,
}

impl<T: JsonBackedState> JsonStoreHandle<T> {
  /// Harness state never reads or writes the owner's persisted settings.
  #[cfg(feature = "link-matrix")]
  pub(crate) fn isolated(app: &AppHandle<Wry>, initial: T) -> Result<Self, Box<dyn Error>> {
    let root = app.state::<PathsManager>().app_dir().clone();
    let store = app
      .store_builder(root.join(T::STORE_FILE))
      .disable_auto_save()
      .build()?;
    Ok(Self {
      swap: Arc::new(ArcSwap::from_pointee(initial)),
      store,
      writes: Mutex::new(()),
      isolated: true,
      store_path: root.join(T::STORE_FILE),
      persist_blocked: AtomicBool::new(false),
    })
  }

  pub fn init(app: &AppHandle<Wry>) -> Result<Self, Box<dyn Error>> {
    let paths_manager = app.state::<PathsManager>();
    let data_root = paths_manager.app_dir();
    let store_path = data_root.join(T::STORE_FILE);
    let store = app.store(store_path)?;

    let (mut initial, blocked) = read_initial::<T, Wry>(&store, &data_root.join(T::STORE_FILE));
    T::before_initialized(app, &mut initial);
    Self::finish_init(store, data_root.join(T::STORE_FILE), initial, blocked)
  }

  pub fn apply_patch(&self, patch: &Value, app: &AppHandle<Wry>) -> Result<T, Box<dyn Error>> {
    let _write = self.writes.lock().unwrap();
    let mut raw = serde_json::to_value(self.swap.load_full().as_ref())?;

    json_merge(&mut raw, patch);

    let new_value = T::materialize(&raw)?;
    commit_snapshot(
      &self.swap,
      new_value,
      self.isolated,
      |value| {
        T::on_updated(app, value);
        self.save_value(serde_json::to_value(value)?)
      },
      |_| {},
    )
  }

  /// Serialize a typed edit against the newest snapshot; unlike merge patches,
  /// this supports deleting a map key without replacing unrelated settings.
  pub fn modify(
    &self,
    app: &AppHandle<Wry>,
    edit: impl FnOnce(&mut T),
  ) -> Result<T, Box<dyn Error>> {
    let _write = self.writes.lock().unwrap();
    let previous = self.swap.load_full();
    let next = modified_snapshot(previous.as_ref(), edit);
    commit_snapshot(
      &self.swap,
      next,
      self.isolated,
      |value| {
        let previous_value = serde_json::to_value(previous.as_ref())?;
        if let Err(error) = self.save_value(serde_json::to_value(value)?) {
          if !self.persist_blocked.load(Ordering::SeqCst) {
            self.store.set(T::ROOT_KEY, previous_value);
          }
          return Err(error);
        }
        Ok(())
      },
      |value| T::on_updated(app, value),
    )
  }

  pub fn reset(&self, app: &AppHandle<Wry>) -> Result<T, Box<dyn Error>> {
    let _write = self.writes.lock().unwrap();
    let default_value = T::default_value();
    let raw = serde_json::to_value(&default_value)?;

    commit_snapshot(
      &self.swap,
      default_value,
      self.isolated,
      |value| {
        T::on_updated(app, value);
        self.save_value(raw)
      },
      |_| {},
    )
  }
}

impl<T: JsonBackedState, R: Runtime> JsonStoreHandle<T, R> {
  fn finish_init(
    store: Arc<Store<R>>,
    store_path: PathBuf,
    initial: T,
    blocked: bool,
  ) -> Result<Self, Box<dyn Error>> {
    if !blocked {
      store.set(T::ROOT_KEY, serde_json::to_value(&initial)?);
      // The original is already backed up, so a failed save must not abort start-up.
      if let Err(error) = store.save() {
        tracing::error!(%error, store = T::STORE_FILE, "could not save repaired settings");
      }
    } else {
      // The plugin saves every registered store on exit, with no dirty check: an unbacked-up
      // original must not be overwritten then. Our own Arc keeps working for explicit saves.
      store.close_resource();
    }
    Ok(Self {
      swap: Arc::new(ArcSwap::from_pointee(initial)),
      store,
      writes: Mutex::new(()),
      isolated: false,
      store_path,
      persist_blocked: AtomicBool::new(blocked),
    })
  }

  /// Saves unless a damaged original is still unbacked-up; then it retries the backup first
  /// and, if that still fails, keeps the change in memory only.
  fn save_value(&self, value: Value) -> Result<(), Box<dyn Error>> {
    if self.persist_blocked.load(Ordering::SeqCst) {
      if backup_store(&self.store_path).is_err() {
        return Ok(());
      }
      self.persist_blocked.store(false, Ordering::SeqCst);
    }
    self.store.set(T::ROOT_KEY, value);
    self.store.save()?;
    Ok(())
  }

  #[inline]
  pub fn load(&self) -> Arc<T> {
    self.swap.load_full()
  }
}

#[cfg(test)]
impl<T: JsonBackedState> JsonStoreHandle<T, tauri::test::MockRuntime> {
  pub(crate) fn isolated_for_test(
    app: &AppHandle<tauri::test::MockRuntime>,
    root: &std::path::Path,
    initial: T,
  ) -> Result<Self, Box<dyn Error>> {
    let store = app
      .store_builder(root.join(T::STORE_FILE))
      .disable_auto_save()
      .build()?;
    Ok(Self {
      swap: Arc::new(ArcSwap::from_pointee(initial)),
      store,
      writes: Mutex::new(()),
      isolated: true,
      store_path: root.join(T::STORE_FILE),
      persist_blocked: AtomicBool::new(false),
    })
  }
}

/// The store plugin ignores load errors, so a truncated or non-JSON file arrives here as an
/// empty store. Backs such a file up before defaults are saved over it; true means the
/// backup failed and nothing may overwrite the file.
fn backup_unreadable_store(path: &std::path::Path, root_key: &str, has_root: bool) -> bool {
  if has_root {
    return false;
  }
  let Ok(bytes) = std::fs::read(path) else {
    return false;
  };
  if bytes.is_empty() {
    return false;
  }
  if serde_json::from_slice::<Value>(&bytes).is_ok_and(|v| v.get(root_key).is_some()) {
    return false;
  }
  tracing::error!(
    root_key,
    "settings file unreadable or missing its root key, backing it up before using defaults"
  );
  backup_store(path).is_err()
}

/// Initial value for a freshly opened store; the flag is true when persisting is blocked.
fn read_initial<T: JsonBackedState, R: Runtime>(
  store: &Store<R>,
  path: &std::path::Path,
) -> (T, bool) {
  let stored = store.get(T::ROOT_KEY);
  let blocked = backup_unreadable_store(path, T::ROOT_KEY, stored.is_some());
  let raw = stored.unwrap_or_else(|| Value::Object(Map::new()));
  if blocked {
    (T::materialize_lenient(&raw), true)
  } else {
    load_or_repair::<T>(&raw, path)
  }
}

/// Reads the stored value, repairing bad fields. The flag is true when the damaged original
/// could not be backed up, so the repaired value must stay in memory only.
fn load_or_repair<T: JsonBackedState>(raw: &Value, path: &std::path::Path) -> (T, bool) {
  let initial = T::materialize_lenient(raw);
  let mut blocked = false;
  if T::materialize(raw).is_err() {
    tracing::error!(
      store = T::STORE_FILE,
      "stored settings not fully accepted, repaired with defaults"
    );
    blocked = backup_store(path).is_err();
  }
  (initial, blocked)
}

/// Copies the store file to a fresh `<file>.bad-<unique>` beside it; never overwrites a backup.
fn backup_store(path: &std::path::Path) -> std::io::Result<()> {
  use std::io::Write;
  let bytes = std::fs::read(path)?;
  let name = path
    .file_name()
    .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
  // A byte-identical backup already exists (e.g. the repaired file could not be saved and
  // every launch sees the same original): keep that one instead of adding another.
  let prefix = format!("{name}.bad-");
  let existing = path
    .parent()
    .and_then(|dir| std::fs::read_dir(dir).ok())
    .is_some_and(|entries| {
      entries.filter_map(Result::ok).any(|e| {
        e.file_name().to_string_lossy().starts_with(&prefix)
          && std::fs::read(e.path()).is_ok_and(|b| b == bytes)
      })
    });
  if existing {
    return Ok(());
  }
  let backup = path.with_file_name(format!("{name}.bad-{}", uuid::Uuid::new_v4()));
  let mut options = std::fs::OpenOptions::new();
  options.write(true).create_new(true);
  #[cfg(unix)]
  {
    use std::os::unix::fs::OpenOptionsExt;
    options.mode(0o600);
  }
  let mut file = options.open(&backup)?;
  let written = file.write_all(&bytes).and_then(|()| file.sync_all());
  if written.is_err() {
    drop(file);
    let _ = std::fs::remove_file(&backup);
  }
  written
}

fn modified_snapshot<T: Clone>(previous: &T, edit: impl FnOnce(&mut T)) -> T {
  let mut next = previous.clone();
  edit(&mut next);
  next
}

// Harness edits must not persist browser consent or run OS settings callbacks.
fn commit_snapshot<T: Clone, E>(
  swap: &ArcSwap<T>,
  next: T,
  isolated: bool,
  persist: impl FnOnce(&T) -> Result<(), E>,
  updated: impl FnOnce(&T),
) -> Result<T, E> {
  if !isolated {
    persist(&next)?;
  }
  swap.store(Arc::new(next.clone()));
  if !isolated {
    updated(&next);
  }
  Ok(next)
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::state::config_models::Config;
  #[test]
  fn backup_never_overwrites_and_fails_when_the_store_file_is_missing() {
    let dir = std::env::temp_dir().join(format!("nezzel-bak-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("x.store.json");
    assert!(backup_store(&path).is_err());
    std::fs::write(&path, b"{}").unwrap();
    backup_store(&path).unwrap();
    backup_store(&path).unwrap();
    assert_eq!(bad_copies(&dir), 1, "identical bytes are backed up once");
    std::fs::write(&path, b"{\"a\":1}").unwrap();
    backup_store(&path).unwrap();
    assert_eq!(bad_copies(&dir), 2);
    std::fs::remove_dir_all(dir).unwrap();
  }

  type MockHandle = JsonStoreHandle<Config, tauri::test::MockRuntime>;

  fn open_raw(path: &std::path::Path) -> (tauri::App<tauri::test::MockRuntime>, MockHandle) {
    let app = tauri::test::mock_builder()
      .plugin(tauri_plugin_store::Builder::new().build())
      .build(tauri::test::mock_context(tauri::test::noop_assets()))
      .unwrap();
    let store = app.store_builder(path).disable_auto_save().build().unwrap();
    let (initial, blocked) = read_initial::<Config, _>(&store, path);
    let handle = MockHandle::finish_init(store, path.to_path_buf(), initial, blocked).unwrap();
    (app, handle)
  }

  #[cfg(unix)]
  #[test]
  fn unsavable_repaired_file_is_backed_up_once_across_launches() {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("nezzel-rofile-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(Config::STORE_FILE);
    std::fs::write(&path, br#"{"config":{"perf"#).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o444)).unwrap();
    let writable = std::fs::OpenOptions::new().write(true).open(&path).is_ok();
    if !writable {
      for _ in 0..3 {
        let _launch = open_raw(&path);
      }
      assert_eq!(bad_copies(&dir), 1);
    }
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    std::fs::remove_dir_all(dir).unwrap();
  }

  #[test]
  fn truncated_file_is_backed_up_once_before_defaults_are_saved() {
    let dir = std::env::temp_dir().join(format!("nezzel-trunc-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(Config::STORE_FILE);
    let original = br#"{"config":{"performance":{"maxConc"#;
    std::fs::write(&path, original).unwrap();
    let (_app, handle) = open_raw(&path);
    assert!(!handle.persist_blocked.load(Ordering::SeqCst));
    let backups: Vec<_> = std::fs::read_dir(&dir)
      .unwrap()
      .filter_map(Result::ok)
      .filter(|e| e.file_name().to_string_lossy().contains(".bad-"))
      .collect();
    assert_eq!(backups.len(), 1);
    assert_eq!(std::fs::read(backups[0].path()).unwrap(), original);
    let saved = std::fs::read_to_string(&path).unwrap();
    assert!(saved.contains("maxConcurrency"), "{saved}");
    std::fs::remove_dir_all(dir).unwrap();
  }

  #[cfg(unix)]
  #[test]
  fn truncated_file_with_failed_backup_stays_untouched_after_a_save() {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("nezzel-trro-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let original = br#"{"config":{"perf"#;
    let path = dir.join(Config::STORE_FILE);
    std::fs::write(&path, original).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
    if std::fs::File::create(dir.join("probe")).is_ok() {
      std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
      std::fs::remove_dir_all(dir).unwrap();
      return;
    }
    let (app, handle) = open_raw(&path);
    assert!(handle.persist_blocked.load(Ordering::SeqCst));
    // The plugin's exit handler saves every registered store without a dirty check.
    assert!(app.get_store(&path).is_none());
    handle.save_value(serde_json::json!({"c": 1})).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), original);
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::remove_dir_all(dir).unwrap();
  }

  fn open_damaged(path: &std::path::Path) -> (MockHandle, Value) {
    let app = tauri::test::mock_builder()
      .plugin(tauri_plugin_store::Builder::new().build())
      .build(tauri::test::mock_context(tauri::test::noop_assets()))
      .unwrap();
    let store = app.store_builder(path).disable_auto_save().build().unwrap();
    let raw = serde_json::json!({"performance": {"maxConcurrency": "bad"}});
    let (initial, blocked) = load_or_repair::<Config>(&raw, path);
    (
      MockHandle::finish_init(store, path.to_path_buf(), initial, blocked).unwrap(),
      raw,
    )
  }

  fn bad_copies(dir: &std::path::Path) -> usize {
    std::fs::read_dir(dir)
      .unwrap()
      .filter_map(Result::ok)
      .filter(|e| e.file_name().to_string_lossy().contains(".bad-"))
      .count()
  }

  #[test]
  fn bad_data_is_backed_up_once_and_the_repaired_file_saved() {
    let dir = std::env::temp_dir().join(format!("nezzel-fix-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(Config::STORE_FILE), br#"{"config":{"x":1}}"#).unwrap();
    let (handle, _) = open_damaged(&dir.join(Config::STORE_FILE));
    assert!(!handle.persist_blocked.load(Ordering::SeqCst));
    assert_eq!(bad_copies(&dir), 1);
    let saved = std::fs::read_to_string(dir.join(Config::STORE_FILE)).unwrap();
    assert!(saved.contains("maxConcurrency"), "{saved}");
    std::fs::remove_dir_all(dir).unwrap();
  }

  #[cfg(unix)]
  #[test]
  fn failed_backup_leaves_the_file_untouched_even_after_a_later_save() {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("nezzel-ro-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let original = br#"{"config":{"x":1}}"#;
    let path = dir.join(Config::STORE_FILE);
    std::fs::write(&path, original).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
    if std::fs::File::create(dir.join("probe")).is_ok() {
      // Running as a user the mode does not restrict: cannot simulate a failed backup.
      std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
      std::fs::remove_dir_all(dir).unwrap();
      return;
    }
    let (handle, _) = open_damaged(&path);
    assert!(handle.persist_blocked.load(Ordering::SeqCst));
    assert_eq!(std::fs::read(&path).unwrap(), original);
    handle
      .save_value(serde_json::json!({"changed": true}))
      .unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), original);
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    handle
      .save_value(serde_json::json!({"changed": true}))
      .unwrap();
    assert_eq!(bad_copies(&dir), 1);
    assert!(!handle.persist_blocked.load(Ordering::SeqCst));
    std::fs::remove_dir_all(dir).unwrap();
  }

  #[test]
  fn typed_edits_add_and_remove_a_host_without_changing_other_config() {
    let mut original = Config::default();
    original.performance.max_concurrency = 7;
    original.auth.cookie_file = Some("/manual.txt".into());
    original
      .auth
      .site_browsers
      .insert("other.test".into(), "firefox".into());
    let added = modified_snapshot(&original, |cfg| {
      cfg
        .auth
        .site_browsers
        .insert("site.test".into(), "chrome".into());
    });
    assert_eq!(added.auth.site_browsers["site.test"], "chrome");
    let removed = modified_snapshot(&added, |cfg| {
      cfg.auth.site_browsers.remove("site.test");
    });
    assert_eq!(
      serde_json::to_value(removed).unwrap(),
      serde_json::to_value(original).unwrap()
    );
  }

  #[test]
  fn isolated_snapshot_edits_do_not_persist_or_notify_but_normal_edits_do() {
    for isolated in [true, false] {
      let path = std::env::temp_dir().join(format!("nezzel-json-{}.json", uuid::Uuid::new_v4()));
      let swap = ArcSwap::from_pointee(serde_json::json!({"browser":"none"}));
      let next = serde_json::json!({"browser":"firefox:/private/profile"});
      let notified = std::cell::Cell::new(0);
      let saved = commit_snapshot(
        &swap,
        next.clone(),
        isolated,
        |value| std::fs::write(&path, serde_json::to_vec(value).unwrap()),
        |_| notified.set(notified.get() + 1),
      )
      .unwrap();
      assert_eq!(saved, next);
      assert_eq!(**swap.load(), next);
      assert_eq!(notified.get(), usize::from(!isolated));
      assert_eq!(path.exists(), !isolated);
      if !isolated {
        assert_eq!(
          serde_json::from_slice::<Value>(&std::fs::read(&path).unwrap()).unwrap(),
          next
        );
        std::fs::remove_file(path).unwrap();
      }
    }
  }
}
