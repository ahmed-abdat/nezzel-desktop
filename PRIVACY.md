# Privacy

Nezzel Desktop has no ads, no account and no Nezzel server. Everything you save is processed on your computer.

## What goes over the network

- **Saving media:** requests go from your computer to the site you're saving from and its content servers. Before handing a link to yt-dlp, Nezzel may read the public page itself (Facebook, LinkedIn, Telegram, Reddit, Threads, Snapchat, Vimeo) to find the media. These requests go only to the site of the link you gave it.
- **Download tools:** on first run Nezzel downloads the tools it needs from their official GitHub releases: [yt-dlp](https://github.com/yt-dlp/yt-dlp), ffmpeg and ffprobe ([ffmpeg-static](https://github.com/eugeneware/ffmpeg-static), or [yt-dlp/FFmpeg-Builds](https://github.com/yt-dlp/FFmpeg-Builds) on Windows ARM), [Deno](https://github.com/denoland/deno) and [AtomicParsley](https://github.com/wez/atomicparsley). Each file is checked against a SHA-256 hash pinned in the app before it's used. gallery-dl and aria2 come inside the installer, so they need no download.
- **Tool updates:** while **Update download tools automatically** is on (the default), Nezzel checks the yt-dlp releases on GitHub at most once a day and checks a new version against the SHA-256 list published with that release.
- **App updates:** while **Update Nezzel automatically** is on (the default), Nezzel checks `github.com/ahmed-abdat/nezzel-desktop/releases` for a newer version when it starts. Updates are signed, and the app checks the signature before installing. Portable, Microsoft Store and Snap builds skip this check.
- GitHub sees your IP address for these requests. No cookies, account or device identifiers are sent with them.

## Crash and error reports

Nezzel sends crash and error reports to our own [Sentry](https://sentry.io) project so we can find and fix bugs in the next release. This is **on by default**. Turn it off under **Settings → Advanced → Send crash and error reports**, or on the welcome screen. There are no analytics.

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

Cookies, passwords and tokens, file names, folders and other local paths, your account or computer name, and email or IP addresses found in errors and logs. Nezzel removes them before a report leaves your computer, and links to local or private network addresses are never included. Like any server, Sentry sees the IP address of the connection; our project is set not to store it.

Reports only work in builds made with our Sentry key. Builds without one send nothing. If you used Nezzel before reports existed, it tells you once that they are on, and a crash from that first start is offered to you instead of being sent.

## Browser logins (cookies)

Some links only work when you're signed in. Nezzel never reads your browser's cookies until you agree:

- When a site asks for a login, Nezzel asks whether to use the browser you're signed in with, for that site.
- Only that site's cookies are passed to yt-dlp (for example `x.com` and `twitter.com` count as one site). Cookies for other sites are left alone.
- The cookies go into a temporary file only your user can read, and it is deleted when the download ends. Leftovers from a crash are removed at the next start.
- If you say yes, Nezzel remembers which browser you chose for that site, never the cookie values. You can turn on **Always use my browser logins** to skip the question.
- Under Settings → Logins you can instead pick a browser yourself (yt-dlp then reads that browser's cookies for every download) or load a `cookies.txt` file.

## Clipboard

While **Automatically fill in copied links** is on (the default), Nezzel watches the clipboard for links it can save and offers them to you. Other clipboard text is ignored. Nothing from the clipboard is sent anywhere unless you download it.

## What stays on your computer

All of this lives in Nezzel's app data folder:

- Your settings.
- Your library and download history: title, site, thumbnail and where the file was saved. **Remove from list** removes the entry, never the file.
- Site passwords, tokens and headers you save under Settings → Logins, in an encrypted vault.
- The downloaded tools.

Download logs are kept in memory only and are gone when you quit. Nezzel's own log (with private data removed) is kept in `logs/` in the app data folder, at most 4 MB. Saved files go to `Downloads/Nezzel` or a folder you choose.
