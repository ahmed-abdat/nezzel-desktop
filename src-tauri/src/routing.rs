//! Leaf routing vocabulary: stage identities and URL-only matchers. Imports nothing from the
//! crate, so the registry (extract), the readers (resolve) and files can all depend on it.
use reqwest::Url;

/// Confidence gates destructive verdicts so a generic miss cannot declare content deleted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Confidence {
  None,
  Low,
  Medium,
  High,
}

/// Stage identity avoids retaining a URL or engine stderr in an attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageId {
  Files,
  Reader(ReaderId),
  Ytdlp,
  Gallery,
  PageScan,
}

/// Reader identities remain a closed vocabulary rather than exposing site input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReaderId {
  Reddit,
  Facebook,
  Threads,
  Linkedin,
  Telegram,
  Snapchat,
}

#[derive(Clone, Copy)]
pub(crate) struct Matcher {
  pub stage: StageId,
  // The rule's label: the registry tests look matchers up by it.
  #[cfg_attr(not(test), allow(dead_code))]
  pub name: &'static str,
  pub priority: u8,
  pub confidence: Confidence,
  pub exclusive: bool,
  pattern: Pattern,
}

#[derive(Clone, Copy)]
pub(crate) enum Pattern {
  Any,
  HostPath {
    hosts: &'static [&'static str],
    subdomains: bool,
    path: Path,
  },
  Identity(fn(&str) -> bool),
  UrlOnly(fn(&Url) -> bool),
}

#[derive(Clone, Copy)]
pub(crate) enum Path {
  Any,
  Prefixes(&'static [&'static str]),
  Contains {
    marker: &'static str,
    exclude: Option<&'static str>,
  },
}

impl Path {
  fn matches(self, path: &str) -> bool {
    match self {
      Self::Any => true,
      Self::Prefixes(prefixes) => prefixes.iter().any(|prefix| {
        path
          .strip_prefix(prefix)
          .is_some_and(|suffix| !suffix.is_empty())
      }),
      Self::Contains { marker, exclude } => {
        path
          .split_once(marker)
          .is_some_and(|(_, suffix)| !suffix.is_empty())
          && !exclude.is_some_and(|excluded| path.contains(excluded))
      }
    }
  }
}

impl Matcher {
  pub(crate) const fn new(
    stage: StageId,
    name: &'static str,
    priority: u8,
    confidence: Confidence,
    pattern: Pattern,
  ) -> Self {
    Self {
      stage,
      name,
      priority,
      confidence,
      exclusive: false,
      pattern,
    }
  }

  pub(crate) const fn reader(
    reader: ReaderId,
    name: &'static str,
    priority: u8,
    accepts: fn(&str) -> bool,
  ) -> Self {
    Self::new(
      StageId::Reader(reader),
      name,
      priority,
      Confidence::High,
      Pattern::Identity(accepts),
    )
  }

  /// An exclusive route keeps every other stage away from the URL.
  pub(crate) const fn exclusive(self) -> Self {
    Self {
      exclusive: true,
      ..self
    }
  }

  pub(crate) fn matches(&self, url: &Url) -> bool {
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
      return false;
    }
    match self.pattern {
      Pattern::Any => true,
      Pattern::Identity(accepts) => accepts(url.as_str()),
      Pattern::UrlOnly(accepts) => accepts(url),
      Pattern::HostPath {
        hosts,
        subdomains,
        path,
      } => {
        url.host_str().is_some_and(|host| {
          hosts.iter().any(|domain| {
            host == *domain
              || subdomains
                && host
                  .strip_suffix(domain)
                  .is_some_and(|prefix| prefix.ends_with('.'))
          })
        }) && path.matches(url.path())
      }
    }
  }
}

pub(crate) const fn host_path(
  hosts: &'static [&'static str],
  subdomains: bool,
  path: Path,
) -> Pattern {
  Pattern::HostPath {
    hosts,
    subdomains,
    path,
  }
}

/// The matching rules, keeping the highest (confidence, priority) rule per stage in first-match
/// order. Registration order breaks ties.
pub(crate) fn best_per_stage<'a>(
  matchers: impl IntoIterator<Item = &'a Matcher>,
  url: &Url,
) -> Vec<&'a Matcher> {
  let mut best: Vec<&Matcher> = Vec::new();
  for matcher in matchers.into_iter().filter(|matcher| matcher.matches(url)) {
    if let Some(existing) = best
      .iter_mut()
      .find(|existing| existing.stage == matcher.stage)
    {
      if (matcher.confidence, matcher.priority) > (existing.confidence, existing.priority) {
        *existing = matcher;
      }
    } else {
      best.push(matcher);
    }
  }
  best
}
