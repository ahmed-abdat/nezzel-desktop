//! Bounded structural view for post ownership; deliberately fails closed on excessive nesting.
use super::html;
use regex::Regex;
use std::sync::LazyLock;

static HIDDEN: LazyLock<Regex> = LazyLock::new(|| {
  Regex::new(r"(?i)(?:display\s*:\s*none|visibility\s*:\s*(?:hidden|collapse))").unwrap()
});
pub(super) struct Node<'a> {
  pub tag: html::Tag<'a>,
  pub parent: Option<usize>,
  pub children: Vec<usize>,
  pub text: String,
  tail: String,
}
impl Node<'_> {
  pub fn attr(&self, name: &str) -> &str {
    self.tag.attrs.get(name).map_or("", String::as_str)
  }
  pub fn has(&self, name: &str) -> bool {
    self.tag.attrs.contains_key(name)
  }
  pub fn class(&self, name: &str) -> bool {
    self
      .attr("class")
      .split_ascii_whitespace()
      .any(|c| c == name)
  }
}
pub(super) struct Document<'a>(pub Vec<Node<'a>>);
impl<'a> Document<'a> {
  pub fn parse(body: &'a str) -> Option<Self> {
    let mut nodes: Vec<Node<'a>> = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    for tag in html::all_tags(body)? {
      if tag.closing {
        if let Some(position) = stack.iter().rposition(|i| nodes[*i].tag.name == tag.name) {
          let closed = stack[position];
          stack.truncate(position);
          nodes[closed].tail.push_str(&html::entities(tag.text));
        }
        continue;
      }
      // HTML permits omitted li end tags; never let a neighbour become the preceding tile's child.
      if tag.name == "li" {
        if let Some(position) = stack
          .iter()
          .rposition(|i| matches!(nodes[*i].tag.name.as_str(), "li" | "ul" | "ol"))
        {
          if nodes[stack[position]].tag.name == "li" {
            stack.truncate(position);
          }
        }
      }
      if stack.len() >= 256 {
        return None;
      }
      let leaf = matches!(
        tag.name.as_str(),
        "area"
          | "base"
          | "br"
          | "col"
          | "embed"
          | "hr"
          | "img"
          | "input"
          | "link"
          | "meta"
          | "param"
          | "source"
          | "track"
          | "wbr"
          | "script"
          | "style"
          | "textarea"
          | "title"
          | "xmp"
          | "iframe"
          | "noembed"
          | "noframes"
          | "plaintext"
      );
      let i = nodes.len();
      let text = html::entities(tag.text);
      // Void-element text follows the element; keep it in order for captions and tile validation.
      let void = matches!(
        tag.name.as_str(),
        "area"
          | "base"
          | "br"
          | "col"
          | "embed"
          | "hr"
          | "img"
          | "input"
          | "link"
          | "meta"
          | "param"
          | "source"
          | "track"
          | "wbr"
      );
      if let Some(parent) = stack.last() {
        nodes[*parent].children.push(i);
      }
      nodes.push(Node {
        tag,
        parent: stack.last().copied(),
        children: Vec::new(),
        text: if void { String::new() } else { text.clone() },
        tail: if void { text } else { String::new() },
      });
      if !leaf {
        stack.push(i);
      }
    }
    Some(Self(nodes))
  }
  pub fn ancestors(&self, i: usize) -> impl Iterator<Item = usize> + '_ {
    std::iter::successors(self.0[i].parent, |i| self.0[*i].parent)
  }
  pub fn visible(&self, i: usize) -> bool {
    std::iter::once(i).chain(self.ancestors(i)).all(|i| {
      let node = &self.0[i];
      !matches!(
        node.tag.name.as_str(),
        "template" | "noscript" | "script" | "style" | "textarea"
      ) && !node.has("hidden")
        && !node.attr("aria-hidden").trim().eq_ignore_ascii_case("true")
        && !HIDDEN.is_match(node.attr("style"))
    })
  }
  pub fn closest(&self, i: usize, predicate: impl Fn(&Node<'_>) -> bool) -> Option<usize> {
    std::iter::once(i)
      .chain(self.ancestors(i))
      .find(|i| predicate(&self.0[*i]))
  }
  pub fn text(&self, i: usize) -> String {
    if self.0[i].tag.name == "br" {
      return "\n".into();
    }
    let mut text = self.0[i].text.clone();
    for child in &self.0[i].children {
      text.push_str(&self.text(*child));
      text.push_str(&self.0[*child].tail);
    }
    text
  }
  pub fn own_text(&self, i: usize) -> String {
    let mut text = self.0[i].text.clone();
    for child in &self.0[i].children {
      text.push_str(&self.0[*child].tail);
    }
    text
  }
}
