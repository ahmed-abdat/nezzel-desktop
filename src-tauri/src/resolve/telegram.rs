//! Port of Android's Telegram public-widget reader. Only this message's own media is eligible.
use super::{dom::Document, http, post, Resolution, Verdict};
use regex::Regex;
use reqwest::Url;
use std::sync::LazyLock;
pub(super) const MATCHER: crate::routing::Matcher = crate::routing::Matcher::reader(
  crate::routing::ReaderId::Telegram,
  "telegram-message",
  20,
  |raw| Identity::parse(raw).is_some(),
);

const CAP: usize = 1024 * 1024;
static PATH: LazyLock<Regex> = LazyLock::new(|| {
  Regex::new(r"^/(?:s/)?([A-Za-z][A-Za-z0-9_]{4,31})/(?:[1-9][0-9]{0,19}/)?([1-9][0-9]{0,19})/?$")
    .unwrap()
});
static BACKGROUND: LazyLock<Regex> = LazyLock::new(|| {
  Regex::new(r#"(?i)background-image\s*:\s*url\(\s*['"]?(https://[^\s'"()]+)['"]?\s*\)"#).unwrap()
});
static CDN: LazyLock<Regex> =
  LazyLock::new(|| Regex::new(r"^cdn[0-9]{1,5}\.(?:telesco\.pe|cdn-telegram\.org)$").unwrap());
static FILE: LazyLock<Regex> = LazyLock::new(|| {
  Regex::new(r"^/file/[A-Za-z0-9_-]{1,2048}={0,2}(?:\.(?:jpg|jpeg|png|webp|gif|avif))?$").unwrap()
});
pub(super) fn is_host(host: &str) -> bool {
  matches!(host, "t.me" | "telegram.me")
}
/// Private channel message links cannot be read by the anonymous public widget.
pub(crate) fn private_message(input: &str) -> bool {
  let Ok(url) = Url::parse(input) else {
    return false;
  };
  if !matches!(url.scheme(), "https" | "http")
    || !url.host_str().is_some_and(is_host)
    || !url.username().is_empty()
    || url.password().is_some()
    || url.port().is_some()
  {
    return false;
  }
  let parts: Vec<_> = url.path().trim_matches('/').split('/').collect();
  matches!(parts.as_slice(), ["c", ids @ ..] if matches!(ids.len(), 2 | 3) && ids.iter().all(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit())))
}

struct Identity {
  key: String,
  url: Url,
}
impl Identity {
  fn parse(input: &str) -> Option<Self> {
    let url = http::https_url(input).ok()?;
    if !is_host(url.host_str()?) {
      return None;
    }
    let c = PATH.captures(url.path())?;
    if [
      "share",
      "proxy",
      "socks",
      "joinchat",
      "addstickers",
      "addemoji",
      "login",
      "invoice",
    ]
    .contains(&c[1].to_ascii_lowercase().as_str())
    {
      return None;
    }
    let key = format!("{}/{}", &c[1], &c[2]);
    Some(Self {
      url: Url::parse(&format!("https://t.me/{key}?embed=1&mode=tme")).ok()?,
      key,
    })
  }
}
fn asset(raw: &str) -> Option<String> {
  let url = http::https_url(raw).ok()?;
  (CDN.is_match(url.host_str()?) && FILE.is_match(url.path()) && url.query().is_none())
    .then(|| url.into())
}
pub(super) async fn read<T: http::Transport>(input: &str, transport: &T) -> Verdict {
  let Some(identity) = Identity::parse(input) else {
    return Verdict::NoMatch;
  };
  // Widget endpoints do not redirect. Private/error/login pages never provide media.
  let Ok(response) = transport.get(&identity.url, CAP).await else {
    return Verdict::Unknown;
  };
  if response.status != 200 || response.body.len() > CAP {
    return Verdict::Unknown;
  }
  let Ok(body) = std::str::from_utf8(&response.body) else {
    return Verdict::Unknown;
  };
  let Some(doc) = Document::parse(body) else {
    return Verdict::Unknown;
  };
  if doc.0.iter().any(|n| n.class("tgme_widget_message_error")) {
    return Verdict::Unknown;
  }
  let messages: Vec<_> = doc
    .0
    .iter()
    .enumerate()
    .filter(|(i, n)| {
      n.class("tgme_widget_message")
        && n.attr("data-post").eq_ignore_ascii_case(&identity.key)
        && doc.visible(*i)
    })
    .map(|(i, _)| i)
    .collect();
  if messages.len() != 1 {
    return Verdict::Unknown;
  }
  let message = messages[0];
  let own = |i| {
    doc.visible(i)
      && doc.closest(i, |n| n.class("tgme_widget_message")) == Some(message)
      && doc
        .closest(i, |n| {
          [
            "tgme_widget_message_link_preview",
            "tgme_widget_message_reply",
            "tgme_widget_message_reply_to",
          ]
          .iter()
          .any(|c| n.class(c))
        })
        .is_none()
  };
  let title = doc
    .0
    .iter()
    .enumerate()
    .find(|(i, n)| own(*i) && n.class("tgme_widget_message_text"))
    .and_then(|(i, _)| post::line(&doc.text(i)));
  let photos: Vec<_> = doc
    .0
    .iter()
    .enumerate()
    .filter(|(i, n)| own(*i) && n.tag.name == "a" && n.class("tgme_widget_message_photo_wrap"))
    .collect();
  let mut urls = Vec::new();
  for (_, photo) in photos.iter().take(20) {
    if let Some(url) = BACKGROUND
      .captures(photo.attr("style"))
      .and_then(|c| asset(&c[1]))
    {
      if !urls.contains(&url) {
        urls.push(url);
      }
    }
  }
  if !urls.is_empty() {
    let items = urls
      .into_iter()
      .map(|url| post::image(url, title.clone()))
      .collect();
    return post::attachments(
      transport,
      input,
      &identity.key,
      "Telegram",
      title,
      None,
      items,
    )
    .await;
  }
  // Android hands playable video posts back to yt-dlp. It knows Telegram's player protocol;
  // thumbnails, link previews and the "Media is too big" shell are never attachments.
  let video = doc.0.iter().enumerate().any(|(i, n)| {
    own(i)
      && n.attr("class").split_ascii_whitespace().any(|name| {
        ["video", "roundvideo", "document", "audio", "voice"]
          .iter()
          .any(|kind| {
            name == format!("tgme_widget_message_{kind}")
              || name.starts_with(&format!("tgme_widget_message_{kind}_"))
          })
      })
  });
  let unavailable = doc.0.iter().enumerate().any(|(i, n)| {
    own(i) && n.class("tgme_widget_message_video_player") && n.class("not_supported")
  }) && !doc
    .0
    .iter()
    .enumerate()
    .any(|(i, n)| own(i) && n.tag.name == "video" && !n.attr("src").is_empty());
  if video && !unavailable {
    Verdict::Verified(Resolution::Passthrough(format!(
      "https://t.me/{}",
      identity.key
    )))
  } else {
    Verdict::Unknown
  }
}
