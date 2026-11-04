//! Direct-file downloads on reqwest. Segmentation/resume design follows Zajil Download
//! Manager (motalim-ai contributors, GPL-3.0-or-later); see Zajil-LICENSE.
mod auth;
mod client;
pub mod engine;
pub mod hosts;
mod mark;
pub mod names;
mod plan;
pub mod probe;
pub(crate) mod runner;
mod sanitize;
mod state;
use std::fmt;
#[derive(Debug)]
pub enum Error {
  Cancelled,
  Address,
  Network,
  Encoding,
  State,
  Busy,
  Io,
  Restart,
  NotFile,
  Empty,
  Space,
  Proxy,
  NotPublic,
  RateLimited,
}
impl fmt::Display for Error {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(f, "{}", self.code())
  }
}
impl std::error::Error for Error {}
impl Error {
  pub fn code(&self) -> &'static str {
    match self {
      Self::Cancelled => "fileCancelled",
      Self::Address => "fileAddressRefused",
      Self::Network => "fileNetwork",
      Self::Encoding => "fileEncoding",
      Self::State => "fileState",
      Self::Busy => "fileBusy",
      Self::Io => "fileIo",
      Self::Restart => "fileChanged",
      Self::NotFile => "fileNotFile",
      Self::Empty => "fileEmpty",
      Self::Space => "fileSpace",
      Self::NotPublic => "fileNotPublic",
      Self::RateLimited => "fileRateLimited",
      Self::Proxy => "fileProxy",
    }
  }
}
impl From<std::io::Error> for Error {
  fn from(_: std::io::Error) -> Self {
    Self::Io
  }
}
pub type Result<T> = std::result::Result<T, Error>;
#[cfg(test)]
mod engine_tests;
#[cfg(test)]
mod tests;
