use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use super::bundled_tools;
use super::ytdlp_update::{binary_client, InstallOutcome};
use std::path::{Path, PathBuf};

use crate::binaries::binaries_extractor::{
  extract_tar_bz2, extract_tar_bz2_bundle, extract_tar_xz, extract_zip, extract_zip_bundle,
};
use crate::binaries::binaries_state::CheckResult;
use crate::paths::PathsManager;
use base64::Engine;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use fs_extra::dir::{move_dir, CopyOptions};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Error, Manager, Wry};
use tokio::fs;
use tokio::io::AsyncWriteExt;

pub(super) const BUNDLED_MANIFEST: &[u8] = include_bytes!("manifest.json");

pub(super) type AnyError = Box<dyn std::error::Error + Send + Sync + 'static>;

pub(super) const SMALL_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

fn locked_in_bytes(bytes: &[u8]) -> bool {
  let text = String::from_utf8_lossy(bytes);
  let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
  compact.contains("\"is_locked\":true")
}

pub(super) async fn read_metadata(path: &Path) -> Result<Metadata, AnyError> {
  match fs::read(path).await {
    Ok(bytes) => match serde_json::from_slice(&bytes) {
      Ok(meta) => Ok(meta),
      Err(_) => {
        // Damaged file means nothing is verified: tools get reinstalled.
        tracing::warn!("tools metadata unreadable, treating tools as not installed");
        let bad = path.with_file_name(format!("metadata.json.{}.bad", uuid::Uuid::new_v4()));
        if fs::rename(path, bad).await.is_err() {
          // Read-only directory (managed install): keep the old error path.
          return Err("tools metadata unreadable".into());
        }
        // A locked build must stay locked even when its metadata is damaged.
        Ok(Metadata {
          is_locked: locked_in_bytes(&bytes),
          ..Metadata::default()
        })
      }
    },
    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Metadata::default()),
    Err(error) => Err(error.into()),
  }
}

struct ManifestRequests {
  manifest: reqwest::Request,
  signature: reqwest::Request,
  public_key: String,
}

fn manifest_requests(
  client: &reqwest::Client,
  url: Option<&str>,
  key: Option<&str>,
) -> Result<Option<ManifestRequests>, AnyError> {
  let Some((url, key)) = url
    .map(str::trim)
    .filter(|value| !value.is_empty())
    .zip(key.map(str::trim).filter(|value| !value.is_empty()))
  else {
    return Ok(None);
  };

  let url = reqwest::Url::parse(url)?;
  if url.scheme() != "https" {
    return Err("manifest URL must use HTTPS".into());
  }
  // Reject malformed build-time keys before making any requests.
  let key_bytes: [u8; 32] = hex::decode(key)?
    .try_into()
    .map_err(|_| "manifest public key must contain 32 bytes")?;
  VerifyingKey::from_bytes(&key_bytes)?;

  Ok(Some(ManifestRequests {
    signature: client
      .get(url.join("manifest.sig")?)
      .timeout(SMALL_REQUEST_TIMEOUT)
      .build()?,
    manifest: client.get(url).timeout(SMALL_REQUEST_TIMEOUT).build()?,
    public_key: key.to_string(),
  }))
}

async fn fetch_manifest(
  client: &reqwest::Client,
  url: Option<&str>,
  key: Option<&str>,
) -> Result<Manifest, AnyError> {
  let Some(requests) = manifest_requests(client, url, key)? else {
    // This snapshot ships with the app; its hashes are trusted like the app code.
    return Ok(serde_json::from_slice(BUNDLED_MANIFEST)?);
  };
  let manifest_bytes = client
    .execute(requests.manifest)
    .await?
    .error_for_status()?
    .bytes()
    .await?;
  let signature = client
    .execute(requests.signature)
    .await?
    .error_for_status()?
    .bytes()
    .await?;
  verify_manifest(&manifest_bytes, &signature, &requests.public_key)
}

fn verify_manifest(
  manifest_bytes: &[u8],
  signature_b64: &[u8],
  public_key_hex: &str,
) -> Result<Manifest, AnyError> {
  let sig_raw = base64::engine::general_purpose::STANDARD.decode(signature_b64)?;
  let sig = Signature::from_slice(&sig_raw)?;

  let key_vec = hex::decode(public_key_hex)?;
  let key_bytes: [u8; 32] = key_vec.as_slice().try_into().map_err(|_| {
    format!(
      "invalid public key length: expected 32, got {}",
      key_vec.len()
    )
  })?;

  let vk = VerifyingKey::from_bytes(&key_bytes)?;
  vk.verify(manifest_bytes, &sig)?;

  let manifest: Manifest = serde_json::from_slice(manifest_bytes)?;
  Ok(manifest)
}

#[derive(Deserialize, Serialize)]
pub(super) struct Manifest {
  #[serde(rename = "generatedAt")]
  generated_at: String,
  pub(super) tools: IndexMap<String, ToolInfo>,
}

#[derive(Deserialize, Serialize)]
pub(super) struct ToolInfo {
  pub(super) version: String,
  pub(super) files: HashMap<String, FileInfo>,
}

impl ToolInfo {
  fn version_for(&self, platform: &str) -> &str {
    BinariesManager::select_file(&self.files, platform)
      .and_then(|(_, file)| file.version.as_deref())
      .unwrap_or(&self.version)
  }
}

#[derive(Clone, Deserialize, Serialize)]
pub(super) struct FileInfo {
  pub(super) url: String,
  pub(super) sha256: String,
  #[serde(default)]
  pub(super) version: Option<String>,
  #[serde(default)]
  entry: Option<String>,
  #[serde(default)]
  bundle: Option<BundleInfo>,
}

#[derive(Clone, Deserialize, Serialize)]
struct BundleInfo {
  #[serde(default)]
  keep_folder: bool,
  #[serde(default)]
  folder_name: Option<String>,
  entry: String,
  #[serde(default)]
  rename_entry_to: Option<String>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub(super) struct Metadata {
  pub(super) versions: HashMap<String, String>,
  pub(super) is_locked: bool,
  #[serde(default)]
  pub(super) ytdlp_last_check: Option<u64>,
  #[serde(default)]
  pub(super) ytdlp_failure_since: Option<u64>,
}

#[derive(Default, Serialize, Deserialize, Clone)]
pub(super) struct ToolError {
  tool: String,
  version: String,
  pub(super) stage: String,
  pub(super) error: String,
}

#[derive(Default, Serialize, Deserialize, Clone)]
struct ToolResult {
  successes: Vec<String>,
  failures: Vec<ToolError>,
  error: Option<String>,
}

#[derive(Default, Serialize, Deserialize, Clone)]
struct ToolStart {
  tool: String,
  version: String,
}

#[derive(Default, Serialize, Deserialize, Clone)]
struct ToolProgress {
  tool: String,
  total: u64,
  received: u64,
}

#[derive(Default, Serialize, Deserialize, Clone)]
pub(super) struct ToolComplete {
  pub(super) tool: String,
}

pub struct BinariesManager {
  pub(super) app: AppHandle<Wry>,
  pub(super) bin_dir: PathBuf,
  pub(super) operation: Arc<tokio::sync::Mutex<()>>,
  pub(super) update_check: tokio::sync::Mutex<()>,
  pub(super) maintenance: tokio::sync::OnceCell<()>,
  bundled: std::sync::OnceLock<HashMap<String, PathBuf>>,
  pub(super) verified_repairs: std::sync::Mutex<HashMap<(PathBuf, String), bool>>,
}

impl BinariesManager {
  pub fn new(app: &AppHandle<Wry>) -> Self {
    let paths_manager = app.state::<PathsManager>();
    Self {
      app: app.clone(),
      bin_dir: paths_manager.bin_dir().clone(),
      operation: Arc::new(tokio::sync::Mutex::new(())),
      update_check: tokio::sync::Mutex::new(()),
      maintenance: tokio::sync::OnceCell::new(),
      bundled: std::sync::OnceLock::new(),
      verified_repairs: std::sync::Mutex::new(HashMap::new()),
    }
  }

  /// The developer harness omits production setup; keep its consumers on this resolver too.
  pub(crate) fn for_app(app: &AppHandle<Wry>) -> tauri::State<'_, Self> {
    if app.try_state::<Self>().is_none() {
      app.manage(Self::new(app));
    }
    app.state::<Self>()
  }

  pub(super) fn bundled_paths(&self) -> &HashMap<String, PathBuf> {
    self.bundled.get_or_init(|| {
      let mut tools = HashMap::new();
      let executable = std::env::current_exe().ok();
      let resources = self.app.path().resource_dir().ok();
      // Tauri's resource helper recognizes only directories named `target`.
      // Developer examples and custom CARGO_TARGET_DIR use the Cargo output root.
      #[cfg(debug_assertions)]
      let resources = resources.or_else(|| {
        let directory = executable.as_ref()?.parent()?;
        [Some(directory), directory.parent()]
          .into_iter()
          .flatten()
          .find(|directory| directory.join(".cargo-lock").is_file())
          .map(Path::to_path_buf)
      });
      for tool in bundled_tools::TOOLS {
        let name = format!("{tool}{}", std::env::consts::EXE_SUFFIX);
        let candidates = if tool == "yt-dlp" {
          vec![resources
            .as_ref()
            .map(|dir| dir.join("tools/yt-dlp").join(&name))]
        } else {
          bundled_tools::sidecar_candidates(executable.as_deref(), resources.as_deref(), &name)
            .into_iter()
            .map(Some)
            .collect()
        };
        if let Some(path) = candidates
          .into_iter()
          .flatten()
          .find(|path| bundled_tools::executable(path))
        {
          // Bundled = exists + executable; its version is the pinned manifest version (verified at build).
          // No spawn here: a slow first launch (Gatekeeper, antivirus) must not drop the bundled copy.
          tools.insert(tool.into(), path);
        }
      }
      tools
    })
  }

  pub(super) fn resolved_tool(
    &self,
    tool: &str,
    info: &ToolInfo,
    meta: &Metadata,
  ) -> Option<PathBuf> {
    let bundled = self
      .bundled_paths()
      .get(tool)
      .filter(|path| bundled_tools::executable(path))
      .cloned();
    let version = meta.versions.get(tool);
    let data = Self::canonical_path(&self.bin_dir, tool).ok()?;
    let file =
      Self::select_file(&info.files, &Self::current_platform()).map(|(_, file)| file.clone())?;
    let eligible = bundled_tools::choose_path(
      tool,
      bundled.clone(),
      info.version_for(&Self::current_platform()),
      Some(data.clone()),
      version.map(String::as_str),
    );
    if eligible.as_ref() != Some(&data) {
      return bundled;
    }
    let version = version?;
    let key = (data.clone(), version.clone());
    let mut verified = self.verified_repairs.lock().ok()?;
    let valid = *verified.entry(key).or_insert_with(|| {
      bundled_tools::verified_repair(
        &data,
        tool,
        version,
        &file.sha256,
        info.version_for(&Self::current_platform()),
      )
    });
    if valid {
      Some(data)
    } else {
      bundled
    }
  }

  /// Resolve the same executable used by startup checks, downloads and cookie export.
  pub(crate) fn tool_path(&self, tool: &str) -> Result<PathBuf, AnyError> {
    let manifest: Manifest = serde_json::from_slice(BUNDLED_MANIFEST)?;
    let info = manifest
      .tools
      .get(tool)
      .ok_or("tool absent from manifest")?;
    let meta = std::fs::read(self.bin_dir.join("metadata.json"))
      .ok()
      .and_then(|bytes| serde_json::from_slice(&bytes).ok())
      .unwrap_or_default();
    self
      .resolved_tool(tool, info, &meta)
      .ok_or_else(|| format!("{tool} requires repair").into())
  }

  /// (tool, version) of the copy in use for problem reports; "missing" when none resolves.
  pub(crate) fn tool_versions(&self) -> Vec<(String, String)> {
    let Ok(manifest) = serde_json::from_slice::<Manifest>(BUNDLED_MANIFEST) else {
      return Vec::new();
    };
    let meta: Metadata = std::fs::read(self.bin_dir.join("metadata.json"))
      .ok()
      .and_then(|bytes| serde_json::from_slice(&bytes).ok())
      .unwrap_or_default();
    manifest
      .tools
      .iter()
      .map(|(tool, info)| {
        let data = Self::canonical_path(&self.bin_dir, tool).ok();
        let version = match self.resolved_tool(tool, info, &meta) {
          Some(path) if data.as_ref() == Some(&path) => meta
            .versions
            .get(tool)
            .cloned()
            .unwrap_or_else(|| "unknown".into()),
          Some(_) => info.version_for(&Self::current_platform()).to_string(),
          None => "missing".into(),
        };
        (tool.clone(), version)
      })
      .collect()
  }

  /// Use resolved helper locations rather than the host's ffmpeg or JavaScript runtime.
  pub(crate) fn tool_environment(&self) -> Result<std::ffi::OsString, AnyError> {
    let mut directories = Vec::new();
    for tool in bundled_tools::TOOLS {
      if let Ok(path) = self.tool_path(tool) {
        if let Some(parent) = path.parent() {
          directories.push(parent.to_path_buf());
        }
      }
    }
    directories.push(self.bin_dir.clone());
    directories.extend(std::env::split_paths(
      &std::env::var_os("PATH").unwrap_or_default(),
    ));
    Ok(std::env::join_paths(directories)?)
  }

  fn repair_plan<'a>(
    &self,
    manifest: &'a Manifest,
    meta: &Metadata,
    allow: Option<&[String]>,
  ) -> Vec<(&'a str, &'a ToolInfo)> {
    manifest
      .tools
      .iter()
      .filter(|(name, info)| {
        // AtomicParsley is optional; yt-dlp embeds metadata using its bundled mutagen.
        let requested = allow.map_or_else(
          || bundled_tools::TOOLS.contains(&name.as_str()),
          |allow| allow.contains(name),
        );
        requested
          && Self::select_file(&info.files, &Self::current_platform()).is_some()
          && self.resolved_tool(name, info, meta).is_none()
      })
      .map(|(name, info)| (name.as_str(), info))
      .collect()
  }

  pub(super) fn client(&self) -> Result<Option<reqwest::Client>, AnyError> {
    let cfg = self.app.state::<crate::SharedConfig>().load();
    binary_client(cfg.network.enable_proxy, cfg.network.proxy.as_deref())
  }

  fn canonical_path(base: &Path, tool: &str) -> Result<PathBuf, Error> {
    if tool == "yt-dlp" {
      return Ok(super::ytdlp_update::executable(base));
    }
    Ok(base.join(bundled_tools::exe_name(tool)))
  }

  pub(super) fn current_platform() -> String {
    let os = match std::env::consts::OS {
      "macos" => "darwin",
      other => other,
    };
    format!("{}-{}", os, std::env::consts::ARCH)
  }

  pub(super) fn select_file<'a>(
    files: &'a HashMap<String, FileInfo>,
    arch: &str,
  ) -> Option<(&'a str, &'a FileInfo)> {
    if let Some((key, file)) = files.get_key_value(arch) {
      return Some((key.as_str(), file));
    }
    let platform = arch.split('-').next().unwrap_or("");
    files
      .iter()
      .find(|(k, _)| k.starts_with(platform))
      .map(|(k, v)| (k.as_str(), v))
  }

  pub async fn check(&self) -> Result<CheckResult, AnyError> {
    self.maintain_ytdlp().await;
    if self
      .load_metadata(&self.bin_dir.join("metadata.json"))
      .await?
      .is_locked
    {
      return Ok(CheckResult { tools: Vec::new() });
    }
    let manifest: Manifest = serde_json::from_slice(BUNDLED_MANIFEST)?;
    let _operation = self.operation.lock().await;
    let meta = self
      .load_metadata(&self.bin_dir.join("metadata.json"))
      .await?;
    if meta.is_locked {
      return Ok(CheckResult { tools: Vec::new() });
    }
    let plan = self.repair_plan(&manifest, &meta, None);
    Ok(CheckResult {
      tools: plan.iter().map(|(name, _)| (*name).to_string()).collect(),
    })
  }

  pub async fn ensure(&self, allow: Option<&[String]>) -> Result<(), AnyError> {
    self.maintain_ytdlp().await;
    let Some(client) = self.client()? else {
      return Ok(());
    };
    let bin = &self.bin_dir;
    fs::create_dir_all(bin).await?;
    let meta_path = bin.join("metadata.json");
    let arch = Self::current_platform();
    if self.load_metadata(&meta_path).await?.is_locked {
      return Ok(());
    }
    let manifest: Manifest = serde_json::from_slice(BUNDLED_MANIFEST)?;
    let plan = {
      let _operation = self.operation.lock().await;
      let meta = self.load_metadata(&meta_path).await?;
      if meta.is_locked {
        return Ok(());
      }
      self.repair_plan(&manifest, &meta, allow)
    };
    if plan.is_empty() {
      return Ok(());
    }
    let mut successes = Vec::new();
    let mut failures = Vec::new();
    for (name, info) in plan {
      let result = if name == "yt-dlp" {
        match self.install_ytdlp(&client, &arch, info, None).await {
          Ok(InstallOutcome::Installed(_) | InstallOutcome::UpToDate) => Ok(()),
          Ok(InstallOutcome::Busy | InstallOutcome::Locked) => self.fail_stage(
            name,
            &info.version,
            "busy",
            "yt-dlp install deferred".into(),
          ),
          Err(error) => self.fail_stage(name, &info.version, "install", error.to_string()),
        }
      } else {
        match self
          .install_single_tool(&client, bin, &arch, name, info)
          .await
        {
          Ok(()) => {
            // Merge into fresh metadata; never overwrite a concurrent updater's versions/check time.
            let _operation = self.operation.lock().await;
            let mut meta = self.load_metadata(&meta_path).await?;
            meta
              .versions
              .insert(name.to_string(), info.version_for(&arch).to_owned());
            self.save_metadata(&meta_path, &meta).map_err(|error| {
              self
                .fail_stage(name, &info.version, "metadata", error.to_string())
                .unwrap_err()
            })
          }
          Err(error) => Err(error),
        }
      };
      match result {
        Ok(()) => successes.push(name.to_string()),
        Err(error) => failures.push(error),
      }
    }
    let failed = !failures.is_empty();
    let _ = self.app.emit(
      "binary_update_complete",
      ToolResult {
        successes,
        failures,
        error: None,
      },
    );
    if failed {
      return Err("one or more tools failed to install".into());
    }
    Ok(())
  }

  pub(super) async fn install_single_tool(
    &self,
    client: &reqwest::Client,
    bin: &Path,
    arch: &str,
    name: &str,
    info: &ToolInfo,
  ) -> Result<(), ToolError> {
    let version = info.version_for(arch);
    let Some((_key, file)) = Self::select_file(&info.files, arch) else {
      return self.fail_stage(
        name,
        version,
        "select_file",
        "no compatible file for current platform".to_string(),
      );
    };

    let url = &file.url;
    let Some(filename) = url.rsplit('/').next() else {
      return self.fail_stage(
        name,
        version,
        "parse_filename",
        "missing file name in URL".to_string(),
      );
    };
    let dest = bin.join(filename);

    let _ = self.app.emit(
      "binary_download_start",
      ToolStart {
        tool: name.to_string(),
        version: version.to_string(),
      },
    );

    if let Err(e) = self.download_and_verify(client, name, file, &dest).await {
      return self.fail_stage(name, version, "download_verify", e.to_string());
    }

    let canonical = Self::canonical_path(bin, name).map_err(|e| {
      self
        .fail_stage(name, version, "canonical_path", e.to_string())
        .err()
        .unwrap()
    })?;

    if tokio::fs::metadata(&canonical).await.is_err() {
      let err = format!(
        "canonical binary missing after install: {}",
        canonical.display()
      );
      return self.fail_stage(name, version, "post_install_check", err);
    }

    let root = canonical.parent().ok_or_else(|| ToolError {
      tool: name.into(),
      version: version.to_owned(),
      stage: "integrity".into(),
      error: "missing tool directory".into(),
    })?;
    bundled_tools::record_repair(root, name, &canonical, version, &file.sha256).map_err(
      |error| {
        self
          .fail_stage(name, version, "integrity", error.to_string())
          .unwrap_err()
      },
    )?;
    if let Ok(mut verified) = self.verified_repairs.lock() {
      verified.clear();
    }
    if name != "yt-dlp" {
      let _ = self.app.emit(
        "binary_download_complete",
        ToolComplete {
          tool: name.to_string(),
        },
      );
    }

    Ok(())
  }

  fn fail_stage(
    &self,
    name: &str,
    version: &str,
    stage: &str,
    msg: String,
  ) -> Result<(), ToolError> {
    self.emit_tool_error(name, version, stage, &msg);
    Err(ToolError {
      tool: name.to_string(),
      version: version.to_string(),
      stage: stage.to_string(),
      error: msg,
    })
  }

  pub(super) async fn fetch_manifest(
    &self,
    client: &reqwest::Client,
  ) -> Result<Manifest, AnyError> {
    fetch_manifest(
      client,
      option_env!("NEZZEL_MANIFEST_URL"),
      option_env!("NEZZEL_MANIFEST_PUBKEY"),
    )
    .await
  }

  async fn download_and_verify(
    &self,
    client: &reqwest::Client,
    tool: &str,
    info: &FileInfo,
    dest: &Path,
  ) -> Result<(), AnyError> {
    let url = &info.url;
    let sha256 = info.sha256.as_str();
    let entry = info.entry.as_deref();
    let bundle = info.bundle.as_ref();
    let mut res = client
      .get(url)
      .timeout(Duration::from_secs(15 * 60))
      .send()
      .await?
      .error_for_status()?;

    let tmp = dest.with_extension("tmp");
    let mut file = fs::File::create(&tmp).await?;
    let mut hasher = Sha256::new();

    let total = res.content_length().unwrap_or(0);
    let mut received: u64 = 0;

    while let Some(chunk) = res.chunk().await? {
      received += chunk.len() as u64;
      hasher.update(&chunk);
      file.write_all(&chunk).await?;
      let _ = self.app.emit(
        "binary_download_progress",
        ToolProgress {
          tool: tool.to_string(),
          received,
          total,
        },
      );
    }

    let hash = hex::encode(hasher.finalize());
    if hash != sha256 {
      return Err("sha256 mismatch".into());
    }

    file.flush().await?;
    drop(file);
    fs::rename(&tmp, dest).await?;

    let parent = dest.parent().ok_or("binary destination has no parent")?;
    let canonical = Self::canonical_path(parent, tool)?;

    if let Some(ext) = dest.extension().and_then(|e| e.to_str()) {
      match ext {
        "zip" => {
          if let Some(b) = bundle {
            let (dir, _entry_abs) = extract_zip_bundle(
              dest,
              parent,
              b.folder_name.as_deref(),
              Path::new(&b.entry),
              b.rename_entry_to.as_deref(),
            )
            .await?;
            fs::remove_file(dest).await?;
            self
              .hoist_bundle_contents_into_bin(&dir, &canonical)
              .await?;
          } else {
            extract_zip(tool, canonical.clone(), dest, parent, entry).await?;
            fs::remove_file(dest).await?;
          }
        }
        "xz" => {
          extract_tar_xz(tool, canonical.clone(), dest, parent, entry).await?;
          fs::remove_file(dest).await?;
        }
        "bz2" => {
          if let Some(b) = bundle {
            let (dir, _entry_abs) = extract_tar_bz2_bundle(
              dest,
              parent,
              b.folder_name.as_deref(),
              Path::new(&b.entry),
              b.rename_entry_to.as_deref(),
            )
            .await?;
            fs::remove_file(dest).await?;
            self
              .hoist_bundle_contents_into_bin(&dir, &canonical)
              .await?;
          } else {
            extract_tar_bz2(tool, canonical.clone(), dest, parent, entry).await?;
            fs::remove_file(dest).await?;
          }
        }
        _ => {
          fs::rename(dest, &canonical).await?;
        }
      }
    } else {
      fs::rename(dest, &canonical).await?;
    }

    #[cfg(unix)]
    {
      use std::os::unix::fs::PermissionsExt;
      let mut perms = std::fs::metadata(&canonical)?.permissions();
      if perms.mode() & 0o111 == 0 {
        perms.set_mode(0o755);
        std::fs::set_permissions(&canonical, perms)?;
      }
    }

    Ok(())
  }

  pub(super) async fn load_metadata(&self, path: &Path) -> Result<Metadata, AnyError> {
    read_metadata(path).await
  }

  pub(super) fn save_metadata(&self, path: &Path, meta: &Metadata) -> Result<(), AnyError> {
    write_metadata(path, &serde_json::to_vec(meta)?)?;
    Ok(())
  }

  fn emit_tool_error(&self, tool: &str, version: &str, stage: &str, error: impl Into<String>) {
    let _ = self.app.emit(
      "binary_download_error",
      ToolError {
        tool: tool.to_string(),
        version: version.to_string(),
        stage: stage.to_string(),
        error: error.into(),
      },
    );
  }

  pub async fn hoist_bundle_contents_into_bin(
    &self,
    bundle_root: &Path,
    canonical: &Path,
  ) -> Result<(), AnyError> {
    let bin_dir = canonical
      .parent()
      .ok_or("canonical has no parent directory")?
      .to_path_buf();

    tokio::fs::create_dir_all(&bin_dir).await?;

    let src = bundle_root.to_path_buf();
    let dest = bin_dir.clone();

    tokio::task::spawn_blocking(move || {
      let mut opts = CopyOptions::new();
      opts.overwrite = true;
      opts.copy_inside = false;
      opts.content_only = true;
      move_dir(&src, &dest, &opts)?;
      Ok::<(), fs_extra::error::Error>(())
    })
    .await??;

    let _ = fs::remove_dir_all(bundle_root).await;

    Ok(())
  }
}

pub(super) fn write_metadata(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
  let temporary = path.with_extension("json.tmp");
  std::fs::write(&temporary, bytes)?;
  std::fs::rename(temporary, path)
}

#[cfg(test)]
mod tests {
  use super::{fetch_manifest, manifest_requests, read_metadata, verify_manifest, BinariesManager};
  use base64::{engine::general_purpose::STANDARD, Engine};
  use ed25519_dalek::{SignatureError, Signer, SigningKey};

  #[tokio::test]
  async fn damaged_metadata_reads_as_nothing_installed_and_is_set_aside() {
    let dir = std::env::temp_dir().join(format!("nezzel-meta-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("metadata.json");
    std::fs::write(&path, b"{not json").unwrap();
    let meta = read_metadata(&path).await.unwrap();
    assert!(meta.versions.is_empty() && !meta.is_locked);
    let set_aside = std::fs::read_dir(&dir)
      .unwrap()
      .filter_map(Result::ok)
      .any(|e| e.file_name().to_string_lossy().ends_with(".bad"));
    assert!(set_aside);
    assert!(!path.exists());
    std::fs::write(&path, br#"{"versions":{"yt-dlp":"1"},"is_locked": true,"#).unwrap();
    assert!(read_metadata(&path).await.unwrap().is_locked);
    // A restart must preserve a seven-day stale streak even before a due update runs.
    let day = 86400;
    let read_stale = |now| {
      super::super::binaries_state::BinariesState::from_metadata(&path, now)
        .update_stale
        .load(std::sync::atomic::Ordering::SeqCst)
    };
    for (contents, now, stale) in [
      (
        br#"{"versions":{},"is_locked":false,"ytdlp_failure_since":100}"#.as_slice(),
        100 + 7 * day,
        true,
      ),
      (
        br#"{"versions":{},"is_locked":false,"ytdlp_failure_since":100}"#.as_slice(),
        99 + 7 * day,
        false,
      ),
      (
        br#"{"versions":{},"is_locked":false,"ytdlp_failure_since":null}"#.as_slice(),
        100 + 8 * day,
        false,
      ),
      (
        br#"{"versions":{},"is_locked":true,"ytdlp_failure_since":100}"#.as_slice(),
        100 + 8 * day,
        false,
      ),
      (b"{}".as_slice(), 100 + 8 * day, false),
      (b"{broken".as_slice(), 100 + 8 * day, false),
    ] {
      std::fs::write(&path, contents).unwrap();
      assert_eq!(read_stale(now), stale);
    }
    std::fs::remove_dir_all(dir).unwrap();
  }

  const MANIFEST: &[u8] = br#"{"generatedAt":"2026-11-02","tools":{}}"#;

  #[test]
  fn unset_manifest_env_builds_no_remote_requests() {
    let client = reqwest::Client::new();
    for (url, key) in [
      (None, None),
      (Some(""), Some("")),
      (Some("https://updates.nezzel.test/manifest.json"), None),
      (None, Some("invalid-but-unused-key")),
      (Some(" \n"), Some("invalid-but-unused-key")),
      (Some("https://updates.nezzel.test/manifest.json"), Some(" ")),
    ] {
      assert!(manifest_requests(&client, url, key).unwrap().is_none());
    }
  }

  #[test]
  fn configured_manifest_builds_only_our_manifest_and_signature_requests() {
    let client = reqwest::Client::new();
    let key = hex::encode(signing_key().verifying_key().as_bytes());
    let requests = manifest_requests(
      &client,
      Some("https://updates.nezzel.test/tools/manifest.json"),
      Some(&key),
    )
    .unwrap()
    .unwrap();
    assert_eq!(
      requests.manifest.url().as_str(),
      "https://updates.nezzel.test/tools/manifest.json"
    );
    assert_eq!(
      requests.signature.url().as_str(),
      "https://updates.nezzel.test/tools/manifest.sig"
    );
    assert_eq!(requests.public_key, key);
  }

  #[test]
  fn invalid_manifest_configuration_is_rejected_before_requests() {
    let client = reqwest::Client::new();
    let key = hex::encode(signing_key().verifying_key().as_bytes());
    for (url, key) in [
      ("http://updates.nezzel.test/manifest.json", key.as_str()),
      ("https://updates.nezzel.test/manifest.json", "not-a-key"),
      ("https://updates.nezzel.test/manifest.json", "ab"),
    ] {
      assert!(manifest_requests(&client, Some(url), Some(key)).is_err());
    }
  }

  #[test]
  fn platform_versions_override_the_shared_tool_label() {
    let info: super::ToolInfo = serde_json::from_value(serde_json::json!({
      "version": "mixed", "files": {
        "darwin-aarch64": {"url": "https://example.test/mac.zip", "sha256": "hash", "version": "9.0.2"},
        "linux-x86_64": {"url": "https://example.test/linux.tar.xz", "sha256": "hash", "version": "N-128280-ga88838af48"},
        "windows-aarch64": {"url": "https://example.test/win.zip", "sha256": "hash"}
      }
    })).unwrap();
    assert_eq!(info.version_for("darwin-aarch64"), "9.0.2");
    assert_eq!(info.version_for("linux-x86_64"), "N-128280-ga88838af48");
    assert_eq!(info.version_for("windows-aarch64"), "mixed");
  }

  #[tokio::test]
  async fn local_manifest_pins_assets_for_every_platform() {
    // No server or network fixture: the same fetch path used by check/ensure
    // must return the embedded manifest with the remote configuration absent.
    let manifest = fetch_manifest(&reqwest::Client::new(), None, None)
      .await
      .unwrap();
    let bin = std::env::temp_dir().join(format!("nezzel-manifest-{}", uuid::Uuid::new_v4()));
    tokio::fs::create_dir_all(&bin).await.unwrap();
    for platform in [
      "darwin-aarch64",
      "darwin-x86_64",
      "linux-aarch64",
      "linux-x86_64",
      "windows-aarch64",
      "windows-x86_64",
    ] {
      for tool in ["yt-dlp", "ffmpeg", "ffprobe", "deno", "AtomicParsley"] {
        let info = &manifest.tools[tool];
        // Require an exact platform entry; don't let the compatibility fallback
        // hide an omitted architecture in the shipping snapshot. AtomicParsley
        // retains the existing source map's x86_64 macOS/Linux fallback.
        let file = if tool == "AtomicParsley" {
          let (selected, file) = BinariesManager::select_file(&info.files, platform).unwrap();
          let (expected_platform, expected_entry) = match platform {
            "darwin-aarch64" | "darwin-x86_64" => ("darwin-x86_64", "AtomicParsley"),
            "linux-aarch64" | "linux-x86_64" => ("linux-x86_64", "AtomicParsley"),
            other => (other, "AtomicParsley.exe"),
          };
          assert_eq!(selected, expected_platform);
          assert_eq!(file.entry.as_deref(), Some(expected_entry));
          file
        } else {
          &info.files[platform]
        };
        let url = reqwest::Url::parse(&file.url).unwrap();
        assert_eq!(url.scheme(), "https");
        assert!(["github.com", "ffmpeg.martin-riedl.de"].contains(&url.host_str().unwrap()));
        if url.host_str() == Some("github.com") {
          assert!(
            ["yt-dlp", "denoland", "wez"].contains(&url.path_segments().unwrap().next().unwrap())
          );
        }
        assert!(!url.path().contains("/latest/"));
        assert_eq!(hex::decode(&file.sha256).unwrap().len(), 32);
        assert!(!info.version.is_empty());
      }
    }
    tokio::fs::remove_dir_all(bin).await.unwrap();
  }

  fn signing_key() -> SigningKey {
    SigningKey::from_bytes(&rand::random())
  }

  fn signature(key: &SigningKey) -> String {
    STANDARD.encode(key.sign(MANIFEST).to_bytes())
  }

  #[test]
  fn accepts_a_manifest_signed_by_the_trusted_key() {
    let key = signing_key();
    let manifest = verify_manifest(
      MANIFEST,
      signature(&key).as_bytes(),
      &hex::encode(key.verifying_key().as_bytes()),
    )
    .unwrap();

    assert_eq!(manifest.generated_at, "2026-11-02");
    assert!(manifest.tools.is_empty());
  }

  #[test]
  fn rejects_a_tampered_manifest_before_deserializing_it() {
    let key = signing_key();
    let error = verify_manifest(
      br#"{"generatedAt":"2025-11-02","tools":{}}"#,
      signature(&key).as_bytes(),
      &hex::encode(key.verifying_key().as_bytes()),
    )
    .err()
    .expect("a signature must not authenticate different manifest bytes");

    assert!(error.downcast_ref::<SignatureError>().is_some());
  }

  #[test]
  fn rejects_a_signature_from_a_different_key() {
    let signer = signing_key();
    let trusted = signing_key();
    let error = verify_manifest(
      MANIFEST,
      signature(&signer).as_bytes(),
      &hex::encode(trusted.verifying_key().as_bytes()),
    )
    .err()
    .expect("an untrusted signer must not authenticate a manifest");

    assert!(error.downcast_ref::<SignatureError>().is_some());
  }

  #[test]
  fn rejects_a_malformed_base64_signature() {
    let trusted = signing_key();
    let error = verify_manifest(
      MANIFEST,
      b"not!base64",
      &hex::encode(trusted.verifying_key().as_bytes()),
    )
    .err()
    .expect("malformed signatures must be rejected");

    assert!(error.downcast_ref::<base64::DecodeError>().is_some());
  }
}
