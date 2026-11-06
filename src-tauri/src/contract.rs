//! Renders the closed vocabularies and numbers the webview shares with Rust into `src/generated/contract.ts`.
//! The values stay with the modules that use them; this file only lists and renders them. TS imports the
//! generated file and never the reverse. Regenerate with `cargo test -- --ignored write_contract_ts`.

use crate::capture::{self, Action};
use crate::platform;
use std::fmt::Write;

const HEADER: &str = "// Generated from src-tauri/src/contract.rs by `cargo test -- --ignored write_contract_ts`. Do not edit.\n";
const TS_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../src/generated/contract.ts");

/// `capture::Choice` on the wire (hand-rendered: the union is closed; `capture_choice_type_is_the_wire_shape` checks it).
const CAPTURE_CHOICE_TS: &str = "export type CaptureChoice = { kind: 'best' } | { kind: 'small' } | { kind: 'audio'; kbps?: number } | { kind: 'video'; height: number; fps?: number };\n";

struct Ts(String);

impl Ts {
  fn doc(&mut self, doc: &str) -> &mut Self {
    let _ = writeln!(self.0, "\n/** {doc} */");
    self
  }

  /// `export const NAME = ['a', 'b'] as const;` and its union type.
  fn words(&mut self, name: &str, ty: &str, words: impl IntoIterator<Item = &'static str>) {
    let list: Vec<String> = words.into_iter().map(|w| format!("'{w}'")).collect();
    let _ = writeln!(
      self.0,
      "export const {name} = [{}] as const;",
      list.join(", ")
    );
    let _ = writeln!(self.0, "export type {ty} = (typeof {name})[number];");
  }

  fn numbers(&mut self, name: &str, numbers: impl IntoIterator<Item = u64>) {
    let list: Vec<String> = numbers.into_iter().map(|n| n.to_string()).collect();
    let _ = writeln!(
      self.0,
      "export const {name} = [{}] as const;",
      list.join(", ")
    );
  }

  fn number(&mut self, name: &str, value: impl std::fmt::Display) {
    let _ = writeln!(self.0, "export const {name} = {value};");
  }

  fn text(&mut self, name: &str, value: &str) {
    let _ = writeln!(self.0, "export const {name} = '{value}';");
  }

  /// `export const NAME = { key: value } as const;` with values already in TS syntax.
  fn object(&mut self, name: &str, fields: &[(&str, String)]) {
    let fields: Vec<String> = fields.iter().map(|(k, v)| format!("{k}: {v}")).collect();
    let _ = writeln!(
      self.0,
      "export const {name} = {{ {} }} as const;",
      fields.join(", ")
    );
  }

  /// `export const SITES = [{ key, name, hosts, extractors }, …] as const;` from `sites.rs`.
  fn sites(&mut self) {
    let quoted = |words: &[&str]| {
      let words: Vec<String> = words.iter().map(|w| format!("'{w}'")).collect();
      format!("[{}]", words.join(", "))
    };
    let _ = writeln!(self.0, "export const SITES = [");
    for site in crate::sites::SITES {
      let hosts: Vec<&str> = site
        .domains
        .iter()
        .chain(site.short_links)
        .copied()
        .collect();
      let _ = writeln!(
        self.0,
        "  {{ key: '{}', name: '{}', hosts: {}, extractors: {} }},",
        site.key,
        site.name,
        quoted(&hosts),
        quoted(site.extractors)
      );
    }
    let _ = writeln!(self.0, "] as const;");
  }
}

fn words_of<T: Copy>(all: &[T], word: fn(T) -> &'static str) -> Vec<&'static str> {
  all.iter().map(|v| word(*v)).collect()
}

pub fn render_ts() -> String {
  let mut ts = Ts(HEADER.to_string());

  ts.doc("What a capture window asks for (`capture::Action`).");
  ts.words(
    "CAPTURE_ACTIONS",
    "CaptureAction",
    words_of(&Action::ALL, Action::as_str),
  );
  ts.doc("What a capture webview may send to `capture_action`.");
  ts.words(
    "CAPTURE_WINDOW_ACTIONS",
    "CaptureWindowAction",
    words_of(&Action::FROM_WINDOW, Action::as_str),
  );
  ts.doc("Actions that reach the main window's queue as a claim on an offer.");
  ts.words(
    "CAPTURE_CLAIMS",
    "CaptureClaim",
    words_of(&Action::CLAIMS, Action::as_str),
  );
  ts.doc("Why a capture read failed; each has `capture.fail.<reason>.title/.hint` (`preview::FailReason`).");
  ts.words(
    "FAIL_REASONS",
    "FailReason",
    words_of(&capture::preview::FailReason::ALL, |r| r.as_str()),
  );

  ts.doc("Native labels of the two capture windows.");
  ts.object(
    "CAPTURE_WINDOW",
    &[
      ("card", format!("'{}'", capture::CARD_LABEL)),
      ("bar", format!("'{}'", capture::BAR_LABEL)),
    ],
  );
  let _ = writeln!(
    ts.0,
    "export type CaptureWindow = (typeof CAPTURE_WINDOW)[keyof typeof CAPTURE_WINDOW];"
  );
  ts.doc("Native label of the close-to-tray note window.");
  ts.text("CLOSE_NOTE_WINDOW", crate::background::NOTE_LABEL);
  ts.doc(
    "Copied-link card size in logical pixels: closed, and the tallest with its format list open.",
  );
  ts.object(
    "CAPTURE_CARD_SIZE",
    &[
      ("width", capture::CARD_WIDTH.to_string()),
      ("height", capture::CARD_HEIGHT.to_string()),
      ("maxHeight", capture::CARD_MAX_HEIGHT.to_string()),
    ],
  );
  ts.doc("Quick bar size in logical pixels as built; the bar then fits its height to its content.");
  ts.object(
    "CAPTURE_BAR_SIZE",
    &[
      ("width", capture::BAR_WIDTH.to_string()),
      ("height", capture::BAR_HEIGHT.to_string()),
    ],
  );

  ts.doc("Card heights offered before the read and for links without their own format list.");
  ts.numbers("GENERIC_HEIGHTS", capture::GENERIC_HEIGHTS);
  ts.doc("Rounded frame rates from here up are their own (high frame rate) format row.");
  ts.number("HIGH_FPS_MIN", capture::preview::HIGH_FPS_MIN);
  ts.doc("The Smaller pick: the highest height at or under this.");
  ts.number("SMALL_MAX_HEIGHT", capture::preview::SMALL_MAX_HEIGHT);
  ts.doc("A quick pick or card format row (`capture::Choice`): what card rows offer and `capture_action` carries. Rust checks the numbers.");
  ts.0.push_str(CAPTURE_CHOICE_TS);

  ts.doc("How often the main window pulls the capture outbox (`capture_pull`), events or not.");
  ts.number("CAPTURE_PULL_MS", capture::PULL_MS);
  ts.doc("Session kinds the capture copy depends on (`platform::Session`).");
  ts.words(
    "CAPTURE_SESSIONS",
    "CaptureSession",
    words_of(&platform::Session::ALL, platform::Session::as_str),
  );
  ts.doc("Linux desktop families whose tray words the copy uses (`platform::Desktop`).");
  ts.words(
    "LINUX_DESKTOPS",
    "LinuxDesktop",
    words_of(&platform::Desktop::ALL, platform::Desktop::as_str),
  );
  ts.doc("Welcome \"Try it\" sample link.");
  ts.text("TRY_LINK", capture::TRY_LINK);
  ts.doc("Quick download shortcut, and what is registered when the default is taken.");
  ts.text(
    "DEFAULT_CAPTURE_SHORTCUT",
    crate::state::config_models::DEFAULT_CAPTURE_SHORTCUT,
  );
  ts.text(
    "FALLBACK_CAPTURE_SHORTCUT",
    crate::state::config_models::FALLBACK_CAPTURE_SHORTCUT,
  );

  ts.doc("Usage keys the web UI may count; Rust drops any other.");
  ts.words(
    "UI_USAGE_KEYS",
    "UiUsageKey",
    crate::usage::keys::UI_KEYS.iter().copied(),
  );
  ts.doc("Browsers a login can come from (yt-dlp names), in the order Settings lists them.");
  ts.words(
    "COOKIE_BROWSERS",
    "CookieBrowser",
    crate::runners::browsers::cookie_browsers(),
  );
  ts.doc("`auth.cookieBrowser`: find a signed-in profile when a site refuses.");
  ts.text(
    "COOKIE_BROWSER_AUTOMATIC",
    crate::runners::browsers::AUTOMATIC,
  );
  ts.doc("`auth.cookieBrowser`: never use browser logins.");
  ts.text("COOKIE_BROWSER_NEVER", crate::runners::browsers::NEVER);
  ts.doc("Login reason: every signed-in profile is behind Windows app-bound encryption.");
  ts.text(
    "LOGIN_APP_BOUND_REASON",
    crate::runners::login::APP_BOUND_REASON,
  );
  ts.doc("How long the engine keeps a login request.");
  ts.number("LOGIN_TTL_MS", crate::runners::login::LOGIN_TTL.as_millis());
  ts.doc("Error codes a login can fix.");
  ts.words(
    "LOGIN_CODES",
    "LoginCode",
    crate::extract::outcome::LOGIN_CODES,
  );
  ts.doc("Error codes the site decides: retrying the same link fails the same way.");
  ts.words(
    "PERMANENT_CODES",
    "PermanentCode",
    crate::extract::outcome::PERMANENT_CODES,
  );
  ts.doc("Subfolder of the download folder for file-share downloads.");
  ts.text("FILES_FOLDER", crate::files::runner::FILES_FOLDER);
  ts.doc("Languages written right to left (primary subtags).");
  ts.words("RTL_LANGS", "RtlLang", crate::i18n::RTL_LANGS);
  ts.doc("Privacy switches until the config has loaded (Rust `PrivacySettings::default`).");
  let privacy = crate::state::config_models::Config::default().privacy;
  ts.object(
    "PRIVACY_DEFAULTS",
    &[
      ("sendReports", privacy.send_reports.to_string()),
      ("sendUsage", privacy.send_usage.to_string()),
    ],
  );
  ts.doc("Why a report the user sent did not go (`diagnostics::send::SendError`); only `notDelivered` is worth a retry.");
  ts.words(
    "SEND_ERRORS",
    "SendError",
    words_of(
      &crate::diagnostics::send::SendError::ALL,
      crate::diagnostics::send::SendError::as_str,
    ),
  );
  ts.doc("Login reasons the engine gives itself; any other `LoginNeeded.reason` is the site's diagnostic code.");
  ts.words(
    "LOGIN_REASONS",
    "LoginReason",
    crate::runners::login::LOGIN_REASONS,
  );
  ts.doc("Row events the engine emits to the webview (`events.rs`).");
  ts.object(
    "ROW_EVENTS",
    &crate::events::ALL.map(|(key, name)| (key, format!("'{name}'"))),
  );
  ts.doc("Known sites (`sites.rs`): display name, hosts (the host or a subdomain), yt-dlp extractor keys.");
  ts.sites();
  ts.0
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn contract_ts_is_current() {
    assert!(
      include_str!("../../src/generated/contract.ts") == render_ts(),
      "src/generated/contract.ts is stale: run `cargo test -- --ignored write_contract_ts` in src-tauri"
    );
  }

  #[test]
  #[ignore = "writes src/generated/contract.ts"]
  fn write_contract_ts() {
    std::fs::write(TS_PATH, render_ts()).unwrap();
  }

  /// The rendered words are the ones the IPC payloads carry.
  #[test]
  fn rendered_words_are_the_wire_spelling() {
    let wire = |value: serde_json::Value| value.as_str().unwrap().to_string();
    for reason in capture::preview::FailReason::ALL {
      assert_eq!(wire(serde_json::to_value(reason).unwrap()), reason.as_str());
    }
    for session in platform::Session::ALL {
      assert_eq!(
        wire(serde_json::to_value(session).unwrap()),
        session.as_str()
      );
    }
    for desktop in platform::Desktop::ALL {
      assert_eq!(
        wire(serde_json::to_value(desktop).unwrap()),
        desktop.as_str()
      );
    }
  }

  /// Every `Choice` variant serializes to a member of the rendered union: its kind, then exactly its fields.
  #[test]
  fn capture_choice_type_is_the_wire_shape() {
    use capture::Choice;
    let members: Vec<&str> = CAPTURE_CHOICE_TS.split(" | ").collect();
    for choice in [
      Choice::Best,
      Choice::Small,
      Choice::Audio { kbps: Some(129) },
      Choice::Video {
        height: 1080,
        fps: Some(60),
      },
    ] {
      let wire = serde_json::to_value(choice).unwrap();
      let wire = wire.as_object().unwrap();
      let kind = wire["kind"].as_str().unwrap();
      let member = members
        .iter()
        .find(|m| m.contains(&format!("kind: '{kind}'")))
        .unwrap_or_else(|| panic!("no TS member for {kind}"));
      let fields = member.matches(':').count();
      assert_eq!(fields, wire.len(), "{member} vs {wire:?}");
      for key in wire.keys() {
        assert!(member.contains(&format!(" {key}")), "{member} lacks {key}");
      }
    }
  }

  /// capabilities/capture.json can't import the labels: it must list exactly the capture windows and the note.
  #[test]
  fn capture_capability_lists_the_capture_windows() {
    let capability: serde_json::Value =
      serde_json::from_str(include_str!("../capabilities/capture.json")).unwrap();
    let mut windows: Vec<&str> = capability["windows"]
      .as_array()
      .unwrap()
      .iter()
      .filter_map(|w| w.as_str())
      .collect();
    windows.sort_unstable();
    let mut expected = vec![
      capture::CARD_LABEL,
      capture::BAR_LABEL,
      crate::background::NOTE_LABEL,
    ];
    expected.sort_unstable();
    assert_eq!(windows, expected);
  }
}
