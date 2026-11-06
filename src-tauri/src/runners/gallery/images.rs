//! Image downloads: transfer, verification and destination naming.
use super::runner::GalleryRunner;
use crate::models::TrackType;
use crate::parsers::gallery::{
  cdn_failure, gallery_deadline, verify_receipt, GalleryAsset, GalleryError, GalleryPost,
};
use crate::runners::{
  gallery_files::{image_type, Staging},
  gallery_process::execute,
  ytdlp_runner::YtdlpRunner,
};
use crate::scheduling::{
  download_pipeline::DownloadEntry,
  group_state::{is_group_running, subscribe_group},
};
use serde_json::json;
use std::path::{Path, PathBuf};
use tauri::AppHandle;

impl GalleryRunner {
  pub(super) async fn images(
    &self,
    post: &GalleryPost,
    assets: &[&GalleryAsset],
    group: &str,
    stage: &Path,
  ) -> Result<Vec<PathBuf>, GalleryError> {
    if assets.is_empty() {
      return Ok(Vec::new());
    }
    let mut input = String::new();
    for asset in assets {
      let directory = stage.join(asset.index.to_string());
      std::fs::create_dir(&directory).map_err(|_| GalleryError::Invalid)?;
      // Per-URL input options isolate filenames and receipts in a single process.
      // JSON encoding prevents a source URL or directory from injecting input directives.
      input.push_str(&format!(
        "-base-directory={}\n-extractor.directlink.transform={}\nhttps://nezzel.invalid/attachment.jpg\n",
        json!(directory),
        json!([[
          r"^https://nezzel\.invalid/attachment\.jpg$",
          asset.item.url.replace('\\', r"\\")
        ]])
      ));
    }
    let input_path = stage.join("images.txt");
    std::fs::write(&input_path, input).map_err(|_| GalleryError::Invalid)?;
    let mut args = self.args.clone();
    args.extend([
      "-o".into(),
      format!(
        "downloader.http.headers={}",
        json!({"Referer": post.source})
      ),
      "-o".into(),
      "downloader.http.filesize-max=134217728".into(),
      "--quiet".into(),
      "--range".into(),
      "1".into(),
      "--child-filter".into(),
      "False".into(),
      "--directory".into(),
      stage.to_string_lossy().into_owned(),
      "--filename".into(),
      "asset.{extension}".into(),
      "-o".into(),
      "extractor.metadata-url=nezzel_url".into(),
      "--write-metadata".into(),
      "--postprocessor-option".into(),
      "filename=receipt.json".into(),
      "--input-file".into(),
      input_path.to_string_lossy().into_owned(),
    ]);
    let output = execute(
      &self.path,
      &args,
      subscribe_group(group),
      gallery_deadline(assets.len()),
    )
    .await?;
    if output.code != 0 {
      return Err(cdn_failure(&String::from_utf8_lossy(&output.stderr)));
    }
    assets
      .iter()
      .map(|asset| verified_image(post, asset, group, &stage.join(asset.index.to_string())))
      .collect()
  }
}

pub(super) fn verified_image(
  post: &GalleryPost,
  asset: &GalleryAsset,
  group: &str,
  stage: &Path,
) -> Result<PathBuf, GalleryError> {
  let receipt = stage.join("receipt.json");
  let metadata = std::fs::symlink_metadata(&receipt).map_err(|_| GalleryError::Invalid)?;
  if !metadata.is_file() || metadata.len() > 1024 * 1024 {
    return Err(GalleryError::Invalid);
  }
  verify_receipt(
    &std::fs::read(receipt).map_err(|_| GalleryError::Invalid)?,
    post,
    asset,
  )?;
  let mut files = std::fs::read_dir(stage)
    .map_err(|_| GalleryError::Invalid)?
    .filter_map(Result::ok)
    .filter(|entry| {
      entry.file_name().to_string_lossy().starts_with("asset.")
        && entry.file_type().is_ok_and(|kind| kind.is_file())
        && !entry.file_name().to_string_lossy().ends_with(".part")
    });
  let path = files
    .next()
    .ok_or(GalleryError::Refused("imageSourceUnavailable"))?
    .path();
  if files.next().is_some() || !is_group_running(group) {
    return Err(GalleryError::Invalid);
  }
  Ok(path)
}

pub(super) async fn filename(
  app: &AppHandle,
  entry: &DownloadEntry,
  asset: &GalleryAsset,
  bytes: &[u8],
  multiple: bool,
  post: &GalleryPost,
) -> Result<PathBuf, GalleryError> {
  let extension = image_type(bytes)?;
  let stage = Staging::new()?;
  let info = stage.json("image.json", &json!({"id":asset.item.id,"title":post.title.as_deref().unwrap_or(&asset.item.id),"uploader":post.uploader,"webpage_url":post.source,"url":asset.item.url,"ext":extension,"width":asset.item.width,"height":asset.item.height,
    "formats":[{"format_id":"image","url":asset.item.url,"ext":extension,"protocol":"https","vcodec":"unknown","acodec":"none"}]}))?;
  let output = YtdlpRunner::new(app)
    .with_location_args(
      &TrackType::Video,
      &entry.template_context,
      entry.overrides.as_ref(),
    )
    .with_args([
      "--skip-download".into(),
      "--simulate".into(),
      "--print".into(),
      "filename".into(),
      "--load-info-json".into(),
      info.to_string_lossy().into_owned(),
    ])
    .output(&entry.group_id)
    .await
    .map_err(|_| GalleryError::Invalid)?
    .ok_or(GalleryError::Cancelled)?;
  if output.code != 0 || output.stdout.len() > 32768 {
    return Err(GalleryError::Invalid);
  }
  let rendered = std::str::from_utf8(&output.stdout)
    .map_err(|_| GalleryError::Invalid)?
    .trim_end_matches(['\r', '\n']);
  if rendered.is_empty() || rendered.contains(['\r', '\n']) {
    return Err(GalleryError::Invalid);
  }
  image_destination(PathBuf::from(rendered), asset.index, multiple, extension)
}

pub(super) fn image_destination(
  path: PathBuf,
  index: usize,
  multiple: bool,
  extension: &str,
) -> Result<PathBuf, GalleryError> {
  if !path.is_absolute() || path.file_name().is_none() {
    return Err(GalleryError::Invalid);
  }
  let path = if multiple {
    path.with_file_name(format!(
      "{}-{:03}.{}",
      path.file_stem().unwrap().to_string_lossy(),
      index,
      extension
    ))
  } else {
    path.with_extension(extension)
  };
  let name = path
    .file_name()
    .ok_or(GalleryError::Invalid)?
    .to_string_lossy();
  if crate::files::sanitize::is_windows_reserved(&name) {
    Ok(path.with_file_name(format!("_{name}")))
  } else {
    Ok(path)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn image_naming_preserves_resolved_folders_and_original_carousel_position() {
    let folder = std::env::temp_dir().join("Nezzel/صور");
    let path = image_destination(folder.join("photo.custom"), 3, true, "jpg").unwrap();
    assert_eq!(path, folder.join("photo-003.jpg"));
    assert!(image_destination(PathBuf::from("relative.jpg"), 1, false, "jpg").is_err());
    for stem in [
      "CON",
      "PrN",
      "AUX",
      "nul",
      "COM1",
      "com9",
      "LPT1",
      "lpt9",
      "CON.photo",
    ] {
      assert_eq!(
        image_destination(folder.join(format!("{stem}.old")), 1, false, "jpg").unwrap(),
        folder.join(format!("_{stem}.jpg"))
      );
    }
    // Bug 5 fixed: the shared files::sanitize check also reserves these.
    for stem in [
      "COM0",
      "LPT0",
      "CONIN$",
      "CONOUT$",
      "COM\u{b9}",
      "LPT\u{b3}",
    ] {
      assert_eq!(
        image_destination(folder.join(format!("{stem}.old")), 1, false, "jpg").unwrap(),
        folder.join(format!("_{stem}.jpg"))
      );
    }
    for stem in ["COM10", "LPT10", "CONSOLE"] {
      assert_eq!(
        image_destination(folder.join(format!("{stem}.old")), 1, false, "jpg").unwrap(),
        folder.join(format!("{stem}.jpg"))
      );
    }
  }
}
