use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;
use std::error::Error;
use tauri::{AppHandle, Wry};

pub trait JsonBackedState:
  Sized + Clone + Send + Sync + 'static + Serialize + DeserializeOwned
{
  const STORE_FILE: &'static str;

  const ROOT_KEY: &'static str;

  fn default_value() -> Self;

  fn materialize(raw: &Value) -> Result<Self, Box<dyn Error>> {
    let mut merged = serde_json::to_value(Self::default_value())?;
    json_merge(&mut merged, raw);
    Ok(serde_json::from_value(merged)?)
  }

  /// Start-up only: never fails. Keeps every stored value, at any depth, that the current
  /// build accepts; a bad leaf or array element is dropped and falls back to the default.
  /// Logs key names only.
  fn materialize_lenient(raw: &Value) -> Self {
    if let Ok(value) = Self::materialize(raw) {
      return value;
    }
    let Value::Object(top) = raw else {
      return Self::default_value();
    };
    let mut accepted = Value::Object(serde_json::Map::new());
    keep_valid::<Self>(&mut accepted, &mut Vec::new(), top);
    Self::materialize(&accepted).unwrap_or_else(|_| Self::default_value())
  }

  fn before_initialized(_app: &AppHandle<Wry>, _value: &mut Self) {}
  fn on_updated(_app: &AppHandle<Wry>, _new_value: &Self) {}
}

pub fn json_merge(base: &mut Value, patch: &Value) {
  match (base, patch) {
    (Value::Object(base_map), Value::Object(patch_map)) => {
      for (k, v) in patch_map {
        json_merge(base_map.entry(k).or_insert(Value::Null), v);
      }
    }
    (a, b) => *a = b.clone(),
  }
}

fn slot<'a>(root: &'a mut Value, path: &[String]) -> Option<&'a mut Value> {
  path.iter().try_fold(root, |node, key| node.get_mut(key))
}

fn accepts<T: JsonBackedState>(accepted: &Value) -> bool {
  T::materialize(accepted).is_ok()
}

/// Adds `patch` entries under `path` one by one, keeping only those that still deserialize.
fn keep_valid<T: JsonBackedState>(
  accepted: &mut Value,
  path: &mut Vec<String>,
  patch: &serde_json::Map<String, Value>,
) {
  for (key, value) in patch {
    let Some(Value::Object(parent)) = slot(accepted, path) else {
      return;
    };
    parent.insert(key.clone(), value.clone());
    if accepts::<T>(accepted) {
      continue;
    }
    path.push(key.clone());
    match value {
      Value::Object(children) => {
        if let Some(node) = slot(accepted, path) {
          *node = Value::Object(serde_json::Map::new());
        }
        if accepts::<T>(accepted) {
          keep_valid::<T>(accepted, path, children);
        } else {
          drop_key(accepted, path, key);
        }
      }
      Value::Array(items) => {
        if let Some(node) = slot(accepted, path) {
          *node = Value::Array(Vec::new());
        }
        if accepts::<T>(accepted) {
          for item in items {
            if let Some(Value::Array(list)) = slot(accepted, path) {
              list.push(item.clone());
            }
            if !accepts::<T>(accepted) {
              if let Some(Value::Array(list)) = slot(accepted, path) {
                list.pop();
              }
              tracing::warn!(key = %key, "stored list entry not accepted, dropped");
            }
          }
        } else {
          drop_key(accepted, path, key);
        }
      }
      _ => drop_key(accepted, path, key),
    }
    path.pop();
  }
}

fn drop_key(accepted: &mut Value, path: &[String], key: &str) {
  tracing::warn!(key = %key, "stored setting not accepted, using default");
  if let Some((_, parent_path)) = path.split_last() {
    if let Some(Value::Object(parent)) = slot(accepted, parent_path) {
      parent.remove(key);
    }
  }
}
