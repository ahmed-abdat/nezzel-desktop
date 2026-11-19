//! How much video SponsorBlock cut out. yt-dlp never prints the removed ranges, so the download
//! asks for `sponsorblock_chapters` after the move (see `with_temporary_files`) and the ranges of
//! the categories we asked it to remove are merged and summed here.
use serde::Deserialize;

pub(super) const PRINT_PREFIX: &str = "NEZZEL_SB|";

#[derive(Deserialize)]
struct Segment {
  start_time: Option<f64>,
  end_time: Option<f64>,
  category: Option<String>,
}

#[derive(Default)]
pub(super) struct RemovedSegments {
  categories: Vec<String>,
  ranges: Vec<(f64, f64)>,
}

impl RemovedSegments {
  pub fn new(categories: Vec<String>) -> Self {
    Self {
      categories,
      ranges: Vec::new(),
    }
  }

  pub fn record(&mut self, line: &str) {
    let Some(json) = line.trim().strip_prefix(PRINT_PREFIX) else {
      return;
    };
    // `NA`, `null` or a quoted default when the video had no SponsorBlock data.
    let Ok(segments) = serde_json::from_str::<Vec<Segment>>(json) else {
      return;
    };
    for segment in segments {
      let (Some(start), Some(end)) = (segment.start_time, segment.end_time) else {
        continue;
      };
      let wanted = segment
        .category
        .is_some_and(|category| self.categories.contains(&category));
      if wanted && start.is_finite() && end.is_finite() && end > start && start >= 0.0 {
        self.ranges.push((start, end));
      }
    }
  }

  /// Whole seconds removed, overlaps counted once; `None` when nothing was removed.
  pub fn seconds(&self) -> Option<u32> {
    let mut ranges = self.ranges.clone();
    ranges.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut total = 0.0;
    let mut current: Option<(f64, f64)> = None;
    for (start, end) in ranges {
      match current {
        Some((from, to)) if start <= to => current = Some((from, to.max(end))),
        Some((from, to)) => {
          total += to - from;
          current = Some((start, end));
        }
        None => current = Some((start, end)),
      }
    }
    if let Some((from, to)) = current {
      total += to - from;
    }
    (total > 0.0).then(|| (total.round() as u32).max(1))
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn removed(categories: &[&str], lines: &[&str]) -> Option<u32> {
    let mut found = RemovedSegments::new(categories.iter().map(|c| c.to_string()).collect());
    for line in lines {
      found.record(line);
    }
    found.seconds()
  }

  #[test]
  fn sums_removed_ranges_once_per_second_of_video() {
    let one =
      r#"NEZZEL_SB|[{"start_time":10.0,"end_time":40.5,"category":"sponsor","type":"skip"}]"#;
    let overlap = r#"NEZZEL_SB|[{"start_time":10.0,"end_time":30.0,"category":"sponsor"},{"start_time":25.0,"end_time":50.0,"category":"selfpromo"},{"start_time":100.0,"end_time":120.0,"category":"sponsor"}]"#;
    let marked = r#"NEZZEL_SB|[{"start_time":0.0,"end_time":9.0,"category":"intro"}]"#;
    for (categories, lines, expected) in [
      (vec!["sponsor"], vec![one], Some(31)),
      (vec!["sponsor", "selfpromo"], vec![overlap], Some(60)),
      (vec!["sponsor"], vec![overlap], Some(40)),
      (vec!["sponsor"], vec![marked], None),
      (
        vec!["sponsor"],
        vec![
          "NEZZEL_SB|NA",
          "NEZZEL_SB|null",
          "NEZZEL_SB|\"[]\"",
          "NEZZEL_SB|[]",
        ],
        None,
      ),
      (
        vec!["sponsor"],
        vec![
          "[SponsorBlock] Found 2 segments in the SponsorBlock database",
          "[ModifyChapters] Removing chapters from a.mp4",
        ],
        None,
      ),
      (
        vec!["sponsor"],
        vec![r#"NEZZEL_SB|[{"start_time":5.0,"end_time":5.2,"category":"sponsor"}]"#],
        Some(1),
      ),
    ] {
      assert_eq!(removed(&categories, &lines), expected, "{lines:?}");
    }
  }
}
