use super::*;
use crate::download_records::resolve_trash_target;
use std::fs;

struct Sandbox(PathBuf);
impl Sandbox {
  fn new() -> Self {
    let path = std::env::temp_dir().join(format!("nezzel-library-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&path).unwrap();
    Self(path)
  }
  fn file(&self, name: &str) -> PathBuf {
    let path = self.0.join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, b"test").unwrap();
    path
  }
  fn index(&self) -> PathBuf {
    self.0.join("library.json")
  }
}
impl Drop for Sandbox {
  fn drop(&mut self) {
    let _ = fs::remove_dir_all(&self.0);
  }
}
fn record(path: PathBuf, time: u64) -> LibraryRecord {
  LibraryRecord {
    id: format!("item-{time}"),
    paths: vec![path],
    url: "https://example.com/video".into(),
    title: "فيديو".into(),
    site: Some("Example".into()),
    kind: "video".into(),
    size: 4,
    completed_at: time,
    thumbnail: None,
    authority: Vec::new(),
  }
}
#[test]
fn append_load_roundtrip_and_prune_missing() {
  let dir = Sandbox::new();
  let path = dir.file("فيديو.mp4");
  let records = DownloadRecords::default();
  records.allow_root("g", dir.0.clone());
  records.record_item("g", "item", path.to_str().unwrap());
  records.seal("g");
  let mut row = record(path.clone(), 10);
  row.authority = recorded_files(&records.get("g"), "item");
  let mut index = Index::load(&dir.index()).unwrap();
  index.append(row);
  index.save(&dir.index()).unwrap();
  let mut loaded = Index::load(&dir.index()).unwrap();
  assert_eq!(loaded.records[0].title, "فيديو");
  assert_eq!(
    loaded.records[0].authority[0].root,
    dir.0.canonicalize().unwrap()
  );
  let restored = DownloadRecords::default();
  restored.restore_history("history", &loaded.records[0].authority, None, None);
  assert!(resolve_trash_target(&restored.get("history"), &path).is_ok());
  assert_eq!(
    loaded.records[0].paths.as_slice(),
    std::slice::from_ref(&path)
  );
  fs::remove_file(path).unwrap();
  loaded.prune();
  loaded.save(&dir.index()).unwrap();
  assert!(Index::load(&dir.index()).unwrap().records.is_empty());
}
#[test]
fn corrupt_and_old_files_start_empty_with_original_backup() {
  let dir = Sandbox::new();
  fs::write(dir.index(), b"not JSON").unwrap();
  assert!(Index::load(&dir.index()).unwrap().records.is_empty());
  let old = br#"{"version":0,"records":[]}"#;
  fs::write(dir.index(), old).unwrap();
  assert!(Index::load(&dir.index()).unwrap().records.is_empty());
  assert_eq!(
    fs::read(dir.index().with_extension("json.bak")).unwrap(),
    b"not JSON"
  );
  let backups = fs::read_dir(&dir.0)
    .unwrap()
    .flatten()
    .map(|e| e.path())
    .filter(|p| p.extension().is_some_and(|e| e == "bak"))
    .collect::<Vec<_>>();
  assert_eq!(backups.len(), 2);
  assert!(backups.iter().any(|p| fs::read(p).unwrap() == old));
}
#[test]
fn cap_keeps_newest_and_append_replaces_duplicate_paths() {
  let mut index = Index::default();
  // Seed once: repeatedly appending thousands of rows only repeats the same branch.
  index.records = (0..5002)
    .rev()
    .map(|i| record(PathBuf::from(format!("/test/{i}.mp4")), i))
    .collect();
  index.append(record(PathBuf::from("/test/5001.mp4"), 5001));
  assert_eq!(index.records.len(), 5000);
  assert_eq!(index.records[0].completed_at, 5001);
  assert_eq!(index.records.last().unwrap().completed_at, 2);
  index.append(record(PathBuf::from("/test/5001.mp4"), 5003));
  assert_eq!(index.records.len(), 5000);
  assert_eq!(index.records[0].completed_at, 5003);
  index.merge_scan(
    (0..2002)
      .map(|i| {
        let mut row = record(PathBuf::from(format!("/scan/{i}.mp4")), 10000 + i);
        row.url.clear();
        row
      })
      .collect(),
  );
  assert_eq!(index.records.len(), 5000);
  assert_eq!(index.scanned.len(), 2000);
  assert_eq!(index.scanned.first().unwrap().completed_at, 12001);
  assert_eq!(index.scanned.last().unwrap().completed_at, 10002);
  assert_eq!(index.records.last().unwrap().completed_at, 2);
  // The WIP format mixed scanned rows into records. Apply both caps when importing it too.
  let dir = Sandbox::new();
  fs::write(
    dir.index(),
    serde_json::to_vec(&serde_json::json!({"version": 1, "records": index.rows()})).unwrap(),
  )
  .unwrap();
  let loaded = Index::load(&dir.index()).unwrap();
  assert_eq!(loaded.records.len(), 5000);
  assert_eq!(loaded.scanned.len(), 2000);
}
#[test]
fn scan_skips_hidden_temporary_and_deep_files_and_refuses_broad_roots() {
  let dir = Sandbox::new();
  let home = dir.0.join("home");
  let downloads = home.join("Downloads");
  let root = downloads.join("Nezzel");
  for name in [
    "فيديو.mp4",
    "photos/pic.JPG",
    "a/b/c/allowed.pdf",
    ".hidden.mp4",
    ".hidden/file.mp4",
    "clip.part",
    "clip.ytdl",
    "clip.aria2",
    "clip.tmp",
    "clip.nezzel-part",
    "clip.nezzel-state",
    "a/b/c/d/deep.mp4",
  ] {
    dir.file(&format!("home/Downloads/Nezzel/{name}"));
  }
  let rows = scan_folder(&root, Some(&home), Some(&downloads));
  let rescanned = scan_folder(&root, Some(&home), Some(&downloads));
  for row in &rows {
    assert_eq!(
      row.id,
      rescanned.iter().find(|r| r.paths == row.paths).unwrap().id,
      "rescanning a path must preserve its row identity"
    );
  }
  let mut names = rows
    .iter()
    .map(|r| {
      r.paths[0]
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned()
    })
    .collect::<Vec<_>>();
  names.sort();
  assert_eq!(names, ["allowed.pdf", "pic.JPG", "فيديو.mp4"]);
  assert_eq!(
    rows.iter().find(|r| r.title == "فيديو.mp4").unwrap().kind,
    "video"
  );
  for denied in [&home, &downloads, &dir.0, Path::new("/")] {
    assert!(scan_folder(denied, Some(&home), Some(&downloads)).is_empty());
  }
}
#[test]
fn removal_survives_reload_and_rescan_but_a_new_download_can_restore_it() {
  let dir = Sandbox::new();
  let path = dir.file("clip.mp4");
  let mut index = Index::default();
  index.append(record(path.clone(), 1));
  index.remove(
    std::slice::from_ref(&path),
    Some(&dir.0.canonicalize().unwrap()),
  );
  index.save(&dir.index()).unwrap();
  let mut loaded = Index::load(&dir.index()).unwrap();
  assert!(loaded.records.is_empty());
  loaded.merge_scan(vec![record(path.clone(), 2)]);
  assert!(loaded.scanned.is_empty());
  loaded.append(record(path, 3));
  assert_eq!(loaded.records.len(), 1);
}
#[cfg(unix)]
#[test]
fn scan_does_not_follow_symlinks_outside_the_owned_folder() {
  let dir = Sandbox::new();
  let file = dir.file("outside/secret.mp4");
  let root = dir.0.join("Nezzel");
  fs::create_dir(&root).unwrap();
  std::os::unix::fs::symlink(&file, root.join("link.mp4")).unwrap();
  std::os::unix::fs::symlink(file.parent().unwrap(), root.join("folder")).unwrap();
  assert!(scan_folder(&root, None, None).is_empty());
}

#[test]
fn removal_matches_windows_native_canonical_paths_and_case() {
  let mut index = Index::default();
  index.append(record(PathBuf::from(r"\\?\C:\Downloads\Clip.mp4"), 1));
  index.remove(
    &[PathBuf::from(r"C:\Downloads\clip.mp4")],
    Some(Path::new(r"C:\Downloads")),
  );
  assert!(index.records.is_empty());
  index.merge_scan(vec![record(PathBuf::from(r"\\?\C:\Downloads\Clip.mp4"), 2)]);
  assert!(index.scanned.is_empty());
}

#[test]
fn unavailable_parent_keeps_history_until_the_drive_is_available() {
  let dir = Sandbox::new();
  let path = dir.file("drive/clip.mp4");
  let mut index = Index::default();
  index.append(record(path.clone(), 1));
  fs::rename(dir.0.join("drive"), dir.0.join("unmounted")).unwrap();
  index.prune();
  assert_eq!(index.records.len(), 1);
  fs::create_dir(dir.0.join("drive")).unwrap();
  index.prune();
  assert!(index.records.is_empty());
}

#[test]
fn scan_ignores_only_final_temporary_extensions() {
  let dir = Sandbox::new();
  for name in [
    "download.mp4",
    "Song.Part.1.mp3",
    "temp.jpg",
    "clip.part.mp4",
    "clip.part",
    "clip.ytdl",
    "clip.aria2",
    "clip.temp",
    "clip.part-Frag23",
  ] {
    dir.file(name);
  }
  let mut names = scan_folder(&dir.0, None, None)
    .into_iter()
    .map(|r| r.title)
    .collect::<Vec<_>>();
  names.sort();
  assert_eq!(
    names,
    [
      "Song.Part.1.mp3",
      "clip.part.mp4",
      "download.mp4",
      "temp.jpg"
    ]
  );
}

#[test]
fn exclusions_are_scoped_bounded_and_pruned_when_files_disappear() {
  let dir = Sandbox::new();
  let root = dir.0.join("Nezzel");
  let mut paths = (0..2002)
    .map(|i| dir.file(&format!("Nezzel/{i}.mp4")))
    .collect::<Vec<_>>();
  paths.push(dir.file("outside.mp4"));
  let root = root.canonicalize().unwrap();
  let mut index = Index::default();
  index.remove(&paths, Some(&root));
  let excluded = serde_json::to_value(&index).unwrap()["excluded"]
    .as_array()
    .unwrap()
    .clone();
  assert_eq!(excluded.len(), 2000);
  assert!(excluded
    .iter()
    .all(|p| Path::new(p.as_str().unwrap()).starts_with(&root)));
  index.save(&dir.index()).unwrap();
  let other_root = dir.0.join("Other");
  for (active_root, expected) in [
    (Some(root.as_path()), 2000),
    (None, 2000),
    (Some(other_root.as_path()), 0),
  ] {
    let mut data = LibraryData::default();
    let loaded = load_index(&mut data, &dir.index(), active_root).unwrap();
    assert_eq!(
      serde_json::to_value(loaded).unwrap()["excluded"]
        .as_array()
        .unwrap()
        .len(),
      expected
    );
  }
  for path in paths {
    fs::remove_file(path).unwrap();
  }
  // Startup must prune missing excluded files before any mutation.
  index.save(&dir.index()).unwrap();
  let mut data = LibraryData::default();
  let index = load_index(&mut data, &dir.index(), Some(&root)).unwrap();
  assert!(serde_json::to_value(index).unwrap()["excluded"]
    .as_array()
    .unwrap()
    .is_empty());
}

#[test]
fn metadata_is_released_for_failed_items_and_cancelled_groups() {
  let library = Library::default();
  library.remember(
    "g",
    [
      ("fatal".into(), CompletionMetadata::default()),
      ("pending".into(), CompletionMetadata::default()),
    ]
    .into(),
  );
  library.forget_item("g", "fatal");
  assert_eq!(library.0.lock().unwrap().metadata["g"].len(), 1);
  library.forget_group("g");
  assert!(library.0.lock().unwrap().metadata.is_empty());
}
