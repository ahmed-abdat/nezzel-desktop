use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::ytdlp_update::{
  check_due, checksum_for, cleanup_leftovers, cleanup_legacy, compare_versions, recover_bundle,
  redirect_tag, swap_bundle, CURRENT_DIR, LATEST_URL,
};
use super::{bundled_tools, YTDLP_PROCESSES};
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

const BUNDLED_MANIFEST: &[u8] = include_bytes!("manifest.json");

type AnyError = Box<dyn std::error::Error + Send + Sync + 'static>;

#[derive(Clone, Copy)]
struct CheckAttempt {
  checked_at: u64,
  previous: Option<u64>,
}

#[derive(Debug, PartialEq, Eq)]
enum InstallOutcome {
  Installed(Option<String>),
  UpToDate,
  Busy,
  Locked,
}

const SMALL_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

fn binary_client(
  enabled: Option<bool>,
  proxy: Option<&str>,
) -> Result<Option<reqwest::Client>, AnyError> {
  let mut builder = reqwest::Client::builder()
    .https_only(true)
    .connect_timeout(Duration::from_secs(15))
    .read_timeout(Duration::from_secs(30))
    .redirect(reqwest::redirect::Policy::custom(|attempt| {
      if attempt
        .previous()
        .first()
        .is_some_and(|url| url.as_str() == LATEST_URL)
      {
        attempt.stop()
      } else {
        reqwest::redirect::Policy::limited(10).redirect(attempt)
      }
    }));
  if enabled == Some(true) {
    let Some(proxy) = proxy.filter(|value| !value.is_empty()) else {
      return Ok(None);
    };
    builder =
      builder.proxy(reqwest::Proxy::all(proxy).map_err(|_| "invalid binary proxy setting")?);
  }
  Ok(Some(builder.build()?))
}

async fn read_metadata(path: &Path) -> Result<Metadata, AnyError> {
  match fs::read(path).await {
    Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Metadata::default()),
    Err(error) => Err(error.into()),
  }
}

async fn commit_ytdlp(
  bin: &Path,
  staged: &Path,
  version: &str,
  attempt: Option<CheckAttempt>,
  operation: Arc<tokio::sync::Mutex<()>>,
  processes: Arc<tokio::sync::RwLock<()>>,
) -> Result<InstallOutcome, AnyError> {
  let operation = operation.lock_owned().await;
  let meta_path = bin.join("metadata.json");
  let mut meta = read_metadata(&meta_path).await?;
  if meta.is_locked {
    return Ok(InstallOutcome::Locked);
  }
  // Another installer may have won while the archive was downloading.
  if meta.versions.get("yt-dlp").is_some_and(|installed| {
    compare_versions(installed, version).is_some_and(|order| order != Ordering::Less)
      && bundled_tools::verified_repair(
        &super::ytdlp_update::executable(bin),
        "yt-dlp",
        installed,
        "",
        "",
      )
  }) {
    return Ok(InstallOutcome::UpToDate);
  }
  let Ok(process_guard) = processes.try_write_owned() else {
    if let Some(attempt) =
      attempt.filter(|attempt| meta.ytdlp_last_check == Some(attempt.checked_at))
    {
      meta.ytdlp_last_check = attempt.previous;
      write_metadata(&meta_path, &serde_json::to_vec(&meta)?)?;
    }
    return Ok(InstallOutcome::Busy);
  };
  let old = meta.versions.insert("yt-dlp".into(), version.into());
  let bytes = serde_json::to_vec(&meta)?;
  let current = bin.join(CURRENT_DIR);
  let source = staged.to_path_buf();
  let bin = bin.to_path_buf();
  // Keep both locks through rollback/cleanup even if the async caller is cancelled.
  tokio::task::spawn_blocking(move || {
    let _operation = operation;
    let _process_guard = process_guard;
    swap_bundle(&source, &current, || write_metadata(&meta_path, &bytes))?;
    for result in [cleanup_leftovers(&current), cleanup_legacy(&bin)] {
      if let Err(error) = result {
        tracing::info!("yt-dlp cleanup deferred: {error}");
      }
    }
    Ok::<_, std::io::Error>(())
  })
  .await??;
  Ok(InstallOutcome::Installed(old))
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
struct Manifest {
  #[serde(rename = "generatedAt")]
  generated_at: String,
  tools: IndexMap<String, ToolInfo>,
}

#[derive(Deserialize, Serialize)]
struct ToolInfo {
  version: String,
  files: HashMap<String, FileInfo>,
}

impl ToolInfo {
  fn version_for(&self, platform: &str) -> &str {
    BinariesManager::select_file(&self.files, platform)
      .and_then(|(_, file)| file.version.as_deref())
      .unwrap_or(&self.version)
  }
}

#[derive(Clone, Deserialize, Serialize)]
struct FileInfo {
  url: String,
  sha256: String,
  #[serde(default)]
  version: Option<String>,
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
struct Metadata {
  versions: HashMap<String, String>,
  is_locked: bool,
  #[serde(default)]
  ytdlp_last_check: Option<u64>,
}

#[derive(Default, Serialize, Deserialize, Clone)]
struct ToolError {
  tool: String,
  version: String,
  stage: String,
  error: String,
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
struct ToolComplete {
  tool: String,
}

pub struct BinariesManager {
  app: AppHandle<Wry>,
  bin_dir: PathBuf,
  operation: Arc<tokio::sync::Mutex<()>>,
  maintenance: tokio::sync::OnceCell<()>,
  bundled: std::sync::OnceLock<HashMap<String, PathBuf>>,
  verified_repairs: std::sync::Mutex<HashMap<(PathBuf, String), bool>>,
}

impl BinariesManager {
  pub fn new(app: &AppHandle<Wry>) -> Self {
    let paths_manager = app.state::<PathsManager>();
    Self {
      app: app.clone(),
      bin_dir: paths_manager.bin_dir().clone(),
      operation: Arc::new(tokio::sync::Mutex::new(())),
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

  fn bundled_paths(&self) -> &HashMap<String, PathBuf> {
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
          vec![
            executable
              .as_ref()
              .and_then(|exe| exe.parent())
              .map(|dir| dir.join(&name)),
            resources.as_ref().map(|dir| dir.join(&name)),
          ]
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

  fn resolved_tool(&self, tool: &str, info: &ToolInfo, meta: &Metadata) -> Option<PathBuf> {
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

  fn client(&self) -> Result<Option<reqwest::Client>, AnyError> {
    let cfg = self.app.state::<crate::SharedConfig>().load();
    binary_client(cfg.network.enable_proxy, cfg.network.proxy.as_deref())
  }

  async fn maintain_ytdlp(&self) {
    self
      .maintenance
      .get_or_init(|| async {
        let operation = self.operation.clone().lock_owned().await;
        match self
          .load_metadata(&self.bin_dir.join("metadata.json"))
          .await
        {
          Ok(meta) if meta.is_locked => return,
          Err(error) => {
            tracing::info!("yt-dlp maintenance deferred: {error}");
            return;
          }
          _ => {}
        }
        let Ok(process_guard) = YTDLP_PROCESSES.clone().try_write_owned() else {
          return;
        };
        let bin = self.bin_dir.clone();
        let result = tokio::task::spawn_blocking(move || {
          let _operation = operation;
          let _process_guard = process_guard;
          std::fs::create_dir_all(&bin)?;
          recover_bundle(&bin.join(CURRENT_DIR))?;
          // This runs once, before any prepare call can create a live stage directory.
          cleanup_leftovers(&bin)?;
          cleanup_legacy(&bin)
        })
        .await;
        match result {
          Ok(Ok(())) => {}
          Ok(Err(error)) => tracing::info!("yt-dlp maintenance deferred: {error}"),
          Err(error) => tracing::info!("yt-dlp maintenance failed: {error}"),
        }
      })
      .await;
  }

  fn canonical_path(base: &Path, tool: &str) -> Result<PathBuf, Error> {
    if tool == "yt-dlp" {
      return Ok(super::ytdlp_update::executable(base));
    }
    #[cfg(windows)]
    {
      Ok(base.join(format!("{tool}.exe")))
    }
    #[cfg(not(windows))]
    {
      Ok(base.join(tool))
    }
  }

  fn current_platform() -> String {
    let os = match std::env::consts::OS {
      "macos" => "darwin",
      other => other,
    };
    format!("{}-{}", os, std::env::consts::ARCH)
  }

  fn select_file<'a>(
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

  /// Spawn daily work without awaiting it from startup.
  pub fn schedule_ytdlp_update(app: &AppHandle<Wry>) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
      let manager = app.state::<Self>();
      if let Err(error) = manager.update_ytdlp().await {
        tracing::info!("yt-dlp update failed: {error}");
      }
    });
  }

  async fn update_ytdlp(&self) -> Result<(), AnyError> {
    self.maintain_ytdlp().await;
    let Some(client) = self.client()? else {
      return Ok(());
    };
    let (old, attempt) = {
      let _operation = self.operation.lock().await;
      if !self
        .app
        .state::<crate::SharedConfig>()
        .load()
        .update
        .update_binaries
      {
        return Ok(());
      }
      let meta_path = self.bin_dir.join("metadata.json");
      let mut meta = self.load_metadata(&meta_path).await?;
      let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
      if meta.is_locked || !check_due(meta.ytdlp_last_check, now) {
        return Ok(());
      }
      let manifest: Manifest = serde_json::from_slice(BUNDLED_MANIFEST)?;
      let Some(info) = manifest.tools.get("yt-dlp") else {
        return Ok(());
      };
      let Some(path) = self.resolved_tool("yt-dlp", info, &meta) else {
        return Ok(());
      };
      let old = if self.bundled_paths().get("yt-dlp") == Some(&path) {
        info.version.clone()
      } else {
        meta
          .versions
          .get("yt-dlp")
          .cloned()
          .ok_or("yt-dlp version missing")?
      };
      if YTDLP_PROCESSES.try_write().is_err() {
        tracing::info!("yt-dlp update deferred: process active");
        return Ok(());
      }
      let attempt = CheckAttempt {
        checked_at: now,
        previous: meta.ytdlp_last_check,
      };
      // Persist failures too, but never hold the operation mutex during network work.
      meta.ytdlp_last_check = Some(now);
      self.save_metadata(&meta_path, &meta)?;
      (old, attempt)
    };
    let response = client
      .get(LATEST_URL)
      .timeout(SMALL_REQUEST_TIMEOUT)
      .send()
      .await?
      .error_for_status()?;
    let tag = redirect_tag(
      response.status().as_u16(),
      response
        .headers()
        .get(reqwest::header::LOCATION)
        .and_then(|value| value.to_str().ok()),
    )?
    .to_string();
    if compare_versions(&tag, &old).ok_or("invalid installed yt-dlp version")? != Ordering::Greater
    {
      tracing::info!("yt-dlp up to date");
      return Ok(());
    }
    let manifest = self.fetch_manifest(&client).await?;
    let arch = Self::current_platform();
    let pinned = manifest
      .tools
      .get("yt-dlp")
      .ok_or("yt-dlp absent from manifest")?;
    let (_, pinned_file) =
      Self::select_file(&pinned.files, &arch).ok_or("no yt-dlp asset for platform")?;
    let asset = pinned_file
      .url
      .rsplit('/')
      .next()
      .filter(|name| !name.is_empty())
      .ok_or("yt-dlp asset name missing")?;
    if !asset
      .bytes()
      .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
    {
      return Err("invalid yt-dlp asset name".into());
    }
    let base = format!("https://github.com/yt-dlp/yt-dlp/releases/download/{tag}");
    let sums = client
      .get(format!("{base}/SHA2-256SUMS"))
      .timeout(SMALL_REQUEST_TIMEOUT)
      .send()
      .await?
      .error_for_status()?
      .text()
      .await?;
    let mut file = pinned_file.clone();
    file.sha256 = checksum_for(&sums, asset).ok_or("yt-dlp asset checksum absent or invalid")?;
    file.url = format!("{base}/{asset}");
    file.version = None;
    let info = ToolInfo {
      version: tag.clone(),
      files: HashMap::from([(arch.clone(), file)]),
    };
    match self
      .install_ytdlp(&client, &arch, &info, Some(attempt))
      .await?
    {
      InstallOutcome::Installed(previous) => {
        tracing::info!("yt-dlp updated {} -> {tag}", previous.unwrap_or(old))
      }
      InstallOutcome::UpToDate => tracing::info!("yt-dlp up to date"),
      InstallOutcome::Busy => tracing::info!("yt-dlp update deferred: process active"),
      InstallOutcome::Locked => tracing::info!("yt-dlp update deferred: metadata locked"),
    }
    Ok(())
  }

  async fn install_ytdlp(
    &self,
    client: &reqwest::Client,
    arch: &str,
    info: &ToolInfo,
    attempt: Option<CheckAttempt>,
  ) -> Result<InstallOutcome, AnyError> {
    let staged = self
      .bin_dir
      .join(format!(".yt-dlp-stage-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&staged).await?;
    let result = async {
      // Network and extraction prepare an isolated directory without the operation mutex.
      self
        .install_single_tool(client, &staged, arch, "yt-dlp", info)
        .await
        .map_err(|error| format!("{}: {}", error.stage, error.error))?;
      let outcome = commit_ytdlp(
        &self.bin_dir,
        &staged,
        &info.version,
        attempt,
        self.operation.clone(),
        YTDLP_PROCESSES.clone(),
      )
      .await?;
      if let Ok(mut verified) = self.verified_repairs.lock() {
        verified.clear();
      }
      if matches!(
        outcome,
        InstallOutcome::Installed(_) | InstallOutcome::UpToDate
      ) {
        let _ = self.app.emit(
          "binary_download_complete",
          ToolComplete {
            tool: "yt-dlp".into(),
          },
        );
      }
      Ok::<_, AnyError>(outcome)
    }
    .await;
    let _ = fs::remove_dir_all(staged).await;
    result
  }

  async fn install_single_tool(
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

  async fn fetch_manifest(&self, client: &reqwest::Client) -> Result<Manifest, AnyError> {
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

  async fn load_metadata(&self, path: &Path) -> Result<Metadata, AnyError> {
    read_metadata(path).await
  }

  fn save_metadata(&self, path: &Path, meta: &Metadata) -> Result<(), AnyError> {
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

fn write_metadata(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
  let temporary = path.with_extension("json.tmp");
  std::fs::write(&temporary, bytes)?;
  std::fs::rename(temporary, path)
}

#[cfg(test)]
mod tests {
  use super::{fetch_manifest, manifest_requests, verify_manifest, BinariesManager, Metadata};
  use base64::{engine::general_purpose::STANDARD, Engine};
  use ed25519_dalek::{SignatureError, Signer, SigningKey};

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

  #[test]
  fn metadata_preserves_failed_check_time_and_reads_legacy_files() {
    let mut meta: Metadata =
      serde_json::from_slice(br#"{"versions":{"yt-dlp":"2026.10.26"},"is_locked":false}"#).unwrap();
    assert!(super::check_due(meta.ytdlp_last_check, 100));
    let root = std::env::temp_dir().join(format!("nezzel-check-time-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("metadata.json");
    // The attempt is committed before network work, which may subsequently fail.
    meta.ytdlp_last_check = Some(100);
    super::write_metadata(&path, &serde_json::to_vec(&meta).unwrap()).unwrap();
    let reloaded: Metadata = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert!(!super::check_due(reloaded.ytdlp_last_check, 101));
    assert_eq!(reloaded.versions["yt-dlp"], "2026.10.26");
    assert!(!path.with_extension("json.tmp").exists());
    std::fs::remove_dir_all(root).unwrap();
  }

  #[test]
  fn proxy_enabled_without_address_skips_all_binary_requests() {
    for proxy in [None, Some("")] {
      assert!(super::binary_client(Some(true), proxy).unwrap().is_none());
    }
    assert!(super::binary_client(Some(false), Some("http://["))
      .unwrap()
      .is_some());
    assert!(
      super::binary_client(Some(true), Some("http://127.0.0.1:8123"))
        .unwrap()
        .is_some()
    );
    assert!(
      super::binary_client(Some(true), Some("socks5h://127.0.0.1:1080"))
        .unwrap()
        .is_some()
    );
    assert!(super::binary_client(Some(true), Some("http://[")).is_err());
  }

  #[tokio::test]
  async fn busy_commit_restores_check_time_and_rechecks_concurrent_version() {
    use super::{CheckAttempt, InstallOutcome};
    use std::sync::Arc;
    let root = std::env::temp_dir().join(format!("nezzel-busy-{}", uuid::Uuid::new_v4()));
    let current = root.join(super::CURRENT_DIR);
    let staged = root.join("staged");
    std::fs::create_dir_all(&current).unwrap();
    std::fs::create_dir_all(&staged).unwrap();
    let name = if cfg!(windows) {
      "yt-dlp.exe"
    } else {
      "yt-dlp"
    };
    std::fs::write(current.join(name), b"old").unwrap();
    std::fs::write(staged.join(name), b"candidate").unwrap();
    let meta_path = root.join("metadata.json");
    let mut meta = Metadata::default();
    meta.versions.insert("yt-dlp".into(), "2026.10.26".into());
    meta.ytdlp_last_check = Some(86500);
    super::write_metadata(&meta_path, &serde_json::to_vec(&meta).unwrap()).unwrap();
    let operation = Arc::new(tokio::sync::Mutex::new(()));
    let processes = Arc::new(tokio::sync::RwLock::new(()));
    let running = processes.clone().read_owned().await;
    let attempt = Some(CheckAttempt {
      checked_at: 86500,
      previous: Some(100),
    });
    assert_eq!(
      super::commit_ytdlp(
        &root,
        &staged,
        "2026.11.01",
        attempt,
        operation.clone(),
        processes.clone()
      )
      .await
      .unwrap(),
      InstallOutcome::Busy
    );
    let restored: Metadata = serde_json::from_slice(&std::fs::read(&meta_path).unwrap()).unwrap();
    assert_eq!(restored.ytdlp_last_check, Some(100));
    assert_eq!(restored.versions["yt-dlp"], "2026.10.26");
    assert_eq!(std::fs::read(current.join(name)).unwrap(), b"old");
    meta.ytdlp_last_check = Some(99999);
    super::write_metadata(&meta_path, &serde_json::to_vec(&meta).unwrap()).unwrap();
    assert_eq!(
      super::commit_ytdlp(
        &root,
        &staged,
        "2026.11.01",
        attempt,
        operation.clone(),
        processes.clone()
      )
      .await
      .unwrap(),
      InstallOutcome::Busy
    );
    let superseded: Metadata = serde_json::from_slice(&std::fs::read(&meta_path).unwrap()).unwrap();
    assert_eq!(superseded.ytdlp_last_check, Some(99999));
    drop(running);
    // A different installer won while this candidate was downloading.
    meta.versions.insert("yt-dlp".into(), "2026.12.01".into());
    meta.ytdlp_last_check = Some(99999);
    std::fs::write(current.join(name), b"newer").unwrap();
    #[cfg(unix)]
    {
      use std::os::unix::fs::PermissionsExt;
      std::fs::set_permissions(current.join(name), std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    super::bundled_tools::record_repair(
      &current,
      "yt-dlp",
      &current.join(name),
      "2026.12.01",
      "verified-archive",
    )
    .unwrap();
    super::write_metadata(&meta_path, &serde_json::to_vec(&meta).unwrap()).unwrap();
    assert_eq!(
      super::commit_ytdlp(&root, &staged, "2026.11.01", attempt, operation, processes)
        .await
        .unwrap(),
      InstallOutcome::UpToDate
    );
    let preserved: Metadata = serde_json::from_slice(&std::fs::read(&meta_path).unwrap()).unwrap();
    assert_eq!(preserved.ytdlp_last_check, Some(99999));
    assert_eq!(std::fs::read(current.join(name)).unwrap(), b"newer");
    std::fs::remove_dir_all(root).unwrap();
  }

  #[tokio::test]
  async fn committed_bundle_merges_metadata_and_cleans_legacy_files() {
    use super::{InstallOutcome, CURRENT_DIR};
    use std::sync::Arc;
    let root = std::env::temp_dir().join(format!("nezzel-commit-{}", uuid::Uuid::new_v4()));
    let staged = root.join("staged");
    std::fs::create_dir_all(staged.join(".extract-empty")).unwrap();
    std::fs::create_dir_all(root.join("_internal")).unwrap();
    let name = if cfg!(windows) {
      "yt-dlp.exe"
    } else {
      "yt-dlp"
    };
    std::fs::write(root.join(name), b"old").unwrap();
    std::fs::write(staged.join(name), b"new").unwrap();
    let mut meta = Metadata::default();
    meta.versions.insert("yt-dlp".into(), "2026.10.26".into());
    meta.versions.insert("deno".into(), "2.6.0".into());
    meta.ytdlp_last_check = Some(100);
    let path = root.join("metadata.json");
    super::write_metadata(&path, &serde_json::to_vec(&meta).unwrap()).unwrap();
    let result = super::commit_ytdlp(
      &root,
      &staged,
      "2026.11.01",
      None,
      Arc::new(tokio::sync::Mutex::new(())),
      Arc::new(tokio::sync::RwLock::new(())),
    )
    .await
    .unwrap();
    assert_eq!(result, InstallOutcome::Installed(Some("2026.10.26".into())));
    let committed: Metadata = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(committed.versions["yt-dlp"], "2026.11.01");
    assert_eq!(committed.versions["deno"], "2.6.0");
    assert_eq!(committed.ytdlp_last_check, Some(100));
    assert_eq!(
      std::fs::read(root.join(CURRENT_DIR).join(name)).unwrap(),
      b"new"
    );
    assert!(!root.join(name).exists());
    assert!(!root.join("_internal").exists());
    assert!(!root.join(CURRENT_DIR).join(".extract-empty").exists());
    std::fs::remove_dir_all(root).unwrap();
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
