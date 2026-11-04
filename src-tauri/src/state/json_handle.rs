use crate::paths::PathsManager;
use crate::state::json_state::{json_merge, JsonBackedState};
use arc_swap::ArcSwap;
use serde_json::{Map, Value};
use std::{
  error::Error,
  sync::{Arc, Mutex},
};
use tauri::{AppHandle, Manager, Runtime, Wry};
use tauri_plugin_store::{Store, StoreExt};

pub struct JsonStoreHandle<T: JsonBackedState, R: Runtime = Wry> {
  swap: Arc<ArcSwap<T>>,
  store: Arc<Store<R>>,
  writes: Mutex<()>,
  isolated: bool,
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
    })
  }

  pub fn init(app: &AppHandle<Wry>) -> Result<Self, Box<dyn Error>> {
    let paths_manager = app.state::<PathsManager>();
    let data_root = paths_manager.app_dir();
    let store_path = data_root.join(T::STORE_FILE);
    let store = app.store(store_path)?;

    let raw = store
      .get(T::ROOT_KEY)
      .unwrap_or_else(|| Value::Object(Map::new()));

    let mut initial = T::materialize(&raw)?;
    T::before_initialized(app, &mut initial);

    let to_save = serde_json::to_value(&initial)?;
    store.set(T::ROOT_KEY, to_save);
    store.save()?;

    Ok(Self {
      swap: Arc::new(ArcSwap::from_pointee(initial)),
      store,
      writes: Mutex::new(()),
      isolated: false,
    })
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
        self.store.set(T::ROOT_KEY, serde_json::to_value(value)?);
        self.store.save()?;
        Ok(())
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
        self.store.set(T::ROOT_KEY, serde_json::to_value(value)?);
        if let Err(error) = self.store.save() {
          self
            .store
            .set(T::ROOT_KEY, serde_json::to_value(previous.as_ref())?);
          return Err(error.into());
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
        self.store.set(T::ROOT_KEY, raw);
        self.store.save()?;
        Ok(())
      },
      |_| {},
    )
  }
}

impl<T: JsonBackedState, R: Runtime> JsonStoreHandle<T, R> {
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
    })
  }
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
