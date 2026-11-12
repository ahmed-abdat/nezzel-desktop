# Changelog

## 0.1.0

First stable release.

- Automatic login: when a site needs you signed in, Nezzel uses your browser's login with no questions asked. The first time, a short note says so; you can turn it off in Settings > Logins. If you are not signed in anywhere, Nezzel can open a sign-in window for that site. Only that site's cookies are used, and they never leave your computer.
- Many more link shapes work: TikTok short links and photo posts, Bilibili player links, Facebook share/photo/m./web. links, Threads share links, and links copied with invisible characters around them. Expired links say "removed" and unsupported sites say so plainly instead of a generic error.
- New file hosts: pixeldrain, Yandex Disk, MediaFire, Internet Archive and GitHub (release files, raw and blob links).
- yt-dlp now updates without a new Nezzel release, from a signed list we publish.
- Linux: copied links are picked up on Wayland too (via wl-paste); the menu shows the right category; tray menu follows the app language.
- Cancel really stops a download and removes its leftover partial files; paused downloads keep theirs.
- Library keeps each download's details and failed rows across restarts; the card stays open while your mouse is on it; Instagram reels show their caption as the title.
- Windows: first YouTube link works without a retry, Facebook share links retry once, language switch updates every menu.
- Fixed many smaller issues found in the beta.4 test round on macOS, Windows and Linux.

## 0.1.0-beta.4

- Facebook photo posts (single photos, albums shared with "share/p", photo.php links) now save the photos instead of stopping at "this post has photos, not a video".
- Threads: a slow answer from Threads no longer turns into "not supported"; Nezzel retries once and follows threads.com/share links to the post.
- Vimeo: a video that needs a login now says so instead of a generic "couldn't download".
- Copied-link card: faster on YouTube, shows the post's caption as the title and its picture, hides the size column when sizes are unknown, and its button no longer looks broken while loading.
- Library: Video, Audio, Photos and Files show only your files; failed downloads stay in All downloads, with one line pointing to them. Remove all failed from the list, or report several failed downloads at once with one note.
- A failed download's page shows the message once and a Copy details button; the raw log is no longer on screen, and copied details keep one line per entry.
- macOS: the first time you close the window while Nezzel keeps running, a note says where it went and how to reopen it.
- Photo file names no longer contain "pNA".
- Anonymous usage counts (when on) now include the failure code, so we can see which errors are common.

## 0.1.0-beta.3

- Playlists: every way of saving a playlist opens a checklist first (select all or none, Space, Ctrl/Cmd+A, one quality for all, "Download N videos"); nothing starts until you choose. The options sheet no longer closes on its own.
- New "Needs you" section for downloads waiting on your choice or a login.
- Logins: one choice, Ask / Never / Always, plus the browser to use; cookies.txt stays under Advanced. With a fixed browser, cookies are read only for sites that need a login.
- New About page in Settings with Report a problem, version and update check. The window opens 880 px wide so the sidebar shows its labels.
- Paste several links at once; after dropping links, "Download all"; unsupported sites offer "Open in browser" and no pointless Retry.
- Clearer tool repair: the real cause of a failed tool download, an offline notice, and an honest "continue without tools".
- Arabic: sizes and speeds in Latin MB and MB/s, natural "time left", login labels read right to left; on macOS the window buttons no longer cover the Continue button or the sheet close button.
- Fixed: the app could crash on quit while reading the clipboard (beta.1); a crash in a background task is now reported at the next start (when reports are on).
- Privacy text now says plainly that crash reports and anonymous usage counts are on by default and can be turned off.

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
