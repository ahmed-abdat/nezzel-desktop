//! Every helper tool Nezzel runs (yt-dlp exporters, gallery-dl, ffmpeg, OS utilities) is set up by
//! `configure_command`: its own process group on Unix, no console window on Windows.
//! `ToolProcess` adds the shared lifecycle for synchronous children: the tree is killed and the
//! child reaped when the owner drops it before the child exits.
use super::ytdlp_process::{
  configure_command, kill_platform_process, platform_process_from_child, PlatformProcess,
};
use std::ffi::OsStr;
use std::io;
use std::process::{ChildStderr, ChildStdout, Command, ExitStatus};
use tokio::sync::oneshot;

/// A command for a helper tool, configured like every other Nezzel child process.
pub(crate) fn command(program: impl AsRef<OsStr>) -> io::Result<Command> {
  let mut command = Command::new(program);
  configure_command(&mut command)?;
  Ok(command)
}

/// The tokio form of [`command`], for children awaited with `tokio::process`.
pub(crate) fn async_command(program: impl AsRef<OsStr>) -> io::Result<tokio::process::Command> {
  let mut command = tokio::process::Command::new(program);
  configure_command(command.as_std_mut())?;
  Ok(command)
}

/// A running child and its process tree. Dropping it before [`ToolProcess::wait`] returns an exit
/// status kills the whole tree (helpers it started included) and reaps the child.
pub(crate) struct ToolProcess {
  tree: Option<PlatformProcess>,
  exit: oneshot::Receiver<io::Result<ExitStatus>>,
  waiter: Option<std::thread::JoinHandle<()>>,
}

pub(crate) struct Spawned {
  pub process: ToolProcess,
  pub stdout: Option<ChildStdout>,
  pub stderr: Option<ChildStderr>,
}

impl ToolProcess {
  /// Configures and spawns `command`. `held` is kept alive until the child exits (for example the
  /// shared yt-dlp binaries lock), so it is released only once the tool is gone.
  pub(crate) fn spawn(command: &mut Command, held: impl Send + 'static) -> io::Result<Spawned> {
    configure_command(command)?;
    let mut child = command.spawn()?;
    let tree = match platform_process_from_child(&child) {
      Ok(tree) => tree,
      Err(error) => {
        let _ = child.kill();
        let _ = child.wait();
        return Err(io::Error::other(error));
      }
    };
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let (sender, exit) = oneshot::channel();
    // Marks the group leader reaped in the live-group registry once `wait` returns, so quit and
    // cancel keep killing only descendants that survive it.
    let reap_token = tree.reap_token();
    let waiter = std::thread::spawn(move || {
      let _held = held;
      let result = child.wait();
      drop(reap_token);
      let _ = sender.send(result);
    });
    Ok(Spawned {
      process: Self {
        tree: Some(tree),
        exit,
        waiter: Some(waiter),
      },
      stdout,
      stderr,
    })
  }

  /// Waits for the child to exit. Call it once; after an exit status, dropping no longer kills.
  pub(crate) async fn wait(&mut self) -> io::Result<ExitStatus> {
    let status = (&mut self.exit)
      .await
      .map_err(|_| io::Error::other("tool waiter stopped"))??;
    self.tree = None;
    Ok(status)
  }
}

impl Drop for ToolProcess {
  fn drop(&mut self) {
    let Some(tree) = self.tree.take() else {
      return;
    };
    kill_platform_process(&tree);
    #[cfg(unix)]
    if let Some(pgid) = tree.pgid {
      unsafe {
        libc::killpg(pgid, libc::SIGKILL);
      }
    }
    // The tree is killed, so the waiter's `wait` returns promptly and reaps the child.
    if let Some(waiter) = self.waiter.take() {
      let _ = waiter.join();
    }
  }
}
