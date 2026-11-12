# End-to-end link matrix (developer-only)

```sh
cd src-tauri
cargo run --features link-matrix --example link_matrix -j 4 -- \
  --links <path>/links.tsv \
  --out <path>/link-matrix-out \
  --jobs 2
```

The command above runs anonymously. For a local login run, add:

```sh
--login 'firefox:/absolute/path/to/profile'
```

For an anonymous nightly job on Ubuntu or in a Linux container, use
[`scripts/link-nightly.sh` and its setup instructions](../../../docs/link-nightly.md).
The harness accepts `LINK_MATRIX_BIN_DIR` (yt-dlp, ffmpeg, ffprobe, deno) and
`LINK_MATRIX_RESOURCES_DIR` (gallery-dl plus gallery-dl.sha256). These override
the local macOS app paths and compile-time resource directory. Linux still needs
an X display and D-Bus session for the windowless Tauri/Wry host.

`--login firefox` uses the browser's default profile; `chrome:Default` is another
example. This explicitly consents once for every selected row. Fetches, playlist
children and downloads pass that choice through the app's normal `LoginSession`
path. The login run is **local-only** and refuses to start under `CI=1`; never
put a browser profile or cookie material in a CI job.

Alternatively, supply a local Netscape cookies.txt export:
```sh
--cookies '/absolute/path/to/cookies.txt'
```
This uses the same `auth.cookie_file` setting and production cookie-source path as
Settings > Logins > Advanced. The file must exist, be readable, and start with
the Netscape cookie header or contain tab-separated seven-column cookie records.
`--cookies` and `--login` are mutually exclusive; both refuse to run with `CI=1`.
Cookie contents and the cookie-file path are never included in reports/evidence.
Use separate output folders for anonymous, browser-login and cookie-file comparisons.

Before a release, run anonymous + one logged-in mode against the same link matrix:
```sh
cargo run --features link-matrix --example link_matrix -j 4 -- \
  --links <path>/links.tsv --out <path>/release-anonymous
cargo run --features link-matrix --example link_matrix -j 4 -- \
  --links <path>/links.tsv --out <path>/release-cookies --cookies <path>/cookies.txt
# Or replace --cookies <path>/cookies.txt with --login firefox.
```
Compare verdicts and attempt traces; investigate failures before releasing.

`--only linkedin` filters one site; `--only reddit,linkedin` selects either site
by an exact match on the input's `site` column, for nightly local per-site runs.
This is opt-in via `link-matrix`; neither the harness nor its network run is in the shipped app or default push tests.
Use public fixtures for repository research matrices. Keep private/account-specific inputs and cookies outside the repository.

## What runs

- Real windowless Tauri Wry app, not a mock extractor/process/network transport.
  Production functions require `AppHandle<Wry>` and cannot use `MockRuntime`.
- Production clipboard URL normalization (`capture/policy.rs::supported_url`).
  HTTP(S) validation represents manual paste for domains outside that allowlist.
- `gallery::run_media_info`: real gallery-first/mixed routing, gallery-dl probe,
  `ytdlp_info::run_ytdlp_info_fetch`, `resolve/` readers, production argument
  builders, error matcher, and login policy. Their original order is preserved;
  the harness does not move site readers ahead of gallery-first routing.
- Production `DownloadItem -> DownloadEntry` conversion and the exact worker
  function called by `scheduling/download_pipeline.rs`,
  `gallery::run_media_download`. This includes real gallery image transfers,
  receipts, naming, finalization, and yt-dlp media transfers/postprocessing.
- Real installed `yt-dlp`, `ffmpeg`, `ffprobe`, and `deno` found through the app's
  bin folder. Bundled gallery-dl is symlinked into an isolated resource directory,
  with its existing SHA256 receipt; production `find_bundled` verifies it.
- Existing production group cancellation on the 180-second per-link deadline;
  up to 25 seconds allowed to drain cancellation/process cleanup. Two links run
  concurrently; playlist children run sequentially within their parent's budget.

## Harness stand-ins and intentional limits

There is no GUI, dispatcher queue, clipboard watcher, tray, Sentry client, vault,
updater, or persisted owner configuration. Default snapshots use isolated stores.
Anonymous runs set browser cookies to `none` to prevent profile discovery. Gallery cache and file-engine state are isolated inside each run folder; a read-only personal cache cannot turn valid public metadata into a false extraction failure.
Login runs set automatic mode and supply the explicitly accepted browser on each
fetch/download entry. Cookie-file runs set only the manual cookie-file setting;
browser discovery stays disabled. No saved owner credentials or vault are loaded. Harness
settings remain in memory after login success; settings edits never save browser
choices or invoke OS settings callbacks. Dummy scheduler channels exist only for
the production login cleanup API. Direct worker calls retain real production routing, but do not exercise GUI events, queue fairness, or frontend
playlist-selection behavior.

The frontend's request mapping is represented in Rust (reference:
`src/stores/media/media.ts::downloadGroup` and
`src/stores/media/options.ts::getAttachmentSelection`). All attachments and all
returned playlist entries are selected. Zero-setup `TrackType::Both` and unset
quality/encoding options are retained even on audio-only sites. This exposes
default format failures instead of adapting format selection to expected types.
Only destination folder and the existing Advanced partial-download option
(first 15 seconds for AV) are overridden. This proves real selection, transfer,
and postprocessing, not full-duration playback or a strict network-byte ceiling.
Photos use the real production size limits. No site bugs are fixed here.

## Reports and validation

`results.tsv` and `results.md` are atomically refreshed after every completed row.
Each run gets a fresh UUID folder, with one numbered directory per input URL.
`evidence.json` contains app diagnostic/fatal events, sanitized engine output,
input annotations, and timing. Last engine lines and the app's actual code and
`errors.runner.<code>.message` key appear separately. Cookie-related engine
lines are suppressed. In browser-login and cookie-file runs, raw engine stderr/logs are omitted entirely;
app diagnostic codes and redacted messages remain. Cookie material and browser
profile paths are never written to reports/evidence. Reports contain public
source URLs and final downloaded file paths.

Both reports include `auth_mode` (`anonymous`, `login:<browser>` without the profile,
or `cookies-file`), the existing `login` (`yes` for a `--login` run, otherwise `no`),
`stages`, `winner`, and `final_code`. Each `stages` entry is
`stage/login/outcome/code/ms`, joined with ` > ` in production event order;
`login` within an attempt is `yes`/`no`, and a missing attempt code is `-`.
The winner stage and final code come from `media_attempts` (the fetch coordinator,
not a download-attempt trace). Playlist fetch traces are appended in order; the
last fetch event supplies the winner/final code. Missing event values stay blank;
file verification and later download failures still determine the row verdict.

Header columns are looked up by name (`site,label,expected_type,url`); legacy
Android rows without a header use that documented four-column layout. Comments
and duplicate URLs are skipped. Blank URL gaps remain FAIL rows explicitly
marked as uninvoked fixtures. Notes containing e.g. `4 feedshare images` or
`3-slide`, and unambiguous labels such as `post-images-3` or
`status-multi-photo-4`, supply explicit count assertions. Photo counts constrain
images; a slide count on a mixed post constrains images plus videos. Unknown expectations require a
verified media file but impose no kind assertion. `none` passes only on a failed
pipeline with terminal app code `noMediaFound`; `gone` requires `notFound404` or
`geoBlocked`. Both require no saved artifacts; earlier fallback codes, ambiguous
private/unavailable codes, cancellation and timeouts cannot satisfy them. `file`
requires a nonempty regular output with magic matching its suffix (PDF, ZIP,
EXE/DLL, MSI, 7z, RAR, gzip/tgz, bzip2, xz, ustar); unknown types fail closed. For a public file without a supported magic signature (for example TXT, CSV or source code), put an independently checked `sha256=<64 hex digits>` token in `note`. That row must match the entire file digest; an incorrect digest, malformed token or empty output fails. This does not relax unannotated rows.
LOGIN and GONE on other expectations require explicit diagnostic/error evidence;
403 and 429 remain FAIL. A timeout remains a harness-budget failure.

Media PASS requires no fatal app event, nonempty final files, ffprobe stream validation,
and matching expected kinds/counts. Every request must produce new verified files;
each selected image/video attachment must have a corresponding file, so a
playlist cannot pass on one child's files after another silently skips. Images
additionally pass production signature
validation and a real ffmpeg decode. Incomplete parts and metadata never count.
FAIL root-cause groups identify diagnostic/engine boundaries, not automatically
proven local bugs; inspect each row's evidence before proposing a site fix.

## Checks

Fast, offline fixture checks (not run by the normal push suite):

```sh
cargo fmt --all --check
cargo clippy --all-targets --features link-matrix -j 4 -- -D warnings
cargo test -j 4 --features link-matrix --example link_matrix -- --test-threads=2
```

Also run the normal Rust, lint and typecheck checks before you commit. A fresh checkout setup also needs the ignored gallery SHA256
resource prepared by `scripts/fetch-gallery-dl.py`, as for the normal app build.
