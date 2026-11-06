//! Android's LinkedIn attachment association: JSON-LD and an explicitly owned image grid must agree.
use super::{
  dom::{Document, Node},
  http, post, Resolution, Verdict,
};
use regex::Regex;
use reqwest::Url;
use serde::{de, Deserialize};
use serde_json::Value;
use std::{
  collections::{BTreeMap, BTreeSet},
  sync::LazyLock,
};
pub(super) const MATCHER: crate::routing::Matcher = crate::routing::Matcher::reader(
  crate::routing::ReaderId::Linkedin,
  "linkedin-post",
  20,
  |raw| Identity::parse(raw).is_some(),
);

const CAP: usize = 6 * 1024 * 1024;
static POST: LazyLock<Regex> =
  LazyLock::new(|| Regex::new(r"^/posts/([A-Za-z0-9_%_-]+)/?$").unwrap());
static UPDATE: LazyLock<Regex> = LazyLock::new(|| {
  Regex::new(r"^(?:/embed)?/feed/update/urn:li:(activity|share|ugcPost):([0-9]+)/?$").unwrap()
});
static TYPED: LazyLock<Regex> =
  LazyLock::new(|| Regex::new(r"(?:^|[_-])(activity|share|ugcPost)-([0-9]+)(?:-|$)").unwrap());
static SHORT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^/p/[A-Za-z0-9_-]+/?$").unwrap());

pub(super) fn is_host(host: &str) -> bool {
  host == "lnkd.in"
    || host == "linkedin.com"
    || host == "www.linkedin.com"
    || host
      .strip_suffix(".linkedin.com")
      .is_some_and(|s| s.len() == 2 && s.bytes().all(|b| b.is_ascii_lowercase()))
}
#[derive(Clone)]
struct Identity {
  url: Url,
  slug: Option<String>,
  ids: BTreeMap<String, String>,
  short: bool,
}
impl Identity {
  fn parse(input: &str) -> Option<Self> {
    let mut url = http::https_url(input).ok()?;
    let host = url.host_str()?;
    if !is_host(host) {
      return None;
    }
    let short = host == "lnkd.in";
    let mut slug = None;
    let mut ids = BTreeMap::new();
    if short {
      if !SHORT.is_match(url.path()) {
        return None;
      }
    } else {
      let path = url.path().replace("%3A", ":").replace("%3a", ":");
      if let Some(c) = UPDATE.captures(&path) {
        ids.insert(c[1].to_owned(), c[2].to_owned());
        url.set_path(&format!("/feed/update/urn:li:{}:{}/", &c[1], &c[2]));
      } else {
        let c = POST.captures(url.path())?;
        let value = c[1].to_owned();
        // Percent escapes are permitted for non-Latin slugs, never path/control delimiters.
        let mut bytes = value.bytes();
        while let Some(b) = bytes.next() {
          if b == b'%' {
            let a = bytes.next()?;
            let b = bytes.next()?;
            let n = u8::from_str_radix(std::str::from_utf8(&[a, b]).ok()?, 16).ok()?;
            if n < 32 || [b'#', b'%', b'.', b'/', b'?', b'\\', 127].contains(&n) {
              return None;
            }
          }
        }
        let typed: Vec<_> = TYPED.captures_iter(&value).collect();
        if typed.len() == 1 {
          ids.insert(typed[0][1].to_owned(), typed[0][2].to_owned());
        }
        url.set_path(&format!("/posts/{value}/"));
        slug = Some(value);
      }
      url.set_host(Some("www.linkedin.com")).ok()?;
    }
    url.set_query(None);
    url.set_fragment(None);
    Some(Self {
      url,
      slug,
      ids,
      short,
    })
  }
  fn same(&self, other: &Self) -> bool {
    if self.short || other.short {
      return false;
    }
    let shared: Vec<_> = self
      .ids
      .keys()
      .filter(|k| other.ids.contains_key(*k))
      .collect();
    if shared.is_empty() {
      self.slug.is_some() && self.slug == other.slug
    } else {
      shared.iter().all(|k| self.ids.get(*k) == other.ids.get(*k))
    }
  }
  fn activity(&self, urn: &str) -> bool {
    urn
      .strip_prefix("urn:li:activity:")
      .is_some_and(|id| self.ids.get("activity").is_some_and(|v| v == id))
  }
  fn attributed(&self, urn: &str) -> bool {
    ["share", "ugcPost"].into_iter().any(|kind| {
      urn
        .strip_prefix(&format!("urn:li:{kind}:"))
        .is_some_and(|id| self.ids.get(kind).is_some_and(|v| v == id))
    })
  }
}

pub(super) async fn read<T: http::Transport>(input: &str, transport: &T) -> Verdict {
  let Some(mut identity) = Identity::parse(input) else {
    return Verdict::NoMatch;
  };
  let mut seen = BTreeSet::new();
  for hop in 0..=3 {
    if !seen.insert(identity.url.to_string()) {
      return Verdict::Unknown;
    }
    let Ok(response) = transport.get(&identity.url, CAP).await else {
      return Verdict::Unknown;
    };
    if (300..400).contains(&response.status) {
      if hop == 3 {
        return Verdict::Unknown;
      }
      let Some(target) = response.location.and_then(|v| identity.url.join(&v).ok()) else {
        return Verdict::Unknown;
      };
      // Android admits only the www post host as a redirect target, not login/regional/other sites.
      if target.host_str() != Some("www.linkedin.com") {
        return Verdict::Unknown;
      }
      let Some(next) = Identity::parse(target.as_str()) else {
        return Verdict::Unknown;
      };
      if next.short || (!identity.short && !identity.same(&next)) {
        return Verdict::Unknown;
      }
      identity = next;
      continue;
    }
    if identity.short || response.status != 200 || response.body.len() > CAP {
      return Verdict::Unknown;
    }
    let Ok(body) = std::str::from_utf8(&response.body) else {
      return Verdict::Unknown;
    };
    if let Some(parsed) = parse(body, &identity) {
      let title = parsed.title;
      let items = parsed
        .urls
        .into_iter()
        .map(|url| post::image(url, title.clone()))
        .collect();
      return post::attachments(
        transport,
        identity.url.as_str(),
        identity.url.path(),
        "LinkedIn",
        title,
        parsed.author,
        items,
      )
      .await;
    }
    // Video/document/inconclusive pages keep the native extractor. In particular lnkd.in must
    // reach yt-dlp as the landed post URL; never feed a video poster through the image path.
    return Verdict::Verified(Resolution::Passthrough(identity.url.into()));
  }
  Verdict::Unknown
}

struct Parsed {
  urls: Vec<String>,
  title: Option<String>,
  author: Option<String>,
}
fn owner(doc: &Document<'_>, i: usize) -> Option<usize> {
  doc
    .ancestors(i)
    .find(|i| doc.0[*i].has("data-activity-urn") || doc.0[*i].has("data-post-url"))
}
fn owned(node: &Node<'_>, identity: &Identity) -> bool {
  (!node.has("data-activity-urn") || identity.activity(node.attr("data-activity-urn")))
    && (!node.has("data-post-url")
      || Identity::parse(node.attr("data-post-url")).is_some_and(|i| i.same(identity)))
}
fn parse(body: &str, resolved: &Identity) -> Option<Parsed> {
  if body.len() > CAP {
    return None;
  }
  let doc = Document::parse(body)?;
  let mut identity = resolved.clone();
  let mut bridge = None;
  if !identity.ids.contains_key("activity") {
    let claims: Vec<_> = doc
      .0
      .iter()
      .enumerate()
      .filter(|(i, n)| doc.visible(*i) && identity.attributed(n.attr("data-attributed-urn")))
      .collect();
    if !claims.is_empty() {
      if claims.len() != 1 {
        return None;
      }
      let (i, node) = claims[0];
      let activity = node
        .attr("data-activity-urn")
        .strip_prefix("urn:li:activity:")?;
      if activity.is_empty() || !activity.bytes().all(|b| b.is_ascii_digit()) {
        return None;
      }
      identity.ids.insert("activity".into(), activity.into());
      if (node.has("data-featured-activity-urn")
        && !identity.activity(node.attr("data-featured-activity-urn")))
        || !owned(node, &identity)
      {
        return None;
      }
      bridge = Some(i);
    }
  }
  let canonical: BTreeSet<_> = doc
    .0
    .iter()
    .filter(|n| n.tag.name == "link" && n.attr("rel").eq_ignore_ascii_case("canonical"))
    .map(|n| n.attr("href"))
    .collect();
  if canonical.len() > 1
    || canonical
      .iter()
      .any(|u| !Identity::parse(u).is_some_and(|i| i.same(&identity)))
  {
    return None;
  }
  let scripts: Vec<_> = doc
    .0
    .iter()
    .filter(|n| {
      n.tag.name == "script"
        && n
          .attr("type")
          .trim()
          .eq_ignore_ascii_case("application/ld+json")
    })
    .collect();
  if scripts.len() > 16 {
    return None;
  }
  let mut posts = Vec::new();
  for script in scripts {
    let text = script.tag.text;
    if !json_budget(text) {
      return None;
    }
    let root = match serde_json::from_str::<Unique>(text) {
      Ok(Unique(root)) => root,
      Err(error) if error.is_data() => return None,
      Err(_) => continue,
    };
    let nodes = if let Some(array) = root.as_array() {
      array.clone()
    } else {
      let mut nodes = vec![root.clone()];
      nodes.extend(list(&root["@graph"]).into_iter().cloned());
      nodes
    };
    for node in nodes {
      if !list(&node["@type"]).iter().any(|t| {
        matches!(
          t.as_str(),
          Some("SocialMediaPosting" | "DiscussionForumPosting")
        )
      }) {
        continue;
      }
      let named: Vec<_> = [node["url"].as_str(), node["@id"].as_str()]
        .into_iter()
        .flatten()
        .collect();
      if named.is_empty() {
        continue;
      }
      let Some(first) = Identity::parse(named[0]) else {
        continue;
      };
      if named
        .iter()
        .any(|u| !Identity::parse(u).is_some_and(|i| i.same(&first)))
      {
        return None;
      }
      if !first.same(&identity) {
        continue;
      }
      if let Some(urn) = node["identifier"]
        .as_str()
        .filter(|u| u.starts_with("urn:li:activity:"))
      {
        if identity.ids.contains_key("activity") && !identity.activity(urn) {
          return None;
        }
      }
      posts.push(node);
    }
  }
  if posts.len() != 1 {
    return None;
  }
  let post = &posts[0];
  let avatars: BTreeSet<_> = list(&post["author"])
    .into_iter()
    .flat_map(|a| image_urls(&a["image"]))
    .collect();
  let images = image_urls(&post["image"]);
  if images.len() > 64 {
    return None;
  }
  let mut evidence = Vec::new();
  for image in images {
    if !avatars.contains(&image) && asset(&image) && !evidence.contains(&image) {
      evidence.push(image);
    }
  }
  if evidence.is_empty() {
    return None;
  }
  let lists: Vec<_> = doc
    .0
    .iter()
    .enumerate()
    .filter(|(i, n)| {
      n.tag.name == "ul"
        && n.class("feed-images-content")
        && doc.visible(*i)
        && owner(&doc, *i).is_some_and(|o| owned(&doc.0[o], &identity))
    })
    .map(|(i, _)| i)
    .collect();
  if lists.len() != 1 {
    return None;
  }
  let grid = lists[0];
  if bridge.is_some() && owner(&doc, grid) != bridge {
    return None;
  }
  let items = &doc.0[grid].children;
  if items.len() > 64 {
    return None;
  }
  let mut urls = Vec::new();
  let mut more = None;
  for (index, i) in items.iter().enumerate() {
    let item = &doc.0[*i];
    if item.tag.name != "li" || !doc.own_text(*i).trim().is_empty() || !doc.visible(*i) {
      return None;
    }
    if item.tag.attrs.iter().any(|(k, v)| match k.as_str() {
      "class" => false,
      "data-test-id" => v != "feed-images-content__list-item",
      "data-position" => v != &index.to_string(),
      "style" => !v.trim().is_empty(),
      _ => true,
    }) {
      return None;
    }
    let badges = item
      .children
      .iter()
      .filter(|i| {
        doc.0[**i].tag.name == "button"
          && doc.0[**i].attr("data-test-id") == "public_post__c2pa-overlay"
      })
      .count();
    if badges > 1 {
      return None;
    }
    let children: Vec<_> = item
      .children
      .iter()
      .filter(|i| {
        !(doc.0[**i].tag.name == "button"
          && doc.0[**i].attr("data-test-id") == "public_post__c2pa-overlay")
      })
      .collect();
    if children.is_empty() || children.len() > 2 {
      return None;
    }
    let image = &doc.0[*children[0]];
    if image.tag.name != "img"
      || !doc.visible(*children[0])
      || image.tag.attrs.iter().any(|(k, v)| match k.as_str() {
        "class" | "alt" | "src" | "data-delayed-url" | "loading" | "width" | "height" => false,
        "data-ghost-url" => {
          !http::https_url(v).is_ok_and(|u| u.host_str() == Some("static.licdn.com"))
        }
        _ => true,
      })
    {
      return None;
    }
    let url = image.attr("data-delayed-url");
    let src = image.attr("src");
    if !evidence.iter().any(|u| u == url)
      || (!src.is_empty() && src != url && !src.starts_with("data:"))
    {
      return None;
    }
    if !urls.iter().any(|u| u == url) {
      urls.push(url.to_owned());
    }
    if children.len() == 2 {
      let overlay = &doc.0[*children[1]];
      if index + 1 != items.len()
        || overlay.tag.name != "div"
        || !overlay.children.is_empty()
        || overlay.attr("data-test-id") != "feed-images-content__overlay"
        || overlay
          .tag
          .attrs
          .keys()
          .any(|k| k != "class" && k != "data-test-id")
      {
        return None;
      }
      let n = overlay.text.trim().strip_prefix('+')?;
      if n.is_empty() || n.len() > 3 || !n.bytes().all(|b| b.is_ascii_digit()) {
        return None;
      }
      more = n.parse::<usize>().ok().filter(|n| *n > 0);
      more?;
    }
  }
  if let Some(count) = more {
    let hidden: Vec<_> = evidence.into_iter().filter(|u| !urls.contains(u)).collect();
    if hidden.len() == count {
      urls.extend(hidden);
    }
  }
  if urls.is_empty() {
    return None;
  }
  if urls.len() == 1 {
    let og: BTreeSet<_> = doc
      .0
      .iter()
      .filter(|n| n.tag.name == "meta" && n.attr("property") == "og:image")
      .map(|n| n.attr("content"))
      .collect();
    if og.len() != 1 || !og.contains(urls[0].as_str()) {
      return None;
    }
  }
  let title = post["headline"]
    .as_str()
    .and_then(post::line)
    .or_else(|| post["articleBody"].as_str().and_then(post::line));
  let author = list(&post["author"])
    .into_iter()
    .find_map(|a| a["name"].as_str().and_then(post::line));
  Some(Parsed {
    urls,
    title,
    author,
  })
}
fn asset(raw: &str) -> bool {
  http::https_url(raw).is_ok_and(|u| u.host_str() == Some("media.licdn.com"))
}
fn list(value: &Value) -> Vec<&Value> {
  match value {
    Value::Null => vec![],
    Value::Array(a) => a.iter().collect(),
    _ => vec![value],
  }
}
fn image_urls(value: &Value) -> Vec<String> {
  list(value)
    .into_iter()
    .filter_map(|v| {
      v.as_str().or_else(|| {
        v.get("contentUrl")
          .or_else(|| v.get("url"))
          .and_then(Value::as_str)
      })
    })
    .map(str::to_owned)
    .collect()
}
fn json_budget(text: &str) -> bool {
  if text.len() > 256 * 1024 {
    return false;
  }
  let (mut depth, mut tokens, mut string_len) = (0usize, 0usize, 0usize);
  let (mut quoted, mut escaped) = (false, false);
  for b in text.bytes() {
    if quoted {
      string_len += 1;
      if string_len > 64 * 1024 {
        return false;
      }
      if escaped {
        escaped = false;
      } else if b == b'\\' {
        escaped = true;
      } else if b == b'"' {
        quoted = false;
      }
    } else {
      match b {
        b'"' => {
          quoted = true;
          string_len = 0;
          tokens += 1;
        }
        b'{' | b'[' => {
          depth += 1;
          tokens += 1;
          if depth > 32 {
            return false;
          }
        }
        b'}' | b']' => {
          depth = depth.saturating_sub(1);
          tokens += 1;
        }
        b',' | b':' => tokens += 1,
        _ => {}
      }
      if tokens > 20_000 {
        return false;
      }
    }
  }
  true
}
// Strict duplicate detection, as on Android. An ambiguous object never supplies post identity.
struct Unique(Value);
impl<'de> Deserialize<'de> for Unique {
  fn deserialize<D: de::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
    struct Visitor;
    impl<'de> de::Visitor<'de> for Visitor {
      type Value = Value;
      fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("JSON without duplicate keys")
      }
      fn visit_unit<E: de::Error>(self) -> Result<Value, E> {
        Ok(Value::Null)
      }
      fn visit_bool<E: de::Error>(self, v: bool) -> Result<Value, E> {
        Ok(v.into())
      }
      fn visit_i64<E: de::Error>(self, v: i64) -> Result<Value, E> {
        Ok(v.into())
      }
      fn visit_u64<E: de::Error>(self, v: u64) -> Result<Value, E> {
        Ok(v.into())
      }
      fn visit_f64<E: de::Error>(self, v: f64) -> Result<Value, E> {
        serde_json::Number::from_f64(v)
          .map(Value::Number)
          .ok_or_else(|| E::custom("nonfinite number"))
      }
      fn visit_str<E: de::Error>(self, v: &str) -> Result<Value, E> {
        Ok(v.into())
      }
      fn visit_string<E: de::Error>(self, v: String) -> Result<Value, E> {
        Ok(v.into())
      }
      fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
        let mut values = Vec::new();
        while let Some(Unique(v)) = seq.next_element()? {
          values.push(v);
        }
        Ok(Value::Array(values))
      }
      fn visit_map<A: de::MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
        let mut values = serde_json::Map::new();
        while let Some((k, Unique(v))) = map.next_entry::<String, Unique>()? {
          if values.insert(k, v).is_some() {
            return Err(de::Error::custom("duplicate key"));
          }
        }
        Ok(Value::Object(values))
      }
    }
    d.deserialize_any(Visitor).map(Self)
  }
}
