//! Both capture windows' offers as one lifecycle under one lock: claim, adopt, upgrade, dismiss, report, retry.

use super::choice::Choice;
use super::outbox::Outbox;
use super::policy::{supported_url, CapturePolicy};
use super::preview::{self, FailReason};
use super::window::{CaptureWindow, CardTimer};
use serde::Serialize;
use std::{
  sync::{
    atomic::{AtomicBool, AtomicU64},
    Mutex,
  },
  time::Instant,
};

/// What a capture window asks for, by its wire word (`capture_action`, the main window's queue, usage keys).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
  Download,
  Options,
  Adopt,
  Dismiss,
  Report,
}

impl Action {
  pub const ALL: [Self; 5] = [
    Self::Download,
    Self::Options,
    Self::Adopt,
    Self::Dismiss,
    Self::Report,
  ];
  /// What a capture webview may send; `adopt` only comes from the main window's focus.
  pub const FROM_WINDOW: [Self; 4] = [Self::Download, Self::Options, Self::Dismiss, Self::Report];
  /// What claims an offer for the main window's queue; any other word settles the offer as `dismiss`.
  pub const CLAIMS: [Self; 3] = [Self::Download, Self::Options, Self::Adopt];

  pub const fn as_str(self) -> &'static str {
    match self {
      Self::Download => "download",
      Self::Options => "options",
      Self::Adopt => "adopt",
      Self::Dismiss => "dismiss",
      Self::Report => "report",
    }
  }

  pub fn parse(word: &str) -> Option<Self> {
    Self::ALL.into_iter().find(|action| action.as_str() == word)
  }
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Offer {
  pub remote_images: bool,
  pub id: u64,
  pub url: String,
  pub site: String,
  pub title: String,
  /// The preview finished (success or failure); independent of whether the item has a title.
  pub read: bool,
  pub details: preview::PreviewDetails,
  pub failed: bool,
  /// Why the read failed (`preview::reason_for_code` vocabulary); None while reading or after success.
  pub reason: Option<FailReason>,
  pub resolving: bool,
  /// Home/feed page with no specific post: no metadata fetch.
  pub feed: bool,
  #[serde(skip)]
  pub(super) adopt_on_focus: bool,
}

impl Offer {
  pub(super) fn with_network(network: &crate::state::config_models::NetworkSettings) -> Self {
    Self {
      remote_images: network.enable_proxy != Some(true),
      ..Self::default()
    }
  }
}

/// Who has settled an offer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Claim {
  #[default]
  None,
  /// The main window's focus took it as a row; a click in either window may still upgrade that row.
  Adopted,
  /// Finished: queued as a download, opened in options, or dismissed.
  Done(Action),
}

/// One capture window's current offer and everything that lives exactly as long as it.
pub(super) struct OfferSlot {
  pub(super) offer: Offer,
  pub(super) card_timer: CardTimer,
  /// The read's media item: None while reading, Some(None) when the read failed.
  pub(super) metadata: Option<Option<serde_json::Value>>,
  pub(super) claim: Claim,
  /// The read in flight.
  pub(super) job: Option<tauri::async_runtime::JoinHandle<()>>,
}

impl OfferSlot {
  fn new(mut offer: Offer) -> Self {
    offer.read = offer.feed;
    Self {
      card_timer: CardTimer::new(&offer),
      // Feed pages have no preview job. Deliver None immediately so the main window reads the list.
      metadata: offer.feed.then_some(None),
      offer,
      claim: Claim::None,
      job: None,
    }
  }
}

/// Where a read's media item stands, for an action that went out before the read finished.
#[derive(Debug, PartialEq)]
pub(super) enum ItemState {
  Read(Option<serde_json::Value>),
  Reading,
  /// No window holds that offer any more.
  Gone,
}

/// Both capture windows' offers under one lock: a claim reads and settles both windows at once.
#[derive(Default)]
pub(super) struct Offers {
  card: Option<OfferSlot>,
  bar: Option<OfferSlot>,
}

impl Offers {
  pub(super) fn slot(&self, window: CaptureWindow) -> Option<&OfferSlot> {
    match window {
      CaptureWindow::Card => self.card.as_ref(),
      CaptureWindow::Bar => self.bar.as_ref(),
    }
  }

  pub(super) fn slot_mut(&mut self, window: CaptureWindow) -> &mut Option<OfferSlot> {
    match window {
      CaptureWindow::Card => &mut self.card,
      CaptureWindow::Bar => &mut self.bar,
    }
  }

  /// The window's slot, only while it still holds offer `id`.
  pub(super) fn holding(&self, window: CaptureWindow, id: u64) -> Option<&OfferSlot> {
    self.slot(window).filter(|slot| slot.offer.id == id)
  }

  pub(super) fn holds(&self, window: CaptureWindow, id: u64) -> bool {
    self.holding(window, id).is_some()
  }

  pub(super) fn current(&self, window: CaptureWindow) -> Option<&Offer> {
    self.slot(window).map(|slot| &slot.offer)
  }

  /// Ends the window's offer: stops its read and forgets its item and claim. Every exit goes through here.
  pub(super) fn retire(&mut self, window: CaptureWindow) -> Option<Offer> {
    let slot = self.slot_mut(window).take()?;
    if let Some(job) = slot.job {
      job.abort();
    }
    Some(slot.offer)
  }

  /// A new link in the window replaces (and retires) the one it held.
  pub(super) fn replace(&mut self, window: CaptureWindow, offer: Offer) {
    self.retire(window);
    *self.slot_mut(window) = Some(OfferSlot::new(offer));
  }

  /// Attaches the read to offer `id`; a read for an offer the window no longer holds stops at once.
  pub(super) fn set_job(
    &mut self,
    window: CaptureWindow,
    id: u64,
    job: tauri::async_runtime::JoinHandle<()>,
  ) {
    match self.slot_mut(window) {
      Some(slot) if slot.offer.id == id => slot.job = Some(job),
      _ => job.abort(),
    }
  }

  /// Records a finished read on offer `id`; false when the window no longer holds it.
  pub(super) fn record_read(
    &mut self,
    window: CaptureWindow,
    id: u64,
    item: Option<serde_json::Value>,
    details: Option<preview::PreviewDetails>,
    reason: Option<FailReason>,
  ) -> bool {
    let Some(slot) = self
      .slot_mut(window)
      .as_mut()
      .filter(|slot| slot.offer.id == id)
    else {
      return false;
    };
    let current = &mut slot.offer;
    current.read = true;
    current.failed = item.is_none();
    current.reason = reason;
    if let Some(item) = &item {
      current.title = item
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .chars()
        .take(300)
        .collect();
      current.details = details.unwrap_or_else(|| preview::summarize(item));
    }
    slot.metadata = Some(item);
    true
  }

  pub(super) fn item(&self, id: u64) -> ItemState {
    let slot = CaptureWindow::ALL
      .into_iter()
      .find_map(|window| self.holding(window, id));
    match slot {
      Some(OfferSlot {
        metadata: Some(item),
        ..
      }) => ItemState::Read(item.clone()),
      Some(_) => ItemState::Reading,
      None => ItemState::Gone,
    }
  }

  /// Claims offer `id` in `window` for `action`; `Adopt` comes only from the main window's focus.
  /// `Report` and `Dismiss` settle the offer as `dismiss`.
  pub(super) fn claim(
    &mut self,
    window: CaptureWindow,
    id: u64,
    action: Action,
  ) -> Result<CaptureAction, String> {
    let current = self.holding(window, id).ok_or("offer expired")?;
    if action != Action::Dismiss && current.offer.url.is_empty() {
      return Err("unsupported URL".into());
    }
    let action = Some(action)
      .filter(|action| Action::CLAIMS.contains(action))
      .unwrap_or(Action::Dismiss);
    // The row's media item goes to the main window once, with the row's first claim (inline when read,
    // else awaited); an upgrade of an adopted row carries none.
    let mut first = current.claim == Claim::None;
    match current.claim {
      Claim::None => {}
      // X11 can focus the main window as the card is clicked: focus adopts the row first, and the
      // click must still start it (or open its options) instead of being dropped.
      Claim::Adopted if matches!(action, Action::Download | Action::Options) => {}
      _ => return Err("offer already handled".into()),
    }
    let url = current.offer.url.clone();
    // One claim per link across both capture windows, under this one lock: two windows (or a double
    // click racing the focus) can't both queue the same link.
    // Adopting never settles the other window: the bar may be the user's live interaction.
    let other = window.other();
    let other_slot = matches!(action, Action::Download | Action::Options)
      .then(|| self.slot(other).filter(|slot| slot.offer.url == url))
      .flatten();
    // The other window's offer was already adopted as a row: upgrade that row instead of a second one.
    // Only a focus adoption is a live row to upgrade. Any other claim is a finished action whose
    // offer lingers in its window (the card after its own Download): a fresh offer for the same
    // link must still work, so that stale offer is just retired below.
    let upgraded = other_slot.filter(|slot| slot.claim == Claim::Adopted);
    let row_id = upgraded.map_or(id, |slot| slot.offer.id);
    first &= upgraded.is_none();
    let item = self
      .slot(window)
      .filter(|_| first)
      .and_then(|slot| slot.metadata.clone())
      .flatten();
    let awaits_item = first && item.is_none();
    let retired = other_slot.is_some().then_some(other);
    if let Some(slot) = self.slot_mut(window) {
      slot.claim = match action {
        Action::Adopt => Claim::Adopted,
        action => Claim::Done(action),
      };
    }
    if let Some(other) = retired {
      self.retire(other);
    }
    Ok(CaptureAction {
      id: row_id,
      url,
      action: action.as_str().into(),
      item,
      choice: None,
      retired,
      awaits_item,
    })
  }

  /// The main window got focus: adopt the offer of a window that adopts on focus, if it was raised in
  /// the background and holds a link the app can take.
  pub(super) fn adopt(&mut self) -> Option<(CaptureWindow, CaptureAction)> {
    CaptureWindow::ALL
      .into_iter()
      .filter(|window| window.adopts_on_focus())
      .find_map(|window| {
        let current = self.current(window)?;
        if !current.adopt_on_focus || supported_url(&current.url).is_none() {
          return None;
        }
        let id = current.id;
        self
          .claim(window, id, Action::Adopt)
          .ok()
          .map(|action| (window, action))
      })
  }

  /// Esc/× settles the window's offer `id`, claimed or not; a stale id leaves the newer offer alone.
  pub(super) fn dismiss(&mut self, window: CaptureWindow, id: u64) {
    if self.holds(window, id) {
      self.retire(window);
    }
  }

  /// A failed read → "Report": hands over what the report needs and retires the offer.
  pub(super) fn take_report(&mut self, window: CaptureWindow, id: u64) -> Option<CaptureReport> {
    if !self
      .holding(window, id)
      .is_some_and(|slot| slot.offer.failed)
    {
      return None;
    }
    let offer = self.retire(window)?;
    Some(CaptureReport {
      url: offer.url,
      reason: offer.reason.unwrap_or(FailReason::Unknown),
    })
  }

  /// A failed read → "Retry": the link to read again as a fresh offer (which retires this one).
  pub(super) fn retry_url(&self, window: CaptureWindow) -> Option<String> {
    self
      .current(window)
      .filter(|offer| offer.failed)
      .map(|offer| offer.url.clone())
  }

  /// The card rows offer `id` actually showed.
  pub(super) fn shown_choices(&self, window: CaptureWindow, id: u64) -> Vec<preview::CardChoice> {
    self
      .holding(window, id)
      .map(|slot| slot.offer.details.choices.clone())
      .unwrap_or_default()
  }
}

pub struct CaptureState {
  pub(super) policy: Mutex<CapturePolicy>,
  pub(super) offers: Mutex<Offers>,
  pub(super) sequence: AtomicU64,
  pub(super) quick_request: AtomicU64,
  pub(super) ready: AtomicBool,
  /// Focus wakes the single watcher instead of starting a competing clipboard read.
  pub(super) clipboard_wake: tokio::sync::Notify,
  pub(super) started: Instant,
  pub(crate) shortcut: Mutex<Option<String>>,
  pub(super) changed: tokio::sync::Notify,
  pub(super) card_keep_open: AtomicBool,
  pub shortcut_failed: AtomicBool,
  /// Why the global shortcut cannot work in this session ("wayland"), when it can't.
  pub(crate) shortcut_reason: Mutex<Option<&'static str>>,
  /// Whether the Wayland compositor offers data-control (background clipboard reads); probed once.
  pub(super) data_control: tokio::sync::OnceCell<bool>,
  /// Whether the main window had focus when the quick bar opened; when it didn't, closing the
  /// bar gives focus back to the app the user came from instead of raising the main window.
  pub(super) main_was_focused: AtomicBool,
  pub(super) main_is_focused: AtomicBool,
  /// Everything sent to the main window, until it pulls past it (`capture_pull`).
  pub(super) pending: Mutex<Outbox>,
}

impl Default for CaptureState {
  fn default() -> Self {
    Self {
      policy: Mutex::default(),
      offers: Mutex::default(),
      sequence: AtomicU64::new(0),
      quick_request: AtomicU64::new(0),
      ready: AtomicBool::new(false),
      clipboard_wake: tokio::sync::Notify::new(),
      started: Instant::now(),
      shortcut: Mutex::default(),
      changed: tokio::sync::Notify::new(),
      card_keep_open: AtomicBool::new(false),
      shortcut_failed: AtomicBool::new(false),
      shortcut_reason: Mutex::default(),
      data_control: tokio::sync::OnceCell::new(),
      main_was_focused: AtomicBool::new(false),
      main_is_focused: AtomicBool::new(false),
      pending: Mutex::default(),
    }
  }
}

/// Failed read → "Report": the main window opens Report a problem with the reason word only;
/// the link goes along only if the user ticks "Include the link" there.
#[derive(Clone, Serialize)]
pub(super) struct CaptureReport {
  url: String,
  reason: FailReason,
}

#[cfg(test)]
impl CaptureReport {
  pub(super) fn for_test(url: &str) -> Self {
    Self {
      url: url.into(),
      reason: FailReason::Unknown,
    }
  }
}

#[derive(Clone, Serialize)]
pub struct CaptureAction {
  pub id: u64,
  pub url: String,
  pub action: String,
  pub item: Option<serde_json::Value>,
  /// Quick pick from the bar or card, or a card format row; None = the user's default choice.
  pub choice: Option<Choice>,
  /// The other capture window whose offer for the same link this claim settled (to hide).
  #[serde(skip)]
  pub retired: Option<CaptureWindow>,
  /// The row's item is still being read: it follows as its own message (`outbox::emit_action`).
  #[serde(skip)]
  pub(super) awaits_item: bool,
}

// Claim and snapshot only: metadata must never delay the initial queue event.
pub(super) fn claim_action(
  state: &CaptureState,
  window: CaptureWindow,
  id: u64,
  action: Action,
) -> Result<CaptureAction, String> {
  let payload = state.offers.lock().unwrap().claim(window, id, action)?;
  if payload.retired.is_some() {
    state.changed.notify_waiters();
  }
  Ok(payload)
}

pub(super) fn adopt_on_focus(state: &CaptureState) -> Option<(CaptureWindow, CaptureAction)> {
  state.offers.lock().unwrap().adopt()
}

pub(super) async fn wait_for_item(state: &CaptureState, id: u64) -> Option<serde_json::Value> {
  tokio::time::timeout(super::read::ITEM_WAIT, async {
    loop {
      let notified = state.changed.notified();
      tokio::pin!(notified);
      notified.as_mut().enable();
      match state.offers.lock().unwrap().item(id) {
        ItemState::Read(item) => return item,
        ItemState::Gone => return None,
        ItemState::Reading => {}
      }
      notified.await;
    }
  })
  .await
  .ok()
  .flatten()
}

/// Settles a window's offer after Esc/×. Infallible: a stale id or an already-claimed offer still closes.
pub(super) fn dismiss_offer(state: &CaptureState, window: CaptureWindow, id: u64) {
  state.offers.lock().unwrap().dismiss(window, id);
  state.changed.notify_waiters();
}

#[cfg(test)]
pub(super) mod tests {
  use super::*;

  #[test]
  fn read_completion_is_independent_of_title_and_success() {
    for item in [Some(serde_json::json!({"formats": []})), None] {
      let mut offers = Offers::default();
      offers.replace(
        CaptureWindow::Bar,
        Offer {
          id: 1,
          ..Offer::default()
        },
      );
      assert!(!offers.current(CaptureWindow::Bar).unwrap().read);
      assert!(offers.record_read(CaptureWindow::Bar, 1, item, None, None));
      let offer = offers.current(CaptureWindow::Bar).unwrap();
      assert!(offer.read);
      assert!(offer.title.is_empty());
    }
  }

  #[test]
  fn feeds_without_a_preview_job_deliver_a_fallback_read() {
    let mut offers = Offers::default();
    let offer = Offer {
      id: 1,
      url: "https://www.youtube.com/feed/subscriptions".into(),
      feed: true,
      ..Offer::default()
    };
    offers.replace(CaptureWindow::Bar, offer);
    assert_eq!(offers.item(1), ItemState::Read(None));
    assert!(offers.current(CaptureWindow::Bar).unwrap().read);
  }

  #[test]
  fn offer_blocks_remote_images_when_proxy_is_enabled_even_without_an_address() {
    let network = crate::state::config_models::NetworkSettings {
      enable_proxy: Some(true),
      proxy: None,
      ..Default::default()
    };
    let offer = Offer::with_network(&network);
    assert_eq!(serde_json::to_value(offer).unwrap()["remoteImages"], false);
  }

  #[test]
  fn offer_allows_remote_images_when_proxy_is_disabled() {
    let offer = Offer::with_network(&crate::state::config_models::NetworkSettings::default());
    assert!(offer.remote_images);
  }

  #[test]
  fn claims_follow_the_action_allowlist() {
    // Any action outside download|options|adopt settles the offer as "dismiss". Unknown words never get here:
    // `capture_action` refuses anything outside `Action::FROM_WINDOW`, and handles "report" itself.
    let state = CaptureState::default();
    pending_offer(&state, CaptureWindow::Bar, false);
    assert_eq!(
      claim_action(&state, CaptureWindow::Bar, 1, Action::Report)
        .unwrap()
        .action,
      "dismiss"
    );
    assert_eq!(
      claim_action(&state, CaptureWindow::Bar, 1, Action::Download)
        .err()
        .as_deref(),
      Some("offer already handled")
    );
    // A stale id and an unknown window both read as expired.
    assert_eq!(
      claim_action(&state, CaptureWindow::Bar, 2, Action::Download)
        .err()
        .as_deref(),
      Some("offer expired")
    );
    assert_eq!(
      claim_action(&state, CaptureWindow::Card, 1, Action::Download)
        .err()
        .as_deref(),
      Some("offer expired")
    );
    // A link the app cannot take only dismisses.
    edit_slot(&state, CaptureWindow::Bar, |slot| {
      slot.offer.url.clear();
      slot.claim = Claim::None;
    });
    assert_eq!(
      claim_action(&state, CaptureWindow::Bar, 1, Action::Options)
        .err()
        .as_deref(),
      Some("unsupported URL")
    );
    assert_eq!(
      claim_action(&state, CaptureWindow::Bar, 1, Action::Dismiss)
        .unwrap()
        .action,
      "dismiss"
    );
  }

  #[test]
  fn adopting_never_retires_the_other_window_and_a_second_adopt_is_refused() {
    let state = CaptureState::default();
    same_link_in_both_windows(&state);
    let adopted = claim_action(&state, CaptureWindow::Card, 1, Action::Adopt).unwrap();
    assert!(adopted.retired.is_none());
    assert!(has_offer(&state, CaptureWindow::Bar));
    assert_eq!(
      claim_action(&state, CaptureWindow::Card, 1, Action::Adopt)
        .err()
        .as_deref(),
      Some("offer already handled")
    );
    // A finished download is not upgraded by a later adopt or dismiss claim.
    let state = CaptureState::default();
    pending_offer(&state, CaptureWindow::Card, true);
    claim_action(&state, CaptureWindow::Card, 1, Action::Download).unwrap();
    for action in [Action::Adopt, Action::Dismiss] {
      assert_eq!(
        claim_action(&state, CaptureWindow::Card, 1, action)
          .err()
          .as_deref(),
        Some("offer already handled"),
        "{action:?}"
      );
    }
  }

  #[tokio::test]
  async fn retiring_the_other_window_drops_its_metadata_claim_and_read_job() {
    let state = CaptureState::default();
    same_link_in_both_windows(&state);
    edit_slot(&state, CaptureWindow::Card, |slot| {
      slot.metadata = Some(None)
    });
    edit_slot(&state, CaptureWindow::Bar, |slot| {
      slot.metadata = Some(None)
    });
    let stopped = reading(&state, CaptureWindow::Card, 1);
    let action = claim_action(&state, CaptureWindow::Bar, 2, Action::Download).unwrap();
    assert_eq!(action.retired, Some(CaptureWindow::Card));
    assert!(!has_offer(&state, CaptureWindow::Card));
    assert_read_stopped(stopped).await;
    let offers = state.offers.lock().unwrap();
    assert_eq!(offers.item(1), ItemState::Gone);
    assert_eq!(offers.item(2), ItemState::Read(None));
  }

  #[test]
  fn dismiss_forgets_metadata_and_stops_the_read_for_the_current_offer_only() {
    let state = CaptureState::default();
    pending_offer(&state, CaptureWindow::Card, false);
    edit_slot(&state, CaptureWindow::Card, |slot| {
      slot.metadata = Some(None)
    });
    dismiss_offer(&state, CaptureWindow::Card, 1);
    assert_eq!(state.offers.lock().unwrap().item(1), ItemState::Gone);
    // A stale id leaves the newer offer, its claim and its read alone (a claim lives only on its offer).
    pending_offer(&state, CaptureWindow::Card, false);
    edit_slot(&state, CaptureWindow::Card, |slot| {
      slot.claim = Claim::Done(Action::Download);
    });
    let _read = reading(&state, CaptureWindow::Card, 1);
    dismiss_offer(&state, CaptureWindow::Card, 7);
    let offers = state.offers.lock().unwrap();
    let slot = offers.slot(CaptureWindow::Card).unwrap();
    assert_eq!(slot.claim, Claim::Done(Action::Download));
    assert!(slot.job.is_some());
  }

  pub(in crate::capture) fn pending_offer(
    state: &CaptureState,
    window: CaptureWindow,
    adoptable: bool,
  ) {
    state.offers.lock().unwrap().replace(
      window,
      Offer {
        id: 1,
        url: "https://www.youtube.com/watch?v=capture".into(),
        adopt_on_focus: adoptable,
        ..Offer::default()
      },
    );
  }

  /// Changes the window's offer slot in place: a test's stand-in for a finished read or an earlier claim.
  fn edit_slot(state: &CaptureState, window: CaptureWindow, edit: impl FnOnce(&mut OfferSlot)) {
    edit(
      state
        .offers
        .lock()
        .unwrap()
        .slot_mut(window)
        .as_mut()
        .unwrap(),
    );
  }

  fn has_offer(state: &CaptureState, window: CaptureWindow) -> bool {
    state.offers.lock().unwrap().slot(window).is_some()
  }

  /// Attaches a read that never ends to offer `id`; the receiver errors once the read is stopped.
  fn reading(
    state: &CaptureState,
    window: CaptureWindow,
    id: u64,
  ) -> tokio::sync::oneshot::Receiver<()> {
    let (alive, stopped) = tokio::sync::oneshot::channel::<()>();
    let job = tauri::async_runtime::spawn(async move {
      let _alive = alive;
      std::future::pending::<()>().await;
    });
    state.offers.lock().unwrap().set_job(window, id, job);
    stopped
  }

  async fn assert_read_stopped(stopped: tokio::sync::oneshot::Receiver<()>) {
    let stopped = tokio::time::timeout(std::time::Duration::from_secs(5), stopped).await;
    assert!(
      matches!(stopped, Ok(Err(_))),
      "the retired offer's read stops"
    );
  }

  #[test]
  fn focus_adopts_card_once_and_a_racing_click_still_acts_once() {
    let state = CaptureState::default();
    pending_offer(&state, CaptureWindow::Card, true);
    let (window, payload) = adopt_on_focus(&state).unwrap();
    assert_eq!(window, CaptureWindow::Card);
    assert_eq!(payload.action, "adopt");
    assert_eq!(payload.id, 1);
    assert!(payload.item.is_none());
    assert!(adopt_on_focus(&state).is_none());
    // A click racing the focus upgrades the adopted row once; nothing claims it after that.
    assert_eq!(
      claim_action(&state, CaptureWindow::Card, 1, Action::Download)
        .unwrap()
        .action,
      "download"
    );
    for action in [Action::Download, Action::Options, Action::Adopt] {
      assert_eq!(
        claim_action(&state, CaptureWindow::Card, 1, action)
          .err()
          .as_deref(),
        Some("offer already handled")
      );
    }
  }

  #[test]
  fn focus_ignores_bar_foreground_created_and_unsupported_offers() {
    let state = CaptureState::default();
    pending_offer(&state, CaptureWindow::Bar, true);
    assert!(adopt_on_focus(&state).is_none());
    pending_offer(&state, CaptureWindow::Card, false);
    assert!(adopt_on_focus(&state).is_none());
    edit_slot(&state, CaptureWindow::Card, |slot| {
      slot.offer.adopt_on_focus = true;
      slot.offer.url = "https://example.com".into();
    });
    assert!(adopt_on_focus(&state).is_none());
    edit_slot(&state, CaptureWindow::Card, |slot| {
      assert_eq!(slot.claim, Claim::None);
    });
  }

  /// The card (id 1) and the bar (id 2) both hold the same link.
  fn same_link_in_both_windows(state: &CaptureState) {
    pending_offer(state, CaptureWindow::Card, true);
    lingering_bar_offer(state);
  }

  /// The bar gets a fresh offer (id 2) for the link the card holds.
  fn lingering_bar_offer(state: &CaptureState) {
    let mut offers = state.offers.lock().unwrap();
    let mut bar = offers.current(CaptureWindow::Card).unwrap().clone();
    bar.id = 2;
    offers.replace(CaptureWindow::Bar, bar);
  }

  #[test]
  fn one_link_is_claimed_once_across_both_capture_windows() {
    let state = CaptureState::default();
    same_link_in_both_windows(&state);
    let first = claim_action(&state, CaptureWindow::Bar, 2, Action::Download).unwrap();
    assert_eq!(first.retired, Some(CaptureWindow::Card));
    // The racing click in the other window finds its offer settled: one queued download, not two.
    assert_eq!(
      claim_action(&state, CaptureWindow::Card, 1, Action::Download)
        .err()
        .as_deref(),
      Some("offer expired")
    );
    assert!(adopt_on_focus(&state).is_none());
    // The retired offer leaves no claim or metadata behind.
    assert!(!has_offer(&state, CaptureWindow::Card));
    assert_eq!(state.offers.lock().unwrap().item(1), ItemState::Gone);
    assert!(has_offer(&state, CaptureWindow::Bar));
  }

  #[test]
  fn a_link_focus_already_adopted_is_upgraded_not_queued_again() {
    let state = CaptureState::default();
    same_link_in_both_windows(&state);
    assert_eq!(adopt_on_focus(&state).unwrap().1.id, 1);
    let upgrade = claim_action(&state, CaptureWindow::Bar, 2, Action::Download).unwrap();
    // Same row id as the adopted one: the main window upgrades it instead of adding a second row.
    assert_eq!((upgrade.id, upgrade.action.as_str()), (1, "download"));
    assert_eq!(
      claim_action(&state, CaptureWindow::Bar, 2, Action::Download)
        .err()
        .as_deref(),
      Some("offer already handled")
    );
  }

  #[test]
  fn a_fresh_bar_offer_works_after_the_card_for_the_same_link_was_downloaded() {
    let state = CaptureState::default();
    pending_offer(&state, CaptureWindow::Card, false);
    claim_action(&state, CaptureWindow::Card, 1, Action::Download).unwrap();
    // The card's offer lingers (claimed) while the owner opens the bar on the same link.
    lingering_bar_offer(&state);
    let payload = claim_action(&state, CaptureWindow::Bar, 2, Action::Download).unwrap();
    assert_eq!(
      (payload.id, payload.retired),
      (2, Some(CaptureWindow::Card))
    );
    // The same bar offer still can't be queued twice.
    assert_eq!(
      claim_action(&state, CaptureWindow::Bar, 2, Action::Download)
        .err()
        .as_deref(),
      Some("offer already handled")
    );
  }

  #[test]
  fn dismiss_settles_even_a_claimed_or_stale_offer() {
    let state = CaptureState::default();
    pending_offer(&state, CaptureWindow::Bar, false);
    claim_action(&state, CaptureWindow::Bar, 1, Action::Download).unwrap();
    dismiss_offer(&state, CaptureWindow::Bar, 1);
    assert!(!has_offer(&state, CaptureWindow::Bar));
    assert!(!has_offer(&state, CaptureWindow::Card));
    pending_offer(&state, CaptureWindow::Bar, false);
    dismiss_offer(&state, CaptureWindow::Bar, 99); // stale id keeps the newer offer
    assert!(has_offer(&state, CaptureWindow::Bar));
  }

  #[test]
  fn a_different_link_in_the_other_window_stays() {
    let state = CaptureState::default();
    same_link_in_both_windows(&state);
    edit_slot(&state, CaptureWindow::Card, |slot| {
      slot.offer.url = "https://www.youtube.com/watch?v=other".into();
    });
    assert!(
      claim_action(&state, CaptureWindow::Bar, 2, Action::Download)
        .unwrap()
        .retired
        .is_none()
    );
    assert!(has_offer(&state, CaptureWindow::Card));
  }

  #[test]
  fn download_and_options_claim_immediately_without_preview_metadata() {
    for action in [Action::Download, Action::Options] {
      let state = CaptureState::default();
      pending_offer(&state, CaptureWindow::Card, true);
      let payload = claim_action(&state, CaptureWindow::Card, 1, action).unwrap();
      assert_eq!(payload.action, action.as_str());
      assert_eq!(payload.url, "https://www.youtube.com/watch?v=capture");
      assert!(payload.item.is_none());
    }
  }

  #[test]
  fn action_includes_an_already_finished_preview() {
    let state = CaptureState::default();
    pending_offer(&state, CaptureWindow::Card, true);
    let item = serde_json::json!({ "title": "Ready" });
    edit_slot(&state, CaptureWindow::Card, |slot| {
      slot.metadata = Some(Some(item.clone()));
    });
    assert_eq!(
      claim_action(&state, CaptureWindow::Card, 1, Action::Options)
        .unwrap()
        .item,
      Some(item)
    );
  }

  #[tokio::test(start_paused = true)]
  async fn preview_completion_covers_success_failure_replacement_and_timeout() {
    let state = CaptureState::default();
    pending_offer(&state, CaptureWindow::Card, true);
    // Tokio's paused clock advances through the timeout without waiting in real time.
    assert!(wait_for_item(&state, 1).await.is_none());
    let item = serde_json::json!({ "title": "Finished" });
    edit_slot(&state, CaptureWindow::Card, |slot| {
      slot.metadata = Some(Some(item.clone()));
    });
    assert_eq!(wait_for_item(&state, 1).await, Some(item));
    edit_slot(&state, CaptureWindow::Card, |slot| {
      slot.metadata = Some(None)
    });
    assert!(wait_for_item(&state, 1).await.is_none());
    state.offers.lock().unwrap().retire(CaptureWindow::Card);
    assert!(wait_for_item(&state, 1).await.is_none());
  }

  /// A failed card offer (id 1) that was adopted and still has its read attached.
  fn failed_offer(state: &CaptureState) -> tokio::sync::oneshot::Receiver<()> {
    pending_offer(state, CaptureWindow::Card, true);
    edit_slot(state, CaptureWindow::Card, |slot| {
      slot.offer.failed = true;
      slot.offer.reason = Some(FailReason::Network);
      slot.metadata = Some(None);
    });
    claim_action(state, CaptureWindow::Card, 1, Action::Adopt).unwrap();
    reading(state, CaptureWindow::Card, 1)
  }

  #[tokio::test]
  async fn report_retires_the_offer_with_its_claim_and_read() {
    let state = CaptureState::default();
    let stopped = failed_offer(&state);
    {
      let mut offers = state.offers.lock().unwrap();
      assert!(
        offers.take_report(CaptureWindow::Card, 2).is_none(),
        "stale id"
      );
      let report = offers.take_report(CaptureWindow::Card, 1).unwrap();
      assert_eq!(
        (report.url.as_str(), report.reason),
        (
          "https://www.youtube.com/watch?v=capture",
          FailReason::Network
        )
      );
      assert!(offers.slot(CaptureWindow::Card).is_none());
      assert!(offers.take_report(CaptureWindow::Card, 1).is_none(), "once");
    }
    assert_read_stopped(stopped).await;
    // Only a failed read can be reported.
    pending_offer(&state, CaptureWindow::Card, false);
    assert!(state
      .offers
      .lock()
      .unwrap()
      .take_report(CaptureWindow::Card, 1)
      .is_none());
    assert!(has_offer(&state, CaptureWindow::Card));
  }

  #[tokio::test]
  async fn retry_reads_the_failed_link_as_a_fresh_offer_that_retires_the_old_one() {
    let state = CaptureState::default();
    let stopped = failed_offer(&state);
    {
      let mut offers = state.offers.lock().unwrap();
      let url = offers.retry_url(CaptureWindow::Card).unwrap();
      // What `offer()` does with it: a fresh id in the same window.
      offers.replace(
        CaptureWindow::Card,
        Offer {
          id: 2,
          url,
          ..Offer::default()
        },
      );
      let slot = offers.slot(CaptureWindow::Card).unwrap();
      assert_eq!(slot.claim, Claim::None);
      assert!(slot.metadata.is_none() && slot.job.is_none());
      assert_eq!(offers.item(1), ItemState::Gone);
      assert!(
        offers.retry_url(CaptureWindow::Card).is_none(),
        "not failed"
      );
    }
    assert_read_stopped(stopped).await;
  }

  #[tokio::test]
  async fn a_read_for_a_replaced_offer_is_stopped_not_attached() {
    let state = CaptureState::default();
    pending_offer(&state, CaptureWindow::Bar, false);
    let stale = reading(&state, CaptureWindow::Bar, 7);
    assert!(state
      .offers
      .lock()
      .unwrap()
      .slot(CaptureWindow::Bar)
      .unwrap()
      .job
      .is_none());
    assert_read_stopped(stale).await;
  }
}
