# Changelog

## 0.1.0-beta.2

Reliability release.

- Fixed: the browser login you chose was never passed to the download tool, so sign-in-only posts (Instagram, Reddit and others) failed even when you were signed in.
- Fixed: clips, live streams and some stream formats failed on macOS and Linux because the bundled ffmpeg could not verify HTTPS certificates.
- Photos from Flickr and Tumblr save again; X posts with a video and photos save everything; Facebook single-photo links save the photo.
- Vimeo links that answered "401" are retried once the way a browser would ask.
- Clearer messages: content your account cannot see, a browser whose cookies cannot be read (Keychain, Full Disk Access, browser open), disk full, file name too long, protected (DRM) streams, Facebook links the site does not hand out, Reddit posts with no media.
- Long non-Latin titles no longer produce file names that are too long on Linux.
- A quiet notice when the download tools could not update for a week, with "Try now".
- Failures the app cannot classify, and timeouts while online, are now included in error reports (same scrubbing and switch as before).

## 0.1.0-beta.1

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
