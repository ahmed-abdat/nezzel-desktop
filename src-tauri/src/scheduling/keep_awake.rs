//! One lease for all running download jobs. Native assertions stay on their owning thread.
use std::sync::{mpsc, Arc, LazyLock, Mutex, Weak};
use tauri::Manager;

pub(crate) struct Lease {
  stop: mpsc::Sender<()>,
}
impl Drop for Lease {
  fn drop(&mut self) {
    let _ = self.stop.send(());
  }
}
static LEASE: LazyLock<Mutex<Weak<Lease>>> = LazyLock::new(|| Mutex::new(Weak::new()));

pub(crate) fn acquire(app: &tauri::AppHandle) -> Option<Arc<Lease>> {
  let cfg = app.state::<crate::SharedConfig>().inner().clone();
  let mut active = LEASE.lock().unwrap_or_else(|e| e.into_inner());
  if let Some(lease) = active.upgrade() {
    return Some(lease);
  }
  let (tx, rx) = mpsc::channel();
  std::thread::Builder::new()
    .name("download-awake".into())
    .spawn(move || {
      let mut assertion = None;
      loop {
        if cfg.load().performance.keep_awake {
          if assertion.is_none() {
            assertion = platform::Assertion::new();
          }
        } else {
          assertion = None;
        }
        match rx.recv_timeout(std::time::Duration::from_secs(1)) {
          Err(mpsc::RecvTimeoutError::Timeout) => {}
          _ => break,
        }
      }
      drop(assertion);
    })
    .ok()?;
  let lease = Arc::new(Lease { stop: tx });
  *active = Arc::downgrade(&lease);
  Some(lease)
}

#[cfg(target_os = "macos")]
mod platform {
  use std::ffi::{c_char, c_void};
  type CFStringRef = *const c_void;
  #[link(name = "CoreFoundation", kind = "framework")]
  unsafe extern "C" {
    fn CFStringCreateWithCString(
      allocator: *const c_void,
      text: *const c_char,
      encoding: u32,
    ) -> CFStringRef;
    fn CFRelease(value: *const c_void);
  }
  #[link(name = "IOKit", kind = "framework")]
  unsafe extern "C" {
    fn IOPMAssertionCreateWithName(
      kind: CFStringRef,
      level: u32,
      name: CFStringRef,
      id: *mut u32,
    ) -> i32;
    fn IOPMAssertionRelease(id: u32) -> i32;
  }
  pub struct Assertion(u32);
  impl Assertion {
    pub fn new() -> Option<Self> {
      // SAFETY: static terminated strings; CF objects released after IOKit has copied them.
      unsafe {
        let kind = CFStringCreateWithCString(
          std::ptr::null(),
          c"PreventUserIdleSystemSleep".as_ptr(),
          0x08000100,
        );
        let name =
          CFStringCreateWithCString(std::ptr::null(), c"Nezzel downloads".as_ptr(), 0x08000100);
        if kind.is_null() || name.is_null() {
          if !kind.is_null() {
            CFRelease(kind);
          }
          if !name.is_null() {
            CFRelease(name);
          }
          return None;
        }
        let mut id = 0;
        let result = IOPMAssertionCreateWithName(kind, 255, name, &mut id);
        CFRelease(kind);
        CFRelease(name);
        (result == 0).then_some(Self(id))
      }
    }
  }
  impl Drop for Assertion {
    fn drop(&mut self) {
      unsafe {
        IOPMAssertionRelease(self.0);
      }
    }
  }
}
#[cfg(windows)]
mod platform {
  #[link(name = "kernel32")]
  unsafe extern "system" {
    fn SetThreadExecutionState(flags: u32) -> u32;
  }
  pub struct Assertion;
  impl Assertion {
    pub fn new() -> Option<Self> {
      // Acquire and release both run on the dedicated lease thread, never a Tokio worker.
      (unsafe { SetThreadExecutionState(0x80000001) } != 0).then_some(Self)
    }
  }
  impl Drop for Assertion {
    fn drop(&mut self) {
      unsafe {
        SetThreadExecutionState(0x80000000);
      }
    }
  }
}
#[cfg(target_os = "linux")]
mod platform {
  use std::process::{Child, Command, Stdio};
  pub struct Assertion(Child);
  impl Assertion {
    pub fn new() -> Option<Self> {
      // systemd-inhibit obtains logind's inhibitor fd; cat exits when our stdin pipe closes.
      Command::new("systemd-inhibit")
        .args([
          "--what=sleep:idle",
          "--mode=block",
          "--who=Nezzel",
          "--why=Downloads running",
          "cat",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()
        .map(Self)
    }
  }
  impl Drop for Assertion {
    fn drop(&mut self) {
      self.0.stdin.take();
      let _ = self.0.kill();
      let _ = self.0.wait();
    }
  }
}
