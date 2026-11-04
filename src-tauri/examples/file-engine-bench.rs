//! cargo run --release --features file-engine-bench --example file-engine-bench -- URL DIR [CONNECTIONS] [pdf|office]
//! Compare with: aria2c -x8 -s8 -k1M --dir DIR URL
use nezzel_lib::files::engine::Engine;
use sha2::{Digest, Sha256};
use std::{io::Read, path::PathBuf, time::Instant};
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
  tracing_subscriber::fmt()
    .with_max_level(tracing::Level::WARN)
    .without_time()
    .with_target(false)
    .init();
  let args: Vec<_> = std::env::args().skip(1).collect();
  if !(2..=4).contains(&args.len()) {
    return Err("usage: file-engine-bench URL DIRECTORY [CONNECTIONS] [pdf|office]".into());
  }
  let connections = args.get(2).map(|s| s.parse()).transpose()?.unwrap_or(8);
  let mut engine = Engine::new()?.with_connections(connections);
  if let Some(format) = args.get(3) {
    engine = engine.with_export(match format.as_str() {
      "pdf" => nezzel_lib::files::hosts::Export::Pdf,
      "office" => nezzel_lib::files::hosts::Export::Office,
      _ => return Err("export must be pdf or office".into()),
    });
  }
  if let Some(directory) = std::env::var_os("NEZZEL_BENCH_STATE_DIRECTORY") {
    engine = engine.with_state_directory(directory.into());
  }
  let (tx, rx) = tokio::sync::watch::channel(true);
  let signal = tokio::spawn(async move {
    let _ = tokio::signal::ctrl_c().await;
    let _ = tx.send(false);
  });
  let start = Instant::now();
  let result = engine
    .download(
      &args[0],
      &PathBuf::from(&args[1]),
      "benchmark",
      None,
      rx,
      &mut |_| {},
    )
    .await;
  signal.abort();
  let _ = signal.await;
  let path = result?;
  let bytes = std::fs::metadata(&path)?.len();
  let mut file = std::fs::File::open(&path)?;
  let mut hash = Sha256::new();
  let mut buffer = [0; 65536];
  loop {
    let count = file.read(&mut buffer)?;
    if count == 0 {
      break;
    }
    hash.update(&buffer[..count]);
  }
  println!(
    "name={} size={bytes} sha256={}",
    path.file_name().unwrap_or_default().to_string_lossy(),
    hex::encode(hash.finalize())
  );
  let elapsed = start.elapsed().as_secs_f64();
  println!(
    "{bytes} bytes in {elapsed:.3} s — {:.2} MB/s ({connections} requested connections)",
    bytes as f64 / 1_000_000.0 / elapsed
  );
  Ok(())
}
