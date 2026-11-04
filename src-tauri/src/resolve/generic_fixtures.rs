//! Shared offline inputs for the legacy parser and the PageScan boundary.
pub(crate) fn videos() -> Vec<(&'static str, Option<&'static str>)> {
  vec![
    (
      r#"<meta content='/clip.mp4?a=1&amp;b=2' property='og:video'><meta property='og:image' content='/cover.jpg'>"#,
      Some("https://example.com/clip.mp4?a=1&b=2"),
    ),
    (
      r#"<video poster='/cover.jpg'><source src='https://cdn.example.com/video.webm'></video>"#,
      Some("https://cdn.example.com/video.webm"),
    ),
    (
      r#"<video src='/clip.mp4'>"#,
      Some("https://example.com/clip.mp4"),
    ),
    (r#"<meta property='og:image' content='/cover.jpg'>"#, None),
    (
      r#"<script>const x = '<video src="https://wrong.example/a.mp4">';</script><!-- <meta property='og:image' content='/a'> -->"#,
      None,
    ),
    (
      r#"<meta property='og:video' content='http://example.com/a.mp4'>"#,
      None,
    ),
  ]
}
pub(crate) fn frontends() -> Vec<(&'static str, &'static str, Option<&'static str>)> {
  vec![
    (
      r#"<script>player.src = `https://www.youtube-nocookie.com/embed/${d.vid}`;</script>"#,
      "https://hackertube.org/watch/qpoRO378qRY",
      Some("https://www.youtube.com/watch?v=qpoRO378qRY"),
    ),
    (
      r#"<iframe src='https://www.youtube.com/embed/qpoRO378qRY?rel=0'></iframe>"#,
      "https://example.com/article",
      Some("https://www.youtube.com/watch?v=qpoRO378qRY"),
    ),
    (
      r#"<a href='https://www.youtube.com/watch?feature=share&amp;v=qpoRO378qRY'>Watch</a>"#,
      "https://example.com/article",
      Some("https://www.youtube.com/watch?v=qpoRO378qRY"),
    ),
    (
      r#"<script>const url = 'https://youtu.be/qpoRO378qRY';</script>"#,
      "https://example.com/watch/abcdefghijk",
      Some("https://www.youtube.com/watch?v=qpoRO378qRY"),
    ),
    (
      r#"<iframe src='//www.youtube.com/embed/qpoRO378qRY'></iframe><iframe src='https://www.youtube.com/embed/abcdefghijk'></iframe>"#,
      "https://example.com/article",
      None,
    ),
    (
      r#"<iframe src='https://www.youtube.com/embed/abcdefghijk'></iframe><iframe src='https://www.youtube.com/embed/qpoRO378qRY'></iframe>"#,
      "https://example.com/watch/qpoRO378qRY",
      Some("https://www.youtube.com/watch?v=qpoRO378qRY"),
    ),
    (
      r#"<iframe src='https://www.youtube.com/embed/abcdefghijk'></iframe><iframe src='https://www.youtube.com/embed/0123456789_'></iframe>"#,
      "https://example.com/watch/qpoRO378qRY",
      None,
    ),
    (
      "<p>An unrelated page</p>",
      "https://example.com/watch/qpoRO378qRY",
      None,
    ),
    (
      "<img src='https://i.ytimg.com/vi/qpoRO378qRY/default.jpg'>",
      "https://example.com/watch?v=qpoRO378qRY",
      Some("https://www.youtube.com/watch?v=qpoRO378qRY"),
    ),
    (
      "<script>const host = 'https://youtube.com/watch';</script>",
      "https://example.com/article/qpoRO378qRY",
      None,
    ),
    (
      "<iframe src='https://youtube.com/embed/qpoRO378qRYextra'></iframe>",
      "https://example.com/article",
      None,
    ),
    (
      "<iframe src='https://youtube.com/embed/videoseries?list=PL123'></iframe>",
      "https://example.com/article",
      None,
    ),
    (
      "<iframe src='https://youtube.com/embed/live_stream?channel=UC123'></iframe>",
      "https://example.com/article",
      None,
    ),
    (
      "<iframe src='https://youtube.com/embed/videoseries?list=PL123'></iframe><iframe src='https://youtube.com/embed/qpoRO378qRY'></iframe>",
      "https://example.com/article",
      Some("https://www.youtube.com/watch?v=qpoRO378qRY"),
    ),
    (
      "<iframe src='https://YouTube.com/embed/qpoRO378qRY'></iframe>",
      "https://example.com/article",
      Some("https://www.youtube.com/watch?v=qpoRO378qRY"),
    ),
    (
      "<!-- <iframe src='https://youtube.com/embed/abcdefghijk'></iframe> --><script>const host = 'youtube.com/embed/';</script>",
      "https://example.com/watch/qpoRO378qRY",
      Some("https://www.youtube.com/watch?v=qpoRO378qRY"),
    ),
    (
      "<!-- <iframe src='https://youtube.com/embed/qpoRO378qRY'></iframe> -->",
      "https://example.com/watch/qpoRO378qRY",
      None,
    ),
    (
      "<meta property='og:video' content='/clip.mp4'><iframe src='https://youtube.com/embed/qpoRO378qRY'></iframe>",
      "https://example.com/article",
      Some("https://example.com/clip.mp4"),
    ),
    (
      "<video src='/clip.mp4'></video><iframe src='https://youtube.com/embed/qpoRO378qRY'></iframe>",
      "https://example.com/article",
      Some("https://example.com/clip.mp4"),
    ),
  ]
}
pub(crate) const SLOTS: &[&str] = &[
  "/watch/qpoRO378qRY",
  "/v/qpoRO378qRY",
  "/embed/qpoRO378qRY",
  "/shorts/qpoRO378qRY",
  "/video/qpoRO378qRY",
  "/live/qpoRO378qRY/",
  "/watch?v=qpoRO378qRY",
];
