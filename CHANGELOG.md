# Changelog

## 0.1.0-beta.1 (unreleased)

First public beta of Nezzel Desktop, a fork of Open Video Downloader 3.2.1.

- Paste or copy a link and Nezzel reads it at once: videos, audio, photos, carousels and plain files.
- One engine picks the right tool per site (yt-dlp, gallery-dl, built-in readers for Reddit, Threads, Instagram and others) and falls back on its own when one fails.
- Faster: slideshows download in one batch; X and Reddit videos start without a second lookup.
- Sites that need an account use the browser you're already signed in to, only after you say yes, and only that site's cookies are passed on.
- Clear failure messages (sign-in needed, region blocked, removed, rate limited) instead of "unknown error".
- Library keeps your download history; "Remove from list" never deletes files.
- English, Arabic (right-to-left) and French.
- Crash and error reports to our own Sentry project (EU), on by default with a switch in Settings and on the welcome screen; no links, file names, paths or cookies are sent automatically. No analytics.

Known limits: macOS and Windows builds are not signed yet (see the install note in the README).
