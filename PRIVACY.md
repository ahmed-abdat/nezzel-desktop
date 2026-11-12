# Privacy

Nezzel Desktop is free and open source, with no ads. Everything you save is processed on your computer. Crash and error reports and anonymous usage counts are on by default (see below).

## What goes over the network

- **Saving media:** requests go from your computer to the site you're saving from and its content servers. Before handing a link to yt-dlp, Nezzel may read the public page itself (Facebook, LinkedIn, Telegram, Reddit, Threads, Snapchat, Vimeo) to find the media. These requests go only to the site of the link you gave it.
- **Download tools:** yt-dlp, ffmpeg, ffprobe, Deno, gallery-dl and aria2 come inside the installer, so nothing is downloaded on first run. Each bundled file is checked against a SHA-256 hash pinned at build time.
- **Tool updates:** while **Update download tools automatically** is on (the default), Nezzel checks the yt-dlp releases on GitHub at most once a day (and once more whenever you press **Try now** on the notice that the tools could not update) and checks a new version against the SHA-256 list published with that release.
- **App updates:** while **Update Nezzel automatically** is on (the default), Nezzel checks `github.com/ahmed-abdat/nezzel-desktop/releases` for a newer version when it starts. Updates are signed, and the app checks the signature before installing. Portable, Microsoft Store and Snap builds skip this check.
- GitHub sees your IP address for these requests. No cookies, account or device identifiers are sent with them.

## Crash and error reports

Nezzel sends crash and error reports to our own [Sentry](https://sentry.io) project so we can find and fix bugs in the next release. This is **on by default**. Turn it off under **Settings → Advanced → Send crash and error reports**, or on the welcome screen.

**When a report is sent**

- Nezzel closed unexpectedly: the report is sent at the next start.
- A download failed (not when you cancel it, a filter skips it, or you're offline).
- An error inside the app window.
- You click **Report a problem** (in About, or on a failed download). This sends even with automatic reports off, because you asked.

**What a report contains**

- App version, operating system and version, processor type; on Linux the distribution, desktop and session type.
- Versions of the bundled tools (yt-dlp, ffmpeg, Deno, gallery-dl, aria2) and your app and system language.
- Your settings as on/off values and choices only (for example "custom folder: on"), never their contents.
- The error code and the steps the download went through.
- Only in reports you send yourself (**Report a problem**, or **Send report** after a crash): the last 50 lines of Nezzel's log, with private data removed. Automatic reports leave the log out.
- For a failed download: the site and the link's shape, for example `instagram.com/reel/{id}` or `youtube.com/watch?v={id}`. Known page words stay; IDs, share keys, user names and everything after `?` are removed. A personal or company subdomain is cut to the main site (`alice.tumblr.com` becomes `tumblr.com`).
- From **Report a problem** on a failed download: the full link, only while **Include the link** is checked (it is by default; uncheck it to send the shape only). Even then the part after `#` is removed, and so are link parameters that carry keys, tokens, signatures, sessions or share and tracking codes (for example Dropbox `rlkey`, a Mega key, `token=`, `si=`). Plus anything you type in the note.

**What is never sent**

Cookies, passwords and tokens, file names, folders and other local paths, your account or computer name, and email or IP addresses found in errors and logs. Nezzel removes them before a report leaves your computer, and links to local or private network addresses are never included. Like any server, Sentry sees the IP address of the connection; our project (hosted in the EU) is set not to store it.

Reports only work in builds made with our Sentry key. Builds without one send nothing. If you used Nezzel before reports existed, it tells you once that they are on, and a crash from that first start is offered to you instead of being sent.

## Usage counts

While **Settings → Advanced → Help improve Nezzel** is on (the default):

- Once a day, Nezzel sends anonymous totals to our own server (`nezzel.hululsoft.mr`): features used, how far links got (received, options shown, downloaded, failed), rough quality, speed and wait-time buckets, your settings, and the sites you saved from (domain only). Each send also names the app version, your system (macOS, Windows or Linux) and processor type, the app language and a rough install age. No ID, links, titles, file names, folders or account names. The server adds your country and keeps daily totals only.
- When a download fails, the count also records the app's error type (for example "sign-in required"), never the link, title, file name or error text.
- Your IP address is never stored. No third-party analytics.

Turn it off and nothing is sent; anything not yet sent is deleted. Builds made without our server address count and send nothing.

## Browser logins (cookies)

Some links only work when you're signed in. **Browser logins are used automatically by default** when a site needs them, without a consent question. Turn them off at **Settings → Logins → Use browser logins automatically**. An earlier explicit refusal or off setting stays off after an update.

- Nezzel tries its own saved session, Firefox, Chromium-family browsers, then Safari. It accepts a source only when it has the site's required, nonempty, unexpired login cookies. Windows App-Bound encrypted browser sessions are skipped.
- Only that site's domain cookies are passed to the download tools and used to authenticate with that site (for example `x.com` and `twitter.com` share a login). They are never uploaded to Nezzel or included in logs, Sentry reports or usage counts.
- To find a session, Nezzel checks browser profile names and cookie metadata. A Chromium/Safari helper may read the browser's full cookie store into a protected **local** temporary export. Nezzel extracts the target site's cookies and deletes that full export before starting the download; other sites' cookies are never passed to the downloader.
- Temporary jars are readable only by your user (0600 on Unix) and deleted after use. Leftovers from a crash are removed at the next start. Your browser's own profile is not modified.
- If no browser source works, supported sites can open a Nezzel sign-in window. Its per-site session stays on your computer. The window is restricted to the site's HTTPS login domains, cannot open other windows, and has no access to Nezzel commands. If Google rejects embedded sign-in, Nezzel offers Firefox or a `cookies.txt` file instead.
- The first time a browser login is actually used, Nezzel shows a one-time, non-blocking notice with a link to the off switch. It is remembered after the notice is displayed; no answer is needed to continue downloading.
- Under **Settings → Logins → Advanced**, you can choose a browser or load a `cookies.txt` file. Manual files are also copied into temporary jars containing only the target site's cookies; the original file is unchanged.

Browser/session storage and temporary files stay on your computer. Authentication cookies are sent only to the site they belong to as needed for your download, never to our reporting or usage services.

## Clipboard

While **Automatically fill in copied links** is on (the default), Nezzel watches the clipboard for links it can save and offers them to you. It reads the title and preview from the site through your proxy if one is set. With the proxy switch on, no site images are loaded. Other clipboard text is ignored. Turn off **Automatically fill in copied links** to stop this.

On Linux Wayland desktops without clipboard access for background apps (for example GNOME), copied links are picked up only while Nezzel’s window is focused.

## What stays on your computer

All of this lives in Nezzel's app data folder:

- Your settings.
- Your library and download history: title, site, thumbnail and where the file was saved. **Remove from list** removes the entry, never the file.
- Site passwords, tokens and headers you save under Settings → Logins, in an encrypted vault.
- The downloaded tools.

Download logs are kept in memory only and are gone when you quit. Nezzel's own log (with private data removed) is kept in `logs/` in the app data folder, at most 4 MB. Saved files go to `Downloads/Nezzel` or a folder you choose.
