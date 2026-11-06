use crate::binaries::bundled_tools::{exe_name, executable, sidecar_candidates};
use std::{
  io::Read,
  path::{Path, PathBuf},
};
use tauri::{AppHandle, Manager};

const STUB_MARKER: &[u8] = b"NEZZEL_ARIA2_UNAVAILABLE\n";
pub fn resolve_aria2(app: &AppHandle) -> Option<PathBuf> {
  find_bundled_aria2(
    std::env::current_exe().ok().as_deref(),
    app.path().resource_dir().ok().as_deref(),
  )
}
fn find_bundled_aria2(exe: Option<&Path>, resources: Option<&Path>) -> Option<PathBuf> {
  sidecar_candidates(exe, resources, &exe_name("aria2c"))
    .into_iter()
    .find(|path| {
      if !executable(path) {
        return false;
      }
      let mut header = [0; STUB_MARKER.len()];
      std::fs::File::open(path)
        .and_then(|mut f| f.read(&mut header))
        .is_ok_and(|size| !header[..size].starts_with(STUB_MARKER))
    })
}

/// Which optional download engines this build ships (Settings > Performance).
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineAvailability {
  aria2_available: bool,
}
#[tauri::command]
pub fn engine_availability(app: AppHandle) -> EngineAvailability {
  EngineAvailability {
    aria2_available: resolve_aria2(&app).is_some(),
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  #[test]
  fn only_bundled_non_stub_sidecars_are_available() {
    let root = std::env::temp_dir().join(format!("aria2-paths-{}", uuid::Uuid::new_v4()));
    let bin = root.join("app");
    let resources = root.join("resources");
    let system = root.join("system");
    for dir in [&bin, &resources, &system] {
      std::fs::create_dir_all(dir).unwrap();
    }
    let name = exe_name("aria2c");
    let name = name.as_str();
    let write = |path: &Path, bytes: &[u8]| {
      std::fs::write(path, bytes).unwrap();
      #[cfg(unix)]
      {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
      }
    };
    write(&system.join(name), b"unbundled executable");
    assert!(find_bundled_aria2(Some(&bin.join("app")), Some(&resources)).is_none());
    write(&bin.join(name), STUB_MARKER);
    assert!(find_bundled_aria2(Some(&bin.join("app")), Some(&resources)).is_none());
    write(&resources.join(name), b"verified bundled executable");
    assert_eq!(
      find_bundled_aria2(Some(&bin.join("app")), Some(&resources)),
      Some(resources.join(name))
    );
    std::fs::remove_dir_all(root).unwrap();
  }
}
