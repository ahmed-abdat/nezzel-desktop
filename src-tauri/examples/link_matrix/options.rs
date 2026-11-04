use std::{error::Error, path::PathBuf};

pub(crate) struct Options {
  pub inputs: Vec<PathBuf>,
  pub out: PathBuf,
  pub only: Option<Vec<String>>,
  pub jobs: usize,
  pub login: Option<String>,
}

impl Options {
  pub fn parse(
    args: impl IntoIterator<Item = String>,
    ci: Option<&str>,
  ) -> Result<Self, Box<dyn Error>> {
    let mut args = args.into_iter();
    let mut inputs = Vec::new();
    let mut out = None;
    let mut only = None;
    let mut jobs = 2;
    let mut login = None;
    while let Some(arg) = args.next() {
      let value = args
        .next()
        .filter(|s| !s.starts_with("--"))
        .ok_or("flag requires a value")?;
      match arg.as_str() {
        "--links" => inputs.push(PathBuf::from(value)),
        "--out" => out = Some(PathBuf::from(value)),
        "--only" => {
          let sites: Vec<_> = value.split(',').map(str::trim).map(str::to_owned).collect();
          if sites.iter().any(String::is_empty) {
            return Err("--only requires nonempty comma-separated sites".into());
          }
          only = Some(sites);
        }
        "--jobs" => jobs = value.parse()?,
        "--login" => {
          let (browser, profile) = value
            .split_once(':')
            .map_or((value.as_str(), None), |(b, p)| (b, Some(p)));
          if !matches!(
            browser,
            "firefox"
              | "chrome"
              | "chromium"
              | "edge"
              | "brave"
              | "opera"
              | "vivaldi"
              | "safari"
              | "whale"
          ) || profile.is_some_and(|p| p.trim().is_empty())
            || value.chars().any(char::is_control)
          {
            return Err("invalid --login; use browser[:profile] with a supported browser".into());
          }
          login = Some(value);
        }
        _ => return Err("unknown argument (use --help)".into()),
      }
    }
    if login.is_some() && ci == Some("1") {
      return Err("--login is local-only and cannot run with CI=1".into());
    }
    if inputs.is_empty() || !(1..=2).contains(&jobs) {
      return Err("supply --links and use --jobs 1 or 2".into());
    }
    Ok(Self {
      inputs,
      out: out.ok_or("missing --out")?,
      only,
      jobs,
      login,
    })
  }

  pub fn includes(&self, site: &str) -> bool {
    self
      .only
      .as_ref()
      .is_none_or(|sites| sites.iter().any(|s| s == site))
  }
}

#[derive(Clone, Default, Debug, PartialEq)]
pub(crate) struct AttemptsColumns {
  pub stages: String,
  pub winner: String,
  pub final_code: String,
}

impl AttemptsColumns {
  pub fn from_events(events: &[(String, serde_json::Value)]) -> Self {
    let mut columns = Self::default();
    let mut stages = Vec::new();
    for (_, event) in events.iter().filter(|(kind, _)| kind == "media_attempts") {
      let Some(attempts) = event["attempts"].as_array() else {
        continue;
      };
      for attempt in attempts {
        stages.push(format!(
          "{}/{}/{}/{}/{}",
          attempt["stage"].as_str().unwrap_or("-"),
          if attempt["login"].as_bool().unwrap_or(false) {
            "yes"
          } else {
            "no"
          },
          attempt["outcome"].as_str().unwrap_or("-"),
          attempt["code"].as_str().unwrap_or("-"),
          attempt["ms"].as_u64().unwrap_or_default()
        ));
      }
      columns.winner = event["winner"]
        .as_u64()
        .and_then(|i| usize::try_from(i).ok())
        .and_then(|i| attempts.get(i))
        .and_then(|a| a["stage"].as_str())
        .unwrap_or_default()
        .into();
      columns.final_code = event["finalCode"].as_str().unwrap_or_default().into();
    }
    columns.stages = stages.join(" > ");
    columns
  }
}

pub(crate) fn prepare_evidence(evidence: &mut serde_json::Value, login: bool) {
  if !login {
    return;
  }
  if let Some(fields) = evidence.as_object_mut() {
    fields.remove("gallery_stderr");
    fields.remove("ytdlp_log");
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  fn options(extra: &[&str], ci: Option<&str>) -> Result<Options, Box<dyn Error>> {
    Options::parse(
      ["--links", "rows.tsv", "--out", "results"]
        .into_iter()
        .chain(extra.iter().copied())
        .map(str::to_owned),
      ci,
    )
  }

  #[test]
  fn parses_browser_and_optional_profile_without_changing_the_spec() {
    for spec in [
      "firefox",
      "firefox:/tmp/Profile 1",
      "chrome:Default",
      "edge:C:\\Profiles\\One",
    ] {
      let parsed = options(&["--login", spec], None).unwrap();
      assert_eq!(parsed.login.as_deref(), Some(spec));
    }
  }

  #[test]
  fn rejects_invalid_login_specs_without_echoing_profile_material() {
    for spec in [
      "",
      "none",
      "automatic",
      "unknown:/secret",
      "firefox:",
      ":/secret",
      "firefox:/secret\nline",
    ] {
      let error = options(&["--login", spec], None).err().unwrap().to_string();
      assert!(error.contains("login"));
      assert!(!error.contains("/secret"));
    }
  }

  #[test]
  fn login_is_refused_in_ci_but_anonymous_runs_are_allowed() {
    assert!(options(&["--login", "firefox"], Some("1")).is_err());
    assert!(options(&[], Some("1")).is_ok());
    assert!(options(&["--login", "firefox"], Some("0")).is_ok());
  }

  #[test]
  fn only_selects_exact_sites_from_comma_separated_list() {
    let parsed = options(&["--only", "reddit, linkedin", "--jobs", "1"], None).unwrap();
    assert!(parsed.includes("reddit"));
    assert!(parsed.includes("linkedin"));
    assert!(!parsed.includes("reddit.com"));
    assert!(!parsed.includes("youtube"));
    assert_eq!(parsed.jobs, 1);
    assert_eq!(parsed.inputs, [PathBuf::from("rows.tsv")]);
    assert_eq!(parsed.out, PathBuf::from("results"));
    let anonymous = options(&[], None).unwrap();
    assert!(anonymous.includes("any-site"));
    assert!(anonymous.only.is_none());
    assert!(anonymous.login.is_none());
  }

  #[test]
  fn rejects_empty_site_lists_and_missing_flag_values() {
    for args in [
      vec!["--only", ""],
      vec!["--only", "reddit,"],
      vec!["--only", ",reddit"],
      vec!["--only"],
      vec!["--login"],
      vec!["--login", "--jobs", "1"],
    ] {
      assert!(options(&args, None).is_err(), "{args:?}");
    }
  }

  #[test]
  fn attempts_preserve_order_and_use_the_event_winner_and_final_code() {
    let events = vec![(
      "media_attempts".into(),
      json!({
        "attempts": [
          {"stage":"ytdlp","login":false,"outcome":"login","code":"signInRequired","ms":12},
          {"stage":"gallery","login":true,"outcome":"unsupported","code":"urlNotSupported","ms":34}
        ], "winner":null,"finalCode":"signInRequired"
      }),
    )];
    assert_eq!(
      AttemptsColumns::from_events(&events),
      AttemptsColumns {
        stages: "ytdlp/no/login/signInRequired/12 > gallery/yes/unsupported/urlNotSupported/34"
          .into(),
        winner: String::new(),
        final_code: "signInRequired".into(),
      }
    );
    let events = vec![(
      "media_attempts".into(),
      json!({
        "attempts": [{"stage":"files","login":false,"outcome":"noMatch","code":null,"ms":1},
          {"stage":"ytdlp","login":true,"outcome":"verified","code":null,"ms":2}],
        "winner":1,"finalCode":null
      }),
    )];
    let columns = AttemptsColumns::from_events(&events);
    assert_eq!(
      columns.stages,
      "files/no/noMatch/-/1 > ytdlp/yes/verified/-/2"
    );
    assert_eq!(columns.winner, "ytdlp");
    assert!(columns.final_code.is_empty());
    assert_eq!(
      AttemptsColumns::from_events(&[]),
      AttemptsColumns::default()
    );
  }
  #[test]
  fn login_evidence_drops_raw_engine_output_and_keeps_app_diagnostics() {
    let mut evidence = json!({
      "login":"yes",
      "gallery_stderr":["Cookie: token=secret; /private/profile/cookies.sqlite https://private.example/token"],
      "ytdlp_log":["/private/profile/cookies.sqlite https://private.example/token"],
      "events":[["media_diagnostic", {"code":"signInRequired", "message":"Sign in required [redacted path] [redacted URL]"}]]
    });
    prepare_evidence(&mut evidence, true);
    let saved = evidence.to_string();
    assert!(!saved.contains("/private/profile"));
    assert!(!saved.contains("https://private.example"));
    assert!(!saved.contains("token=secret"));
    assert!(saved.contains("signInRequired"));
    assert!(saved.contains("Sign in required"));
  }
}
