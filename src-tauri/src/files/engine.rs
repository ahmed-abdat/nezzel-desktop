//! Bounded segmented transfers, durable resume and conditional range validation.
//! The algorithm follows Zajil's design (GPL-3.0-or-later); see Zajil-LICENSE.
use super::{
  auth::StateKey,
  client::{wait, Client, Response},
  mark,
  plan::{self, Segment, Validator},
  probe::{Kind, Probe},
  state::{self, State},
  Error, Result,
};
use std::{
  collections::{HashMap, HashSet},
  fs::File,
  path::{Path, PathBuf},
  sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, LazyLock, Mutex, Weak,
  },
  time::{Duration, Instant, SystemTime},
};
use tokio::{
  sync::{mpsc, watch, Semaphore},
  task::JoinSet,
};
static FILES: LazyLock<Arc<Semaphore>> = LazyLock::new(|| Arc::new(Semaphore::new(3)));
static SESSIONS: LazyLock<Mutex<HashMap<String, Weak<tokio::sync::Mutex<()>>>>> =
  LazyLock::new(|| Mutex::new(HashMap::new()));
fn session(identity: &str) -> Result<Arc<tokio::sync::Mutex<()>>> {
  let mut sessions = SESSIONS.lock().map_err(|_| Error::State)?;
  sessions.retain(|_, s| s.strong_count() > 0);
  Ok(
    sessions
      .get(identity)
      .and_then(Weak::upgrade)
      .unwrap_or_else(|| {
        let gate = Arc::new(tokio::sync::Mutex::new(()));
        sessions.insert(identity.to_owned(), Arc::downgrade(&gate));
        gate
      }),
  )
}
const BATCH: usize = 256 * 1024;
#[derive(Clone)]
pub struct Engine {
  client: Client,
  connections: usize,
  state_key: Option<Arc<StateKey>>,
  state_root: Option<PathBuf>,
  file_row: bool,
  export: super::hosts::Export,
}
#[derive(Debug)]
pub enum Event {
  Progress {
    bytes: u64,
    total: Option<u64>,
    speed: f64,
  },
  Checkpoint {
    bytes: u64,
  },
}
struct Board {
  state: State,
  active: HashSet<usize>,
  writing: HashSet<usize>,
  speeds: Vec<(Instant, u64)>,
}
impl Board {
  fn take(&mut self, connections: usize) -> Option<usize> {
    if let Some(i) = self
      .state
      .segments
      .iter()
      .enumerate()
      .find_map(|(i, s)| (!self.active.contains(&i) && s.remaining() != Some(0)).then_some(i))
    {
      self.active.insert(i);
      return Some(i);
    }
    if connections <= 1 || self.state.segments.len() >= 1024 {
      return None;
    }
    let now = Instant::now();
    let candidate = self
      .active
      .iter()
      .copied()
      .filter_map(|i| {
        if self.writing.contains(&i) {
          return None;
        }
        let s = &self.state.segments[i];
        let left = s.remaining()?;
        if left < 2 * plan::MIN_SEGMENT {
          return None;
        }
        let (start, initial) = self.speeds[i];
        let elapsed = now.duration_since(start).as_secs_f64();
        let speed = (s.done - initial) as f64 / elapsed.max(0.001);
        Some((
          i,
          if elapsed >= 0.5 && speed > 0.0 {
            left as f64 / speed
          } else {
            left as f64
          },
        ))
      })
      .max_by(|a, b| a.1.total_cmp(&b.1))
      .map(|p| p.0)?;
    let s = &mut self.state.segments[candidate];
    let end = s.end?;
    let split = s.start + s.done + (end - s.start - s.done) / 2;
    s.end = Some(split);
    let i = self.state.segments.len();
    self.state.segments.push(Segment {
      start: split,
      end: Some(end),
      done: 0,
    });
    self.speeds.push((now, 0));
    self.active.insert(i);
    Some(i)
  }
}
struct Assignment {
  worker: usize,
  index: usize,
  body: Option<Response>,
}
struct Shared {
  file: Arc<File>,
  state_key: Arc<StateKey>,
  board: Mutex<Board>,
  pause: tokio::sync::Mutex<Instant>,
  limit: AtomicUsize,
  io: Arc<tokio::sync::RwLock<()>>,
  _lease: Arc<tokio::sync::OwnedMutexGuard<()>>,
}
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
  tokio::task::spawn_blocking(f)
    .await
    .map_err(|_| Error::Io)?
}
impl Engine {
  pub fn new() -> Result<Self> {
    Ok(Self {
      client: Client::new()?,
      connections: plan::DEFAULT_CONNECTIONS,
      state_key: None,
      state_root: None,
      file_row: false,
      export: super::hosts::Export::Pdf,
    })
  }
  /// Metadata admitted this row as a file, possibly only after yt-dlp rejected its URL.
  /// Preserve that decision when a cached probe expires; HTML and media remain refused.
  pub(crate) fn for_file_row(mut self) -> Self {
    self.file_row = true;
    self
  }
  pub fn with_state_directory(mut self, directory: PathBuf) -> Self {
    self.state_root = Some(directory);
    self
  }
  /// Native Google files default to PDF; Office selects DOCX, XLSX or PPTX by kind.
  pub fn with_export(mut self, export: super::hosts::Export) -> Self {
    self.export = export;
    self
  }
  /// Resume, probe caches and runner cleanup must use the same export identity.
  pub(crate) fn request_identity(&self, url: &str) -> String {
    if matches!(self.export, super::hosts::Export::Office)
      && super::hosts::plan(url, self.export).is_some_and(|p| p.native_export())
    {
      state::identity(&format!("{url}\nexport=office"))
    } else {
      state::identity(url)
    }
  }
  /// Connection count is bounded internally; all transfers also share an eight-connection host cap.
  pub fn with_connections(mut self, connections: usize) -> Self {
    self.connections = connections.clamp(1, plan::MAX_CONNECTIONS);
    self
  }
  #[cfg(test)]
  pub(crate) fn local() -> Self {
    Self {
      client: Client::local(),
      connections: 8,
      state_key: Some(Arc::new(StateKey::ephemeral())),
      state_root: None,
      file_row: false,
      export: super::hosts::Export::Pdf,
    }
  }
  #[cfg(test)]
  pub(crate) fn scripted(script: super::client::Script) -> Self {
    let mut client = Client::local();
    client.script = Some(script);
    Self {
      client,
      connections: 8,
      state_key: Some(Arc::new(StateKey::ephemeral())),
      state_root: None,
      file_row: false,
      export: super::hosts::Export::Pdf,
    }
  }
  async fn authentication_key(&self) -> Result<Arc<StateKey>> {
    match &self.state_key {
      Some(key) => Ok(key.clone()),
      None => {
        let root = self
          .state_root
          .clone()
          .map(Ok)
          .unwrap_or_else(super::auth::default_root)?;
        Ok(Arc::new(blocking(move || StateKey::load(&root)).await?))
      }
    }
  }
  pub(crate) async fn discard(&self, dir: PathBuf, identity: String, owner: String) -> Result<()> {
    let _lease = session(&identity)?.lock_owned().await;
    let key = self.authentication_key().await?;
    blocking(move || state::discard(&dir, &identity, &owner, &key)).await
  }
  async fn request(
    &self,
    url: &str,
    range: Option<(u64, Option<u64>, &str)>,
    mut cancel: watch::Receiver<bool>,
  ) -> Result<Response> {
    for attempt in 0..plan::ATTEMPTS {
      let delay = match self.client.get(url, range, cancel.clone()).await {
        Ok(body) => {
          let status = body.response.status().as_u16();
          // Reject raw GitHub HTML before any probe, resumed or ranged body can be saved.
          if body.response.status().is_success()
            && body
              .response
              .headers()
              .get("content-type")
              .and_then(|v| v.to_str().ok())
              .is_some_and(|v| {
                matches!(
                  crate::media_types::base_mime(v).as_str(),
                  "text/html" | "application/xhtml+xml"
                )
              })
            && super::hosts::plan(url, self.export).is_some_and(|plan| plan.github_raw())
          {
            return Err(Error::NotFile);
          }
          if matches!(status, 429 | 509) && super::hosts::relax_rate_limit(url) {
            return Ok(body);
          }
          if !plan::TRANSIENT_STATUS.contains(&status) {
            return Ok(body);
          }
          tracing::warn!(status, attempt = attempt + 1, "Transient file HTTP status");
          plan::retry_after(
            body
              .response
              .headers()
              .get("retry-after")
              .and_then(|h| h.to_str().ok()),
            SystemTime::now(),
          )?
        }
        Err(Error::Network) => None,
        Err(error) => return Err(error),
      };
      if attempt + 1 == plan::ATTEMPTS {
        return Err(Error::Network);
      }
      tracing::warn!(
        attempt = attempt + 1,
        "Retrying file request after transient failure"
      );
      wait(&mut cancel, async {
        tokio::time::sleep(delay.unwrap_or_else(|| plan::backoff(attempt))).await;
        Ok(())
      })
      .await?;
    }
    Err(Error::Network)
  }
  pub async fn probe(&self, url: &str, cancel: watch::Receiver<bool>) -> Result<Probe> {
    if let Some(plan) = super::hosts::plan(url, self.export) {
      return tokio::time::timeout(Duration::from_secs(30), self.probe_share(plan, cancel))
        .await
        .map_err(|_| Error::Network)?;
    }
    let mut probe = Probe::from_response(self.request(url, None, cancel).await?)?;
    if self.file_row && probe.kind == Kind::Unknown {
      probe.confirm_unknown()?;
    }
    Ok(probe)
  }
  async fn probe_share(
    &self,
    mut plan: super::hosts::Plan,
    mut cancel: watch::Receiver<bool>,
  ) -> Result<Probe> {
    if plan.folder() {
      return Err(Error::NotFile);
    }
    let mut url = plan.url.clone();
    for _ in 0..5 {
      let mut body = self.request(url.as_str(), None, cancel.clone()).await?;
      let status = body.response.status().as_u16();
      let api = plan.api(&url);
      if super::hosts::sign_in_page(body.response.url()) {
        return Err(Error::NotPublic);
      }
      let json_error = status != 200
        && body
          .response
          .headers()
          .get("content-type")
          .and_then(|v| v.to_str().ok())
          .is_some_and(|v| {
            v.split(';')
              .next()
              .is_some_and(|v| v.trim().eq_ignore_ascii_case("application/json"))
          });
      if !api && status != 200 && !json_error {
        return Err(plan.response_error(status, ""));
      }
      let html = body
        .response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| {
          matches!(
            v.split(';').next().unwrap_or_default().trim(),
            "text/html" | "application/xhtml+xml"
          )
        });
      if !api && status == 200 && (!html || plan.direct(body.response.url())) {
        let mut probe = Probe::from_response(body)?;
        if let Some(name) = &plan.name {
          probe.name.clone_from(name);
        }
        probe.size = probe.size.or(plan.size);
        probe.confirm_unknown()?;
        // Keep the confirmed endpoint (and its UUID) for conditional requests. A fresh
        // resolve on resume renews it; the initial body is consumed without another GET.
        probe.source = url.to_string();
        return Ok(probe);
      }
      let page = body.response.url().clone();
      let mut bytes = Vec::new();
      while let Some(chunk) = wait(&mut cancel, async {
        body
          .response
          .chunk()
          .await
          .map_err(super::client::network_error)
      })
      .await?
      {
        if bytes.len() + chunk.len() > 2 * 1024 * 1024 {
          return Err(Error::NotFile);
        }
        bytes.extend_from_slice(&chunk);
      }
      let text = std::str::from_utf8(&bytes).map_err(|_| Error::NotFile)?;
      if status != 200 {
        return Err(plan.response_error(status, text));
      }
      url = if api {
        plan.next_api(&url, text)?
      } else {
        plan.next(&page, text)?
      };
    }
    Err(Error::NotFile)
  }
  pub async fn download(
    &self,
    url: &str,
    dir: &Path,
    owner: &str,
    probe: Option<Probe>,
    mut cancel: watch::Receiver<bool>,
    event: &mut (dyn FnMut(Event) + Send),
  ) -> Result<PathBuf> {
    let _slot = wait(&mut cancel, async {
      FILES.clone().acquire_owned().await.map_err(|_| Error::Busy)
    })
    .await?;
    let state_key = self.authentication_key().await?;
    let plan = super::hosts::plan(url, self.export);
    let identity = self.request_identity(url);
    let shared_file = plan.is_some();
    let probe = if shared_file && probe.is_none() {
      Some(self.probe(url, cancel.clone()).await?)
    } else {
      probe
    };
    let mut transfer_url = if shared_file {
      probe
        .as_ref()
        .map(|p| p.source.clone())
        .unwrap_or_else(|| url.to_owned())
    } else {
      url.to_owned()
    };
    let gate = session(&identity)?;
    // Blocking disk writes cannot be aborted. Their lease delays a subsequent resume until
    // every surviving write from an aborted task has released the part's OS file lock.
    let lease = Arc::new(wait(&mut cancel, async { Ok(gate.clone().lock_owned().await) }).await?);
    let (directory, id) = (dir.to_path_buf(), identity.clone());
    let key = state_key.clone();
    let resumed = blocking(move || state::resume(&directory, &id, &key)).await?;
    let (mut target, mut initial) = if let Some(target) = resumed {
      (target, probe)
    } else {
      let p = match probe {
        Some(p) => p,
        None => self.probe(url, cancel.clone()).await?,
      };
      if p.kind != Kind::File {
        return Err(Error::NotFile);
      }
      let (directory, id, name) = (dir.to_path_buf(), identity.clone(), p.name.clone());
      let key = state_key.clone();
      (
        blocking(move || state::target(&directory, &id, &name, &key)).await?,
        Some(p),
      )
    };
    let mut resume_body = Vec::new();
    if let Some(saved) = target.state.as_ref() {
      if saved.ranges
        && saved.size.is_some()
        && saved.validator.if_range().is_some()
        && saved.bytes() > 0
      {
        let next = saved
          .segments
          .iter()
          .enumerate()
          .find(|(_, s)| s.remaining() != Some(0));
        let (idx, start, end) = next
          .map(|(i, s)| (i, s.start + s.done, s.end.map(|n| n - 1)))
          .unwrap_or((0, 0, Some(0)));
        let validator = saved.validator.if_range().ok_or(Error::State)?;
        let body = self
          .request(&transfer_url, Some((start, end, validator)), cancel.clone())
          .await?;
        if valid_range(&body, saved, start, end).is_ok() {
          initial = None;
          if next.is_some() {
            resume_body.push((idx, body));
          }
        } else if body.response.status() == reqwest::StatusCode::OK {
          let mut p = Probe::from_response(body)?;
          if shared_file {
            p.confirm_unknown()?;
            p.source.clone_from(&transfer_url);
          }
          initial = Some(p);
        } else {
          drop(body);
          if initial.is_none() {
            initial = Some(self.probe(url, cancel.clone()).await?);
          }
        }
      } else if initial.is_none() {
        initial = Some(self.probe(url, cancel.clone()).await?);
      }
    }
    // A range 200/mismatched 206 invalidates every segment. Stop all old workers before reset.
    for attempt in 0..=1 {
      if let Some(mut p) = initial.take() {
        if shared_file {
          transfer_url.clone_from(&p.source);
        }
        if p.kind != Kind::File {
          return Err(Error::NotFile);
        }
        if p.size == Some(0) {
          return Err(Error::Empty);
        }
        p.body.acquire(cancel.clone()).await?;
        let parallel =
          attempt == 0 && p.ranges && p.size.is_some() && p.validator.if_range().is_some();
        let segments = plan::segments(p.size, self.connections, parallel);
        let name = target
          .final_path
          .file_name()
          .and_then(|n| n.to_str())
          .ok_or(Error::State)?
          .to_owned();
        let mut state = State::new(
          identity.clone(),
          name,
          p.size,
          p.ranges,
          p.validator,
          segments,
        );
        state.mime = Some(p.mime);
        resume_body = vec![(0, p.body)];
        // Verify one conditional range before segment zero starts consuming the original GET.
        // A signed or connection-limited server may reject Range: keep its one-use full stream.
        if state.segments.len() > 1 {
          let segment = &state.segments[1];
          let start = segment.start;
          let end = segment.end.map(|n| n - 1);
          let validator = state.validator.if_range().ok_or(Error::State)?;
          match self
            .client
            .get(&transfer_url, Some((start, end, validator)), cancel.clone())
            .await
          {
            Ok(body) if valid_range(&body, &state, start, end).is_ok() => {
              resume_body.push((1, body))
            }
            Ok(body) if body.response.status() == reqwest::StatusCode::OK => {
              let mut p = Probe::from_response(body)?;
              if shared_file {
                p.confirm_unknown()?;
              }
              if p.kind != Kind::File {
                return Err(Error::NotFile);
              }
              state = State::new(
                identity.clone(),
                state.name,
                p.size,
                false,
                p.validator,
                plan::segments(p.size, 1, false),
              );
              state.mime = Some(p.mime);
              resume_body = vec![(0, p.body)];
            }
            Err(Error::Cancelled) => return Err(Error::Cancelled),
            _ => {
              state.segments = plan::segments(state.size, 1, false);
              state.ranges = false;
            }
          }
        }
        let size = state.size;
        target.state = Some(state);
        let file = target.part.try_clone()?;
        let dir = target
          .final_path
          .parent()
          .ok_or(Error::State)?
          .to_path_buf();
        let state_path = target.state_path.clone();
        blocking(move || {
          mark::free_space(&dir, size.unwrap_or(0))?;
          reset_part(&file, &state_path, size)
        })
        .await?;
      }
      let mut state = target.state.take().ok_or(Error::State)?;
      state.owner = state::identity(owner);
      let (dir, needed) = (
        target
          .final_path
          .parent()
          .ok_or(Error::State)?
          .to_path_buf(),
        state.size.unwrap_or(0).saturating_sub(state.bytes()),
      );
      blocking(move || mark::free_space(&dir, needed)).await?;
      let count = state.segments.len();
      let now = Instant::now();
      let shared = Arc::new(Shared {
        _lease: lease.clone(),
        file: Arc::new(target.part.try_clone()?),
        state_key: state_key.clone(),
        board: Mutex::new(Board {
          state,
          active: HashSet::new(),
          writing: HashSet::new(),
          speeds: vec![(now, 0); count],
        }),
        pause: tokio::sync::Mutex::new(now),
        limit: AtomicUsize::new(if attempt > 0 { 1 } else { self.connections }),
        io: Arc::new(tokio::sync::RwLock::new(())),
      });
      checkpoint(&shared, &target.state_path).await?;
      let result = self
        .transfer(
          &transfer_url,
          &shared,
          std::mem::take(&mut resume_body),
          &target.state_path,
          cancel.clone(),
          event,
        )
        .await;
      if matches!(result, Err(Error::Restart)) && attempt == 0 {
        drop(shared);
        initial = Some(self.probe(url, cancel.clone()).await?);
        continue;
      }
      result?;
      if !*cancel.borrow() {
        return Err(Error::Cancelled);
      }
      checkpoint(&shared, &target.state_path).await?;
      let path = target.final_path.clone();
      let part_path = target.part_path.clone();
      let state_path = target.state_path.clone();
      return blocking(move || {
        let state = shared.board.lock().map_err(|_| Error::State)?.state.clone();
        let n = state.bytes();
        if n == 0 {
          return Err(Error::Empty);
        }
        if state.size.is_some_and(|size| size != n) {
          return Err(Error::Network);
        }
        if shared.file.metadata()?.len() != n {
          return Err(Error::Network);
        }
        reject_error_body(&shared.file, n, &state.name, state.mime.as_deref())?;
        shared.file.sync_data()?;
        mark::mark(&shared.file, &part_path);
        shared.file.sync_all()?;
        let mut path = path;
        let original = path
          .file_name()
          .and_then(|n| n.to_str())
          .ok_or(Error::State)?
          .to_owned();
        for n in 0..=1000 {
          let candidate = path
            .parent()
            .ok_or(Error::State)?
            .join(state::alternative(&original, n));
          match state::publish(&part_path, &candidate) {
            Ok(()) => {
              path = candidate;
              break;
            }
            Err(_) if candidate.symlink_metadata().is_ok() && n < 1000 => continue,
            Err(e) => return Err(e),
          }
        }
        if let Err(error) = std::fs::remove_file(state_path) {
          // Publication succeeded. An AV-held sidecar must not turn completion into failure.
          tracing::warn!(error_kind=?error.kind(), "Published file checkpoint cleanup deferred");
        }
        state::sync_directory(path.parent().ok_or(Error::State)?)?;
        Ok(path)
      })
      .await;
    }
    Err(Error::Restart)
  }
  async fn transfer(
    &self,
    url: &str,
    shared: &Arc<Shared>,
    initial: Vec<(usize, Response)>,
    state_path: &Path,
    mut cancel: watch::Receiver<bool>,
    event: &mut (dyn FnMut(Event) + Send),
  ) -> Result<()> {
    let (updates, mut rx) = mpsc::channel(64);
    let (stop_tx, stop) = watch::channel(true);
    let mut workers = JoinSet::new();
    let mut initial = initial;
    let connections = shared.limit.load(Ordering::Relaxed);
    let count = shared
      .board
      .lock()
      .map_err(|_| Error::State)?
      .state
      .segments
      .len()
      .min(connections);
    for worker in 0..count {
      let first = if worker == 0 {
        initial.first().map(|(i, _)| *i).or_else(|| {
          shared
            .board
            .lock()
            .ok()?
            .state
            .segments
            .iter()
            .position(|s| s.remaining() != Some(0))
        })
      } else {
        None
      };
      let index = {
        let mut b = shared.board.lock().map_err(|_| Error::State)?;
        if let Some(i) = first {
          b.active.insert(i);
          Some(i)
        } else {
          b.take(connections)
        }
      };
      let Some(index) = index else { break };
      let body = initial
        .iter()
        .position(|(i, _)| *i == index)
        .map(|position| initial.remove(position).1);
      let (engine, url, shared, updates, stop) = (
        self.clone(),
        url.to_owned(),
        shared.clone(),
        updates.clone(),
        stop.clone(),
      );
      workers.spawn(async move {
        engine
          .worker(
            &url,
            Assignment {
              worker,
              index,
              body,
            },
            shared,
            updates,
            stop,
          )
          .await
      });
    }
    drop(initial);
    drop(updates);
    let mut timer = tokio::time::interval(Duration::from_secs(1));
    timer.tick().await;
    let mut first_checkpoint = true;
    let mut last = (
      Instant::now(),
      shared.board.lock().map_err(|_| Error::State)?.state.bytes(),
    );
    let mut speed = 0.0;
    let mut result = Ok(());
    let mut updates_open = true;
    while !workers.is_empty() {
      tokio::select! {biased;
        changed=cancel.changed()=>{let _=changed;result=Err(Error::Cancelled);break;},
        joined=workers.join_next()=>{match joined {Some(Ok(Ok(())))=>{},Some(Ok(Err(e)))=>{result=Err(e);break;},Some(Err(_))=>{result=Err(Error::Io);break;},None=>{}}},
        _=timer.tick()=>{
          let bytes=checkpoint(shared,state_path).await?;event(Event::Checkpoint{bytes});
        },
        update=rx.recv(),if updates_open=>{
          if update.is_none() {updates_open=false;}
          if update.is_some() {
            let (bytes,total)={let b=shared.board.lock().map_err(|_|Error::State)?;(b.state.bytes(),b.state.size)};
            let elapsed=last.0.elapsed().as_secs_f64();
            if elapsed>=0.2 || first_checkpoint {
              let current=bytes.saturating_sub(last.1) as f64/elapsed.max(0.001);speed=if speed==0.0 {current}else{speed*0.75+current*0.25};last=(Instant::now(),bytes);
              event(Event::Progress{bytes,total,speed});
            }
            if first_checkpoint {let bytes=checkpoint(shared,state_path).await?;event(Event::Checkpoint{bytes});first_checkpoint=false;}
          }
        },
      }
      if !*cancel.borrow() {
        result = Err(Error::Cancelled);
        break;
      }
    }
    let _ = stop_tx.send(false);
    while workers.join_next().await.is_some() {}
    // A reduced connection limit can leave a segment unassigned after the last accepted
    // connection finishes. Drain those segments with one worker after all others exit.
    if result.is_ok() {
      let next = shared.board.lock().map_err(|_| Error::State)?.take(1);
      if let Some(index) = next {
        let (updates, _rx) = mpsc::channel(1);
        result = self
          .worker(
            url,
            Assignment {
              worker: 0,
              index,
              body: None,
            },
            shared.clone(),
            updates,
            cancel.clone(),
          )
          .await;
      }
    }
    let bytes = checkpoint(shared, state_path).await?;
    event(Event::Checkpoint { bytes });
    tokio::task::yield_now().await;
    if result.is_ok() {
      let b = shared.board.lock().map_err(|_| Error::State)?;
      event(Event::Progress {
        bytes: b.state.bytes(),
        total: b.state.size,
        speed,
      });
    }
    result
  }
  async fn worker(
    &self,
    url: &str,
    assignment: Assignment,
    shared: Arc<Shared>,
    updates: mpsc::Sender<()>,
    mut cancel: watch::Receiver<bool>,
  ) -> Result<()> {
    let Assignment {
      worker,
      mut index,
      mut body,
    } = assignment;
    loop {
      let mut failures = 0;
      let mut previous = 0;
      loop {
        let until = *shared.pause.lock().await;
        wait(&mut cancel, async {
          tokio::time::sleep_until(until.into()).await;
          Ok(())
        })
        .await?;
        let before = shared
          .board
          .lock()
          .map_err(|_| Error::State)?
          .state
          .segments[index]
          .done;
        if before > previous {
          failures = 0;
        }
        previous = before;
        let response = if let Some(body) = body.take() {
          Ok(body)
        } else {
          let (start, end, validator) = {
            let b = shared.board.lock().map_err(|_| Error::State)?;
            let s = &b.state.segments[index];
            (
              s.start + s.done,
              s.end.map(|n| n - 1),
              b.state.validator.if_range().map(str::to_owned),
            )
          };
          if let Some(validator) = validator {
            self
              .client
              .get(url, Some((start, end, &validator)), cancel.clone())
              .await
          } else {
            return Err(Error::Restart);
          }
        };
        let result = match response {
          Ok(response) => {
            let status = response.response.status().as_u16();
            if status == 429 || status == 503 {
              let delay = plan::retry_after(
                response
                  .response
                  .headers()
                  .get("retry-after")
                  .and_then(|h| h.to_str().ok()),
                SystemTime::now(),
              )?
              .unwrap_or_else(|| plan::backoff(failures));
              *shared.pause.lock().await = Instant::now() + delay;
              if status == 429 {
                shared.limit.store(1, Ordering::Relaxed);
              }
              Err(Error::Network)
            } else if status == 403 {
              shared.limit.store(1, Ordering::Relaxed);
              Err(Error::Network)
            } else if plan::TRANSIENT_STATUS.contains(&status) {
              Err(Error::Network)
            } else {
              self
                .consume(index, response, &shared, &updates, cancel.clone())
                .await
            }
          }
          Err(e) => Err(e),
        };
        match result {
          Ok(()) => break,
          Err(Error::Network) => {
            let after = shared
              .board
              .lock()
              .map_err(|_| Error::State)?
              .state
              .segments[index]
              .done;
            if after > before {
              failures = 0;
            } else {
              failures += 1;
            }
            if failures >= 5 {
              return Err(Error::Network);
            }
            if worker >= shared.limit.load(Ordering::Relaxed) {
              let mut b = shared.board.lock().map_err(|_| Error::State)?;
              b.active.remove(&index);
              return Ok(());
            }
            wait(&mut cancel, async {
              tokio::time::sleep(plan::backoff(failures)).await;
              Ok(())
            })
            .await?;
          }
          Err(e) => return Err(e),
        }
      }
      let mut b = shared.board.lock().map_err(|_| Error::State)?;
      b.active.remove(&index);
      if worker >= shared.limit.load(Ordering::Relaxed) {
        return Ok(());
      }
      let Some(next) = b.take(shared.limit.load(Ordering::Relaxed)) else {
        return Ok(());
      };
      index = next;
    }
  }
  async fn consume(
    &self,
    index: usize,
    mut body: Response,
    shared: &Arc<Shared>,
    updates: &mpsc::Sender<()>,
    mut cancel: watch::Receiver<bool>,
  ) -> Result<()> {
    if body.ranged && body.response.status() != reqwest::StatusCode::PARTIAL_CONTENT {
      return Err(Error::Restart);
    }
    let ranged = body.ranged;
    let (start, end) = {
      let b = shared.board.lock().map_err(|_| Error::State)?;
      let s = &b.state.segments[index];
      let start = s.start + s.done;
      let end = s.end.map(|n| n - 1);
      if body.response.status() == reqwest::StatusCode::PARTIAL_CONTENT {
        valid_range(&body, &b.state, start, end)?;
      } else if body.response.status() != reqwest::StatusCode::OK || start != 0 {
        return Err(Error::Restart);
      }
      (start, end)
    };
    if body
      .response
      .headers()
      .get_all("content-encoding")
      .iter()
      .any(|v| {
        v.to_str()
          .map_or(true, |v| !v.trim().eq_ignore_ascii_case("identity"))
      })
    {
      return Err(Error::Encoding);
    }
    // Secondary requests must be conditional 206 responses, even for a zero-offset segment.
    if start > 0 && body.response.status() != reqwest::StatusCode::PARTIAL_CONTENT {
      return Err(Error::Restart);
    }
    let mut buffer = Vec::with_capacity(BATCH);
    let mut last = Instant::now();
    let mut received = 0u64;
    loop {
      let until = *shared.pause.lock().await;
      wait(&mut cancel, async {
        tokio::time::sleep_until(until.into()).await;
        Ok(())
      })
      .await?;
      let chunk = tokio::select! {
        result=wait(&mut cancel,async {body.response.chunk().await.map_err(super::client::network_error)})=>result?,
        _=tokio::time::sleep_until((last+Duration::from_millis(200)).into()),if !buffer.is_empty()=>{
          write(shared,index,std::mem::take(&mut buffer)).await?;
          let _=updates.try_send(());last=Instant::now();
          if shared.board.lock().map_err(|_|Error::State)?.state.segments[index].remaining()==Some(0) {return Ok(());}
          continue;
        }
      };
      match chunk {
        Some(chunk) => {
          received = received
            .checked_add(chunk.len() as u64)
            .ok_or(Error::Network)?;
          if ranged && end.is_some_and(|end| received > end - start + 1) {
            return Err(Error::Network);
          }
          for data in chunk.chunks(BATCH) {
            buffer.extend_from_slice(data);
            if buffer.len() >= BATCH || last.elapsed() >= Duration::from_millis(200) {
              write(shared, index, std::mem::take(&mut buffer)).await?;
              let _ = updates.try_send(());
              last = Instant::now();
              if shared
                .board
                .lock()
                .map_err(|_| Error::State)?
                .state
                .segments[index]
                .remaining()
                == Some(0)
              {
                return Ok(());
              }
            }
          }
        }
        None => {
          if !buffer.is_empty() {
            write(shared, index, buffer).await?;
            let _ = updates.try_send(());
          }
          let mut b = shared.board.lock().map_err(|_| Error::State)?;
          if b.state.segments[index].end.is_none() {
            let end = b.state.segments[index].start + b.state.segments[index].done;
            b.state.segments[index].end = Some(end);
            b.state.size = Some(end);
          }
          if b.state.segments[index].remaining().is_some_and(|n| n > 0) {
            return Err(Error::Network);
          }
          return Ok(());
        }
      }
    }
  }
}
fn valid_range(body: &Response, state: &State, start: u64, end: Option<u64>) -> Result<()> {
  if body.response.status() != reqwest::StatusCode::PARTIAL_CONTENT {
    return Err(Error::Restart);
  }
  let range = body
    .response
    .headers()
    .get("content-range")
    .and_then(|h| h.to_str().ok())
    .ok_or(Error::Restart)?;
  let (a, b, total) = plan::content_range(range)?;
  if a != start
    || end != Some(b)
    || state.size != Some(total)
    || !state
      .validator
      .matches(&Validator::from_headers(body.response.headers()))
    || body
      .response
      .content_length()
      .is_some_and(|len| len != b - a + 1)
  {
    return Err(Error::Restart);
  }
  Ok(())
}
async fn write(shared: &Arc<Shared>, index: usize, data: Vec<u8>) -> Result<()> {
  write_using(shared, index, data, positional_write).await
}
async fn write_using(
  shared: &Arc<Shared>,
  index: usize,
  data: Vec<u8>,
  output: impl FnOnce(&File, &[u8], u64) -> Result<()> + Send + 'static,
) -> Result<()> {
  let permit = shared.io.clone().read_owned().await;
  let (offset, n) = {
    let mut board = shared.board.lock().map_err(|_| Error::State)?;
    let s = &board.state.segments[index];
    let n = s
      .remaining()
      .map_or(data.len(), |n| data.len().min(n as usize));
    let offset = s.start + s.done;
    board.writing.insert(index);
    (offset, n)
  };
  let shared = shared.clone();
  blocking(move || {
    // Keep the I/O lease inside the non-abortable operation. A checkpoint waits for
    // writes to finish and commit their counters; progress never waits on disk under board.
    let _permit = permit;
    let result = output(&shared.file, &data[..n], offset);
    let mut board = shared.board.lock().map_err(|_| Error::State)?;
    board.writing.remove(&index);
    if result.is_ok() {
      board.state.segments[index].done += n as u64;
    }
    result
  })
  .await
}
async fn checkpoint(shared: &Arc<Shared>, path: &Path) -> Result<u64> {
  let permit = shared.io.clone().write_owned().await;
  let state = shared.board.lock().map_err(|_| Error::State)?.state.clone();
  let (shared, path) = (shared.clone(), path.to_path_buf());
  blocking(move || {
    let _permit = permit;
    state.save(&path, &shared.file, &shared.state_key)?;
    Ok(state.bytes())
  })
  .await
}
fn positional_write(file: &File, mut data: &[u8], mut offset: u64) -> Result<()> {
  while !data.is_empty() {
    #[cfg(unix)]
    let n = {
      use std::os::unix::fs::FileExt;
      file.write_at(data, offset)?
    };
    #[cfg(windows)]
    let n = {
      use std::os::windows::fs::FileExt;
      file.seek_write(data, offset)?
    };
    if n == 0 {
      return Err(Error::Io);
    }
    offset += n as u64;
    data = &data[n..];
  }
  Ok(())
}
fn reset_part(file: &File, checkpoint: &Path, size: Option<u64>) -> Result<()> {
  state::invalidate(checkpoint)?;
  file.set_len(0)?;
  sparse(file)?;
  if let Some(n) = size {
    file.set_len(n)?;
  }
  Ok(())
}
fn reject_error_body(file: &File, size: u64, name: &str, mime: Option<&str>) -> Result<()> {
  let extension = name
    .rsplit('.')
    .next()
    .unwrap_or_default()
    .to_ascii_lowercase();
  let mut bytes = vec![0; size.min(512) as usize];
  #[cfg(unix)]
  {
    use std::os::unix::fs::FileExt;
    file.read_exact_at(&mut bytes, 0)?;
  }
  #[cfg(windows)]
  {
    use std::os::windows::fs::FileExt;
    let n = file.seek_read(&mut bytes, 0)?;
    bytes.truncate(n);
  }
  let claimed = match mime.unwrap_or_default() {
    "application/pdf" => "pdf",
    "application/zip" | "application/x-zip-compressed" => "zip",
    // This legacy MIME is a Windows-download bucket, including MSI and scripts (Apache mime.types).
    "application/x-msdownload" => match extension.as_str() {
      "msi" | "bat" | "cmd" | "com" => extension.as_str(),
      _ => "windows",
    },
    "application/vnd.microsoft.portable-executable" => "exe",
    "application/x-7z-compressed" => "7z",
    "application/x-rar-compressed" | "application/vnd.rar" => "rar",
    "application/gzip" | "application/x-gzip" => "gz",
    _ => extension.as_str(),
  };
  let matches_magic = |kind: &str| match kind {
    "pdf" => bytes.starts_with(b"%PDF-"),
    "zip" => [b"PK\x03\x04", b"PK\x05\x06", b"PK\x07\x08"]
      .iter()
      .any(|m| bytes.starts_with(*m)),
    "exe" | "dll" => bytes.starts_with(b"MZ"),
    "msi" => bytes.starts_with(b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1"),
    "windows" => bytes.starts_with(b"MZ") || bytes.starts_with(b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1"),
    "7z" => bytes.starts_with(b"7z\xbc\xaf\x27\x1c"),
    "rar" => bytes.starts_with(b"Rar!\x1a\x07\x00") || bytes.starts_with(b"Rar!\x1a\x07\x01\x00"),
    "gz" | "tgz" => bytes.starts_with(b"\x1f\x8b"),
    _ => true,
  };
  if !matches_magic(claimed) || !matches_magic(&extension) {
    return Err(Error::NotFile);
  }
  if matches!(extension.as_str(), "html" | "htm" | "xhtml") {
    return Ok(());
  }
  let text = String::from_utf8_lossy(&bytes)
    .trim_start()
    .to_ascii_lowercase();
  let plain_text = std::str::from_utf8(&bytes)
    .is_ok_and(|s| s.chars().all(|c| !c.is_control() || c.is_whitespace()));
  let file_magic =
    bytes.starts_with(b"%PDF-") || bytes.starts_with(b"MZ") || bytes.starts_with(b"PK");
  if text.starts_with("<!doctype html")
    || text.starts_with("<html")
    || (size <= 512
      && matches!(
        extension.as_str(),
        "zip" | "pdf" | "exe" | "dmg" | "iso" | "7z" | "rar"
      )
      && plain_text
      && !file_magic)
  {
    return Err(Error::NotFile);
  }
  Ok(())
}
fn sparse(file: &File) -> Result<()> {
  #[cfg(windows)]
  {
    use std::os::windows::io::AsRawHandle;
    let mut returned = 0;
    unsafe {
      windows_sys::Win32::System::IO::DeviceIoControl(
        file.as_raw_handle().cast(),
        windows_sys::Win32::System::Ioctl::FSCTL_SET_SPARSE,
        std::ptr::null(),
        0,
        std::ptr::null_mut(),
        0,
        &mut returned,
        std::ptr::null_mut(),
      );
    }
  }
  #[cfg(not(windows))]
  let _ = file;
  Ok(())
}

#[cfg(test)]
mod io_tests {
  use super::*;
  #[test]
  fn restart_retires_checkpoint_before_truncation_and_failed_retirement_keeps_bytes() {
    let dir = std::env::temp_dir().join(format!("nezzel-reset-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&dir).unwrap();
    let key = StateKey::ephemeral();
    let id = state::identity("https://files.example/reset.bin");
    let target = state::target(&dir, &id, "reset.bin", &key).unwrap();
    positional_write(&target.part, b"original", 0).unwrap();
    // A checkpoint that cannot be removed must prevent any destructive file operation.
    std::fs::create_dir(&target.state_path).unwrap();
    assert!(reset_part(&target.part, &target.state_path, Some(8)).is_err());
    assert_eq!(std::fs::read(&target.part_path).unwrap(), b"original");
    std::fs::remove_dir(&target.state_path).unwrap();
    let mut saved = State::new(
      id.clone(),
      "reset.bin".into(),
      Some(8),
      true,
      Validator::default(),
      plan::segments(Some(8), 1, false),
    );
    saved.segments[0].done = 4;
    saved.save(&target.state_path, &target.part, &key).unwrap();
    reset_part(&target.part, &target.state_path, Some(8)).unwrap();
    assert!(!target.state_path.exists());
    assert_eq!(std::fs::read(&target.part_path).unwrap(), [0; 8]);
    // Simulate death after truncation/preallocation and before the new checkpoint.
    // Even a full-length part must never resurrect the old completed offsets.
    assert!(state::resume(&dir, &id, &key).unwrap().is_none());
    drop(target);
    std::fs::remove_dir_all(dir).unwrap();
  }
  #[tokio::test]
  async fn slow_disk_does_not_hold_board_and_checkpoint_waits_for_committed_bytes() {
    let dir = std::env::temp_dir().join(format!("nezzel-io-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&dir).unwrap();
    let key = Arc::new(StateKey::ephemeral());
    let target = state::target(
      &dir,
      &state::identity("https://files.example/io.zip"),
      "io.zip",
      &key,
    )
    .unwrap();
    let lease = Arc::new(tokio::sync::Mutex::new(())).lock_owned().await;
    let state = State::new(
      state::identity("https://files.example/io.zip"),
      "io.zip".into(),
      Some(4),
      false,
      Validator::default(),
      plan::segments(Some(4), 1, false),
    );
    let shared = Arc::new(Shared {
      file: Arc::new(target.part.try_clone().unwrap()),
      state_key: key.clone(),
      board: Mutex::new(Board {
        state,
        active: HashSet::new(),
        writing: HashSet::new(),
        speeds: vec![(Instant::now(), 0)],
      }),
      pause: tokio::sync::Mutex::new(Instant::now()),
      limit: AtomicUsize::new(1),
      io: Arc::new(tokio::sync::RwLock::new(())),
      _lease: Arc::new(lease),
    });
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let worker_shared = shared.clone();
    let writer = tokio::spawn(async move {
      write_using(
        &worker_shared,
        0,
        b"data".to_vec(),
        move |file, data, offset| {
          let _ = entered_tx.send(());
          release_rx.recv().map_err(|_| Error::Io)?;
          positional_write(file, data, offset)
        },
      )
      .await
    });
    entered_rx.await.unwrap();
    {
      let board = shared
        .board
        .try_lock()
        .expect("disk I/O must not hold the board");
      assert_eq!(board.state.bytes(), 0);
    }
    let mut checkpoint = Box::pin(checkpoint(&shared, &target.state_path));
    assert!(matches!(
      futures::poll!(&mut checkpoint),
      std::task::Poll::Pending
    ));
    release_tx.send(()).unwrap();
    writer.await.unwrap().unwrap();
    assert_eq!(checkpoint.await.unwrap(), 4);
    assert_eq!(State::load(&target.state_path, &key).unwrap().bytes(), 4);
    assert_eq!(std::fs::read(&target.part_path).unwrap(), b"data");
    drop(shared);
    drop(target);
    std::fs::remove_dir_all(dir).unwrap();
  }
}
