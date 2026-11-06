use std::io;
use std::process::Command;
use std::time::Duration;

#[cfg(unix)]
use std::collections::HashMap;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
#[cfg(unix)]
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};

#[cfg(windows)]
use core::ffi::c_void;

#[cfg(windows)]
use std::os::windows::io::AsRawHandle;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};

#[cfg(windows)]
use windows_sys::Win32::System::JobObjects::{
  AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
  SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
  JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};

#[cfg(windows)]
use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

/// Engine process groups we started whose leader has not been reaped yet.
///
/// An entry lives until the thread that waits on the child reaps it (see
/// [`ReapToken`]), never until the handle is dropped: cancel paths drop the
/// handle while the wait thread is still reaping. While the leader is
/// unreaped its pgid cannot be reused, so signalling a registered entry is safe.
/// Entries carry a generation so a stale token can never match a newer group
/// that happens to reuse the same pgid.
#[cfg(unix)]
#[derive(Debug, Default)]
pub struct Registry {
  state: Mutex<RegistryState>,
}

#[cfg(unix)]
#[derive(Debug, Default)]
struct RegistryState {
  next_generation: u64,
  groups: HashMap<i32, Entry>,
}

/// `leader_reaped`: the group leader was waited on, but descendants (e.g. an
/// ffmpeg that ignores SIGTERM) may still be running. The entry then stays until
/// `killpg(pgid, 0)` says ESRCH; a pgid is never reused while a member lives.
#[cfg(unix)]
#[derive(Debug)]
struct Entry {
  generation: u64,
  leader_reaped: bool,
}

#[cfg(unix)]
impl RegistryState {
  /// True if this exact registration is still tracked; prunes reaped-leader
  /// entries whose group is empty.
  fn live(&mut self, pgid: i32, generation: u64) -> bool {
    match self.groups.get(&pgid) {
      Some(entry) if entry.generation == generation => {
        if entry.leader_reaped && group_gone(pgid) {
          self.groups.remove(&pgid);
          false
        } else {
          true
        }
      }
      _ => false,
    }
  }
}

#[cfg(unix)]
impl Registry {
  fn lock(&self) -> MutexGuard<'_, RegistryState> {
    // A panic elsewhere must not hide live groups from the quit path.
    self.state.lock().unwrap_or_else(|e| e.into_inner())
  }

  fn register(&self, pgid: i32) -> u64 {
    let mut state = self.lock();
    state.next_generation += 1;
    let generation = state.next_generation;
    state.groups.insert(
      pgid,
      Entry {
        generation,
        leader_reaped: false,
      },
    );
    generation
  }

  /// The leader was reaped: forget the group if it is empty, else keep it
  /// (marked) so cancel and quit can still finish off the survivors.
  fn leader_reaped(&self, pgid: i32, generation: u64) {
    let mut state = self.lock();
    if let Some(entry) = state.groups.get_mut(&pgid) {
      if entry.generation == generation {
        entry.leader_reaped = true;
      }
    }
    state.live(pgid, generation);
  }

  fn is_current(&self, pgid: i32, generation: u64) -> bool {
    self.lock().live(pgid, generation)
  }

  #[cfg(test)]
  fn groups_forget(&self, pgid: i32) {
    self.lock().groups.remove(&pgid);
  }

  fn snapshot(&self) -> Vec<(i32, u64)> {
    let mut state = self.lock();
    let all: Vec<(i32, u64)> = state
      .groups
      .iter()
      .map(|(p, e)| (*p, e.generation))
      .collect();
    all
      .into_iter()
      .filter(|(pgid, generation)| state.live(*pgid, *generation))
      .collect()
  }

  /// Signal the group only if this exact registration is still tracked. The
  /// check and the syscall happen under the lock. A tracked group either has
  /// its leader unreaped or still has members, so its pgid cannot be reused.
  fn signal(&self, pgid: i32, generation: u64, signal: i32) -> bool {
    let mut state = self.lock();
    if !state.live(pgid, generation) {
      return false;
    }
    unsafe { libc::killpg(pgid, signal) };
    true
  }
}

#[cfg(unix)]
static GLOBAL_REGISTRY: LazyLock<Arc<Registry>> = LazyLock::new(Arc::default);

#[cfg(unix)]
const CANCEL_KILL_DELAY: Duration = Duration::from_secs(3);

#[cfg(unix)]
fn group_gone(pgid: i32) -> bool {
  unsafe {
    libc::killpg(pgid, 0) == -1 && io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
  }
}

#[derive(Debug)]
pub struct PlatformProcess {
  #[cfg(unix)]
  pub pgid: Option<i32>,
  #[cfg(unix)]
  registration: Option<(Arc<Registry>, u64)>,
  #[cfg(windows)]
  pub job: Option<JobHandle>,
}

impl PlatformProcess {
  /// A handle that refers to no process (tests, placeholders).
  #[cfg(test)]
  pub fn detached() -> Self {
    #[cfg(unix)]
    {
      Self {
        pgid: None,
        registration: None,
      }
    }
    #[cfg(windows)]
    {
      Self { job: None }
    }
    #[cfg(not(any(unix, windows)))]
    {
      Self {}
    }
  }

  /// Token to drop once the child has been reaped (`wait` returned).
  pub fn reap_token(&self) -> ReapToken {
    #[cfg(unix)]
    {
      ReapToken {
        entry: self
          .pgid
          .zip(self.registration.clone())
          .map(|(pgid, (registry, generation))| (registry, pgid, generation)),
      }
    }
    #[cfg(not(unix))]
    {
      ReapToken {}
    }
  }
}

/// Marks the group leader as reaped when dropped (the group stays tracked while
/// descendants live). Hold it in the code that calls `wait()` on the child and
/// drop it right after `wait` returns.
#[derive(Debug)]
pub struct ReapToken {
  #[cfg(unix)]
  entry: Option<(Arc<Registry>, i32, u64)>,
}

#[cfg(unix)]
impl Drop for ReapToken {
  fn drop(&mut self) {
    if let Some((registry, pgid, generation)) = self.entry.take() {
      registry.leader_reaped(pgid, generation);
    }
  }
}

#[cfg(windows)]
#[derive(Debug)]
pub struct JobHandle {
  handle: isize,
}

#[cfg(windows)]
impl JobHandle {
  #[inline]
  fn as_handle(&self) -> HANDLE {
    self.handle as HANDLE
  }
}

#[cfg(windows)]
impl Drop for JobHandle {
  fn drop(&mut self) {
    if self.handle != 0 {
      unsafe { CloseHandle(self.as_handle()) };
    }
  }
}

pub fn configure_command(command: &mut Command) -> io::Result<()> {
  #[cfg(unix)]
  {
    unsafe {
      command.pre_exec(|| {
        let result = libc::setpgid(0, 0);
        if result == 0 {
          Ok(())
        } else {
          Err(io::Error::last_os_error())
        }
      });
    }
  }

  #[cfg(windows)]
  {
    command.creation_flags(CREATE_NO_WINDOW);
  }

  Ok(())
}

pub fn platform_process_from_child(child: &std::process::Child) -> Result<PlatformProcess, String> {
  #[cfg(unix)]
  {
    Ok(register_child(child, &GLOBAL_REGISTRY))
  }

  #[cfg(windows)]
  {
    let job = create_job_for_child(child)
      .map(Some)
      .map_err(|e| format!("yt-dlp job object error: {e}"))?;
    Ok(PlatformProcess { job })
  }

  #[cfg(not(any(unix, windows)))]
  {
    let _ = child;
    Ok(PlatformProcess {})
  }
}

#[cfg(unix)]
fn register_child(child: &std::process::Child, registry: &Arc<Registry>) -> PlatformProcess {
  let pgid = child.id() as i32;
  let generation = registry.register(pgid);
  PlatformProcess {
    pgid: Some(pgid),
    registration: Some((registry.clone(), generation)),
  }
}

pub fn kill_platform_process(platform: &PlatformProcess) {
  #[cfg(unix)]
  kill_platform_process_after(platform, CANCEL_KILL_DELAY);

  #[cfg(windows)]
  {
    if let Some(job) = &platform.job {
      unsafe { TerminateJobObject(job.as_handle(), 1) };
    }
  }

  #[cfg(not(any(unix, windows)))]
  let _ = platform;
}

/// SIGTERM now; SIGKILL after `delay` if that registration is still unreaped.
#[cfg(unix)]
pub(crate) fn kill_platform_process_after(platform: &PlatformProcess, delay: Duration) {
  let (Some(pgid), Some((registry, generation))) = (platform.pgid, platform.registration.clone())
  else {
    return;
  };
  if !registry.signal(pgid, generation, libc::SIGTERM) {
    return;
  }
  std::thread::spawn(move || {
    std::thread::sleep(delay);
    registry.signal(pgid, generation, libc::SIGKILL);
  });
}

/// Quit path: stop every engine process group still alive. SIGTERM first, then
/// SIGKILL for whatever is still unreaped after `grace`. Windows job objects
/// already kill on close, so this is a no-op there. An empty registry returns
/// at once.
pub fn terminate_all_live(grace: Duration) {
  #[cfg(unix)]
  terminate_all_in(&GLOBAL_REGISTRY, grace);

  #[cfg(not(unix))]
  let _ = grace;
}

#[cfg(unix)]
fn terminate_all_in(registry: &Registry, grace: Duration) {
  let mut groups = registry.snapshot();
  if groups.is_empty() {
    return;
  }
  for (pgid, generation) in &groups {
    registry.signal(*pgid, *generation, libc::SIGTERM);
  }
  let poll = Duration::from_millis(50);
  let mut waited = Duration::ZERO;
  loop {
    groups
      .retain(|(pgid, generation)| registry.is_current(*pgid, *generation) && !group_gone(*pgid));
    if groups.is_empty() || waited >= grace {
      break;
    }
    let step = poll.min(grace - waited);
    std::thread::sleep(step);
    waited += step;
  }
  for (pgid, generation) in groups {
    registry.signal(pgid, generation, libc::SIGKILL);
  }
}

#[cfg(windows)]
fn create_job_for_child(child: &std::process::Child) -> io::Result<JobHandle> {
  let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null::<u16>()) };
  if handle.is_null() {
    return Err(io::Error::last_os_error());
  }

  let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
  info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;

  let info_ptr = (&info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION) as *const c_void;
  let info_size = std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32;

  let set_ok = unsafe {
    SetInformationJobObject(
      handle,
      JobObjectExtendedLimitInformation,
      info_ptr,
      info_size,
    )
  };
  if set_ok == 0 {
    unsafe { CloseHandle(handle) };
    return Err(io::Error::last_os_error());
  }

  let process_handle = child.as_raw_handle() as HANDLE;
  let assign_ok = unsafe { AssignProcessToJobObject(handle, process_handle) };
  if assign_ok == 0 {
    unsafe { CloseHandle(handle) };
    return Err(io::Error::last_os_error());
  }

  Ok(JobHandle {
    handle: handle as isize,
  })
}

#[cfg(all(test, unix))]
mod tests {
  use super::*;
  use std::io::Read;
  use std::process::{Child, Stdio};
  use std::sync::mpsc;

  /// A shell that ignores SIGTERM, with a backgrounded grandchild that does too.
  fn spawn_stubborn() -> Child {
    let mut command = Command::new("/bin/sh");
    command
      .args(["-c", "trap '' TERM; sleep 30 & printf ready; wait"])
      .stdout(Stdio::piped());
    configure_command(&mut command).unwrap();
    let mut child = command.spawn().unwrap();
    let mut ready = [0; 5];
    child.stdout.take().unwrap().read_exact(&mut ready).unwrap();
    child
  }

  /// Mirrors the runners: a thread owns the child, reaps it, then drops the token.
  fn reap_in_thread(mut child: Child, token: ReapToken) -> mpsc::Receiver<()> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
      child.wait().unwrap();
      drop(token);
      let _ = tx.send(());
    });
    rx
  }

  const LIMIT: Duration = Duration::from_secs(4);

  /// Leader exits at once; its same-group child ignores SIGTERM and keeps running.
  fn spawn_leader_with_stubborn_descendant() -> Child {
    let mut command = Command::new("/bin/sh");
    command
      .args(["-c", "(trap '' TERM; exec sleep 30) & printf ready"])
      .stdout(Stdio::piped());
    configure_command(&mut command).unwrap();
    let mut child = command.spawn().unwrap();
    let mut ready = [0; 5];
    child.stdout.take().unwrap().read_exact(&mut ready).unwrap();
    child
  }

  fn wait_group_gone(pgid: i32) -> bool {
    let deadline = std::time::Instant::now() + LIMIT;
    while std::time::Instant::now() < deadline {
      if group_gone(pgid) {
        return true;
      }
      std::thread::sleep(Duration::from_millis(10));
    }
    false
  }

  #[test]
  fn quit_pass_kills_a_descendant_that_outlives_the_reaped_leader() {
    let registry = Arc::new(Registry::default());
    let child = spawn_leader_with_stubborn_descendant();
    let platform = register_child(&child, &registry);
    let pgid = platform.pgid.unwrap();
    reap_in_thread(child, platform.reap_token())
      .recv_timeout(LIMIT)
      .unwrap();
    assert!(!group_gone(pgid), "descendant still runs");
    assert_eq!(registry.snapshot().len(), 1, "group stays tracked");

    terminate_all_in(&registry, Duration::from_millis(300));

    assert!(wait_group_gone(pgid));
    assert!(registry.snapshot().is_empty(), "pruned once empty");
  }

  #[test]
  fn cancel_kills_a_descendant_that_outlives_the_reaped_leader() {
    let registry = Arc::new(Registry::default());
    let child = spawn_leader_with_stubborn_descendant();
    let platform = register_child(&child, &registry);
    let pgid = platform.pgid.unwrap();
    let reaped = reap_in_thread(child, platform.reap_token());

    kill_platform_process_after(&platform, Duration::from_millis(200));
    drop(platform);
    reaped.recv_timeout(LIMIT).unwrap();

    assert!(wait_group_gone(pgid));
  }

  #[test]
  fn terminate_all_kills_groups_that_ignore_sigterm() {
    let registry = Arc::new(Registry::default());
    let child = spawn_stubborn();
    let platform = register_child(&child, &registry);
    let pgid = platform.pgid.unwrap();
    let reaped = reap_in_thread(child, platform.reap_token());

    terminate_all_in(&registry, Duration::from_millis(300));

    reaped.recv_timeout(LIMIT).expect("leader exits");
    assert!(wait_group_gone(pgid), "grandchild must be dead too");
    assert!(registry.snapshot().is_empty());
  }

  #[test]
  fn reaping_leaves_the_registry_but_dropping_the_handle_does_not() {
    let registry = Arc::new(Registry::default());
    let mut child = spawn_stubborn();
    let platform = register_child(&child, &registry);
    let pgid = platform.pgid.unwrap();
    let token = platform.reap_token();

    drop(platform);
    assert_eq!(registry.snapshot().len(), 1, "handle drop keeps the entry");

    unsafe { libc::killpg(pgid, libc::SIGKILL) };
    child.wait().unwrap();
    drop(token);
    // The killed `sleep` stays a zombie group member until launchd reaps it (slow under load).
    assert!(wait_group_gone(pgid));
    assert!(registry.snapshot().is_empty());
  }

  #[test]
  fn cancel_escalates_to_sigkill_after_the_handle_is_dropped() {
    let registry = Arc::new(Registry::default());
    let child = spawn_stubborn();
    let platform = register_child(&child, &registry);
    let pgid = platform.pgid.unwrap();
    let reaped = reap_in_thread(child, platform.reap_token());

    kill_platform_process_after(&platform, Duration::from_millis(200));
    drop(platform); // downloads return right after cancelling

    reaped.recv_timeout(LIMIT).expect("leader exits");
    assert!(wait_group_gone(pgid));
  }

  #[test]
  fn stale_registration_never_signals_a_reused_pgid() {
    let registry = Registry::default();
    let mut child = spawn_stubborn();
    let pgid = child.id() as i32;
    let old = registry.register(pgid);
    registry.leader_reaped(pgid, old);
    registry.groups_forget(pgid);
    let new = registry.register(pgid);

    assert!(!registry.signal(pgid, old, libc::SIGKILL));
    assert!(child.try_wait().unwrap().is_none(), "group untouched");

    assert!(registry.signal(pgid, new, libc::SIGKILL));
    child.wait().unwrap();
  }

  #[test]
  fn poisoned_registry_still_reports_live_groups() {
    let registry = Arc::new(Registry::default());
    registry.register(4242);
    let poisoner = registry.clone();
    let _ = std::thread::spawn(move || {
      let _guard = poisoner.state.lock().unwrap();
      panic!("poison");
    })
    .join();
    assert_eq!(registry.snapshot().len(), 1);
  }
}
