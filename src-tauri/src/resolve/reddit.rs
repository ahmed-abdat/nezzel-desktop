//! Logged-out Reddit HTML, bound to one post. Unknown pages retain yt-dlp/login fallback.
use super::{dom::Document, http, post, Resolution, Verdict};
use crate::models::parsed::{AttachmentKind, MediaAttachment};
use reqwest::Url;
use std::collections::HashSet;

pub(super) const MATCHER: crate::routing::Matcher = crate::routing::Matcher::reader(
  crate::routing::ReaderId::Reddit,
  "reddit-post",
  20,
  is_post_url,
);

const PAGE_CAP: usize = 4 * 1024 * 1024;

pub(crate) fn is_host(host: &str) -> bool {
  matches!(
    host,
    "reddit.com"
      | "www.reddit.com"
      | "old.reddit.com"
      | "new.reddit.com"
      | "sh.reddit.com"
      | "m.reddit.com"
      | "np.reddit.com"
      | "redd.it"
      | "v.redd.it"
  )
}

struct Identity {
  url: Url,
  post_id: Option<String>,
  video_id: Option<String>,
}
impl Identity {
  fn parse(raw: &str) -> Option<Self> {
    let input = http::https_url(raw).ok()?;
    let host = input.host_str()?;
    if !is_host(host) {
      return None;
    }
    let parts: Vec<_> = input.path().trim_matches('/').split('/').collect();
    let mut post_id = None;
    let mut video_id = None;
    let path = match (host, parts.as_slice()) {
      ("redd.it", [id]) if identifier(id, 16) => {
        post_id = Some((*id).to_owned());
        format!("/comments/{id}/")
      }
      ("v.redd.it", [id]) if identifier(id, 24) => {
        video_id = Some((*id).to_owned());
        format!("/video/{id}/")
      }
      ("redd.it" | "v.redd.it", _) => return None,
      (_, ["comments", id, rest @ ..]) if identifier(id, 16) && rest.len() <= 2 => {
        post_id = Some((*id).to_owned());
        format!("/comments/{id}/")
      }
      (_, [kind @ ("r" | "u" | "user"), name, "comments", id, rest @ ..])
        if owner(name) && identifier(id, 16) && rest.len() <= 2 =>
      {
        post_id = Some((*id).to_owned());
        // Slugless pages can be shells or login gates even when the full permalink is public.
        match rest.first().filter(|slug| {
          !slug.is_empty()
            && slug.len() <= 300
            && slug
              .bytes()
              .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'%'))
        }) {
          Some(slug) => format!("/{kind}/{name}/comments/{id}/{slug}/"),
          None => format!("/comments/{id}/"),
        }
      }
      (_, ["gallery", id]) if identifier(id, 16) => {
        post_id = Some((*id).to_owned());
        format!("/comments/{id}/")
      }
      (_, ["video", id]) if identifier(id, 24) => {
        video_id = Some((*id).to_owned());
        format!("/video/{id}/")
      }
      (_, [kind @ ("r" | "u" | "user"), name, "s", token])
        if owner(name)
          && !token.is_empty()
          && token.len() <= 32
          && token.bytes().all(|c| c.is_ascii_alphanumeric()) =>
      {
        format!("/{kind}/{name}/s/{token}/")
      }
      _ => return None,
    };
    Some(Self {
      url: Url::parse(&format!("https://www.reddit.com{path}")).ok()?,
      post_id,
      video_id,
    })
  }

  fn fallback_url(&self) -> String {
    self.post_id.as_ref().map_or_else(
      || self.url.to_string(),
      |id| format!("https://www.reddit.com/comments/{id}/"),
    )
  }
}
fn identifier(value: &str, cap: usize) -> bool {
  !value.is_empty()
    && value.len() <= cap
    && value
      .bytes()
      .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}
fn owner(value: &str) -> bool {
  !value.is_empty()
    && value.len() <= 64
    && value
      .bytes()
      .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
}
pub(crate) fn is_post_url(raw: &str) -> bool {
  Identity::parse(raw).is_some()
}

pub(super) async fn read<T: http::Transport>(raw: &str, transport: &T) -> Verdict {
  let Some(requested) = Identity::parse(raw) else {
    return Verdict::NoMatch;
  };
  let mut url = requested.url.clone();
  let mut fallback = requested.fallback_url();
  let mut bound_post = requested.post_id.clone();
  let mut seen = HashSet::new();
  let mut redirects = 0;
  let mut answered = false;
  loop {
    if !seen.insert(url.to_string()) {
      break;
    }
    let Ok(response) = transport.get(&url, PAGE_CAP).await else {
      break;
    };
    if [301, 302, 303, 307, 308].contains(&response.status) {
      if redirects >= 3 {
        break;
      }
      let Some(next) = response.location.as_deref().and_then(|v| url.join(v).ok()) else {
        break;
      };
      let Some(identity) = Identity::parse(next.as_str()) else {
        break;
      };
      if bound_post
        .as_ref()
        .is_some_and(|id| identity.post_id.as_ref() != Some(id))
        || next
          .host_str()
          .is_none_or(|h| h == "redd.it" || h == "v.redd.it")
      {
        break;
      }
      if let Some(id) = &identity.post_id {
        fallback = identity.fallback_url();
        bound_post = Some(id.clone());
      }
      url = next;
      url.set_query(None);
      redirects += 1;
      continue;
    }
    if response.status != 200 || response.body.len() > PAGE_CAP {
      break;
    }
    let Ok(body) = std::str::from_utf8(&response.body) else {
      break;
    };
    if !answered {
      if let Some(answer) = challenge(body, &url) {
        answered = true;
        url = answer;
        continue;
      }
    }
    let Some(identity) = Identity::parse(url.as_str()) else {
      break;
    };
    let Some(id) = identity.post_id else {
      break;
    };
    if let Some(result) = parse(body, &id, requested.video_id.as_deref()) {
      return match result {
        Post::Video {
          target,
          title,
          author,
          thumbnail,
        } => {
          let Ok(url) = http::https_url(&target) else {
            break;
          };
          if transport.admit_media(&url).await.is_err() {
            break;
          }
          if let Some(raw) = &thumbnail {
            let Ok(url) = http::https_url(raw) else {
              break;
            };
            if transport.admit_media(&url).await.is_err() {
              break;
            }
          }
          Verdict::Verified(Resolution::PostMedia {
            url: target,
            title,
            thumbnail,
            uploader: author,
          })
        }
        Post::Items {
          title,
          author,
          items,
        } => {
          // Refresh through the page that actually exposed the media; challenge answers expire.
          let mut source = url.clone();
          source.set_query(None);
          post::attachments(
            transport,
            source.as_str(),
            &id,
            "Reddit",
            title,
            author,
            items,
          )
          .await
        }
      };
    }
    break;
  }
  Verdict::Verified(Resolution::Passthrough(fallback))
}

/// The one known check doubles a hex literal; no script is ever executed and no cookies are used.
fn challenge(body: &str, page: &Url) -> Option<Url> {
  if body.len() > 32 * 1024 {
    return None;
  }
  let document = Document::parse(body)?;
  let pattern = regex::Regex::new(
    r#"await\s*\(\s*async\s+(\w+)\s*=>\s*(\w+)\s*\+\s*(\w+)\s*\)\s*\(\s*"([0-9a-f]{8,256})"\s*\)"#,
  )
  .ok()?;
  let mut seeds = document
    .0
    .iter()
    .filter(|n| n.tag.name == "script")
    .flat_map(|n| pattern.captures_iter(n.tag.text));
  let seed = seeds.next()?;
  if seeds.next().is_some() || seed[1] != seed[2] || seed[1] != seed[3] {
    return None;
  }
  let forms: Vec<_> = document
    .0
    .iter()
    .enumerate()
    .filter(|(_, n)| n.tag.name == "form")
    .collect();
  let [(form_index, form)] = forms.as_slice() else {
    return None;
  };
  if !form.attr("method").eq_ignore_ascii_case("get") {
    return None;
  }
  let action = http::https_url(page.join(form.attr("action")).ok()?.as_str()).ok()?;
  if action.host_str() != page.host_str()
    || action.path() != page.path()
    || action.query().is_some()
  {
    return None;
  }
  let inputs: Vec<_> = document
    .0
    .iter()
    .enumerate()
    .filter(|(i, n)| n.tag.name == "input" && document.ancestors(*i).any(|p| p == *form_index))
    .map(|(_, n)| n)
    .collect();
  if inputs.is_empty() || inputs.len() > 8 {
    return None;
  }
  let mut names = HashSet::new();
  let mut answer = page.clone();
  answer.set_query(None);
  for input in inputs {
    let name = input.attr("name");
    if name.is_empty()
      || name.len() > 32
      || !name.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
      || !names.insert(name)
    {
      return None;
    }
    let value = if name == "solution" {
      format!("{}{}", &seed[4], &seed[4])
    } else {
      let value = input.attr("value");
      if value.len() > 512
        || !value
          .bytes()
          .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
      {
        return None;
      }
      value.to_owned()
    };
    answer.query_pairs_mut().append_pair(name, &value);
  }
  (names.contains("solution") && names.contains("jsc_token")).then_some(answer)
}

enum Post {
  Video {
    target: String,
    title: Option<String>,
    author: Option<String>,
    thumbnail: Option<String>,
  },
  Items {
    title: Option<String>,
    author: Option<String>,
    items: Vec<MediaAttachment>,
  },
}
fn parse(body: &str, id: &str, video_id: Option<&str>) -> Option<Post> {
  let doc = Document::parse(body)?;
  let roots: Vec<_> = doc
    .0
    .iter()
    .enumerate()
    .filter(|(_, n)| n.tag.name == "shreddit-post" && n.attr("id") == format!("t3_{id}"))
    .map(|(i, _)| i)
    .collect();
  let [root] = roots.as_slice() else {
    return None;
  };
  let root = *root;
  let post = &doc.0[root];
  if post.tag.attrs.keys().any(|k| k.contains("nsfw")) {
    return None;
  }
  let owned = |i| doc.closest(i, |n| n.tag.name == "shreddit-post") == Some(root);
  let title = post::line(post.attr("post-title"));
  let author = post::line(post.attr("author"));
  match post.attr("post-type") {
    // Promoted and profile posts ship v.redd.it clips as "gif"; image gifs fall through to "image".
    "video" | "gif" if manifest(post.attr("content-href")).is_some() => {
      let manifest = manifest(post.attr("content-href"))?;
      if video_id.is_some_and(|expected| !manifest.contains(&format!("/{expected}/"))) {
        return None;
      }
      let (_, player) = doc.0.iter().enumerate().find(|(i, n)| {
        owned(*i)
          && player(&n.tag.name)
          && n.attr("post-id") == format!("t3_{id}")
          && manifest_from_player(n.attr("src")).as_deref() == Some(manifest.as_str())
      })?;
      Some(Post::Video {
        target: manifest,
        title,
        author,
        thumbnail: image_url(player.attr("poster")).map(|u| u.to_string()),
      })
    }
    "link" => {
      let url = http::https_url(post.attr("content-href")).ok()?;
      let host = url.host_str()?;
      if is_host(host) || host.ends_with(".reddit.com") || host.ends_with(".redd.it") {
        return None;
      }
      Some(Post::Video {
        target: url.to_string(),
        title,
        author,
        thumbnail: None,
      })
    }
    "image" | "gif" => {
      let items = vec![photo(
        &doc,
        root,
        Some(post.attr("content-href")),
        title.clone(),
      )?];
      Some(Post::Items {
        title,
        author,
        items,
      })
    }
    "gallery" | "mixed" => {
      let pages: Vec<_> = doc
        .0
        .iter()
        .enumerate()
        .filter(|(i, n)| {
          owned(*i)
            && n.tag.name == "li"
            && n.attr("slot").starts_with("page-")
            && doc
              .ancestors(*i)
              .any(|p| doc.0[p].tag.name == "gallery-carousel")
        })
        .map(|(i, _)| i)
        .collect();
      if pages.is_empty() || pages.len() > 40 {
        return None;
      }
      let mut items = Vec::new();
      for page in pages {
        let clip =
          doc.0.iter().enumerate().find(|(i, n)| {
            owned(*i) && player(&n.tag.name) && doc.ancestors(*i).any(|p| p == page)
          });
        let item = if let Some((_, n)) = clip {
          let target = manifest_from_player(n.attr("src"))?;
          MediaAttachment {
            id: target.clone(),
            file_index: None,
            kind: AttachmentKind::Video,
            url: target,
            thumbnail: image_url(n.attr("poster")).map(|u| u.to_string()),
            alt: title.clone(),
            width: None,
            height: None,
          }
        } else {
          photo(&doc, page, None, title.clone())?
        };
        items.push(item);
      }
      let mut seen = HashSet::new();
      if items.iter().any(|a| !seen.insert(a.id.clone())) {
        return None;
      }
      Some(Post::Items {
        title,
        author,
        items,
      })
    }
    _ => None,
  }
}
fn player(name: &str) -> bool {
  matches!(name, "shreddit-player" | "shreddit-player-2" | "video")
}
fn manifest(raw: &str) -> Option<String> {
  let url = http::https_url(raw).ok()?;
  let id = url.path().trim_matches('/');
  (url.host_str() == Some("v.redd.it") && identifier(id, 24))
    .then(|| format!("https://v.redd.it/{id}/DASHPlaylist.mpd"))
}
fn manifest_from_player(raw: &str) -> Option<String> {
  let mut url = http::https_url(raw).ok()?;
  let id = url.path_segments()?.next()?.to_owned();
  url.set_path(&format!("/{id}"));
  url.set_query(None);
  manifest(url.as_str())
}
fn image_url(raw: &str) -> Option<Url> {
  let url = http::https_url(raw).ok()?;
  (matches!(
    url.host_str(),
    Some("i.redd.it" | "preview.redd.it" | "external-preview.redd.it")
  ) && !url.query_pairs().any(|(k, _)| k == "blur"))
  .then_some(url)
}
fn photo(
  doc: &Document<'_>,
  root: usize,
  content: Option<&str>,
  title: Option<String>,
) -> Option<MediaAttachment> {
  let owner = doc.closest(root, |n| n.tag.name == "shreddit-post")?;
  let mut candidates = Vec::new();
  if let Some(url) = content
    .and_then(image_url)
    .filter(|u| u.host_str() == Some("i.redd.it"))
  {
    candidates.push((url, 0));
  }
  for (i, node) in doc.0.iter().enumerate() {
    if !doc.ancestors(i).any(|p| p == root)
      || doc.closest(i, |n| n.tag.name == "shreddit-post") != Some(owner)
    {
      continue;
    }
    if node.tag.name == "a" {
      if let Some(url) = image_url(node.attr("href")) {
        candidates.push((url, 0));
      }
    } else if node.tag.name == "img" {
      for key in ["src", "data-lazy-src", "srcset", "data-lazy-srcset"] {
        for entry in node.attr(key).split(',') {
          let mut parts = entry.split_ascii_whitespace();
          if let Some(url) = parts.next().and_then(image_url) {
            let width = parts
              .next()
              .and_then(|s| s.strip_suffix('w'))
              .and_then(|s| s.parse::<u32>().ok())
              .or_else(|| {
                url
                  .query_pairs()
                  .find(|(k, _)| k == "width")
                  .and_then(|(_, v)| v.parse().ok())
              })
              .unwrap_or(0);
            candidates.push((url, width));
          }
        }
      }
    }
  }
  let original = candidates
    .iter()
    .find(|(u, _)| u.host_str() == Some("i.redd.it"))
    .or_else(|| candidates.iter().max_by_key(|(_, w)| *w))?;
  let preview = candidates
    .iter()
    .filter(|(_, w)| *w >= 320)
    .min_by_key(|(_, w)| *w)
    .unwrap_or(original);
  let mut stable = original.0.clone();
  stable.set_query(None);
  Some(MediaAttachment {
    file_index: None,
    id: stable.to_string(),
    kind: AttachmentKind::Image,
    url: original.0.to_string(),
    thumbnail: Some(preview.0.to_string()),
    alt: title,
    width: None,
    height: None,
  })
}
