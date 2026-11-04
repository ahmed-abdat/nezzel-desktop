pub mod binaries_extractor;
pub mod binaries_manager;
pub mod binaries_state;
pub(crate) mod bundled_tools;
pub(crate) mod ytdlp_update;

// Every app-owned yt-dlp process holds a read guard until it has been reaped.
pub(crate) static YTDLP_PROCESSES: std::sync::LazyLock<std::sync::Arc<tokio::sync::RwLock<()>>> =
  std::sync::LazyLock::new(|| std::sync::Arc::new(tokio::sync::RwLock::new(())));
