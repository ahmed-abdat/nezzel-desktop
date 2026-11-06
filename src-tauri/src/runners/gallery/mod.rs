//! gallery-dl is a separate executable. No gallery Python code is linked into the app.
//! Each child module changes for one reason: `runner` (process setup), `probe` (reading a post),
//! `images` (image transfer), `info` (info and preview stages), `routing` (engine choice),
//! `download` (the download flow).
mod download;
mod images;
mod info;
mod probe;
mod routing;
mod runner;

pub use download::run_media_download;
pub use info::preview_media;
#[cfg(feature = "link-matrix")]
pub use info::run_media_info;
pub(crate) use info::{error_diagnostic, photo_info, run_gallery_info};
#[cfg(test)]
pub(in crate::runners) use runner::arguments;
pub(crate) use runner::available;
