# Image engine

Nezzel runs gallery-dl 1.32.19 as a separate executable. It does not import Python
code into the app. The manifest pins the official source and executable SHA-256
hashes; `gallery-dl-build-requirements.txt` pins every wheel used by native builds.
The license generator includes the full GPL-2.0 license and exact corresponding
source URL in the app's third-party notices. Bundled Python/runtime/build credits
and full upstream licence texts are in `scripts/gallery-dl-bundled-NOTICES.txt`,
also included by the licence generator. These cover the native lock and optional
modules/libraries inventoried in the hash-verified official Windows/Linux files.

## Preparing a sidecar

Run `python3 scripts/fetch-gallery-dl.py --target <Rust target triple>` before
building Tauri. Windows x64 and Linux x64 use official release executables.
Windows ARM64 uses the official x64 executable through Windows 11 emulation.
macOS ARM64, macOS Intel, and Linux ARM64 build on native runners using Python
3.14.6 and PyInstaller. The script rejects cross-building and unpinned Python.
PyInstaller collects all `gallery_dl` submodules, including dynamically imported
extractors. CI builds each target on its own native runner.
The publish workflow prepares every target; Rust CI prepares its Linux sidecar.

The script verifies source/download hashes, installs with pip `--require-hashes
--only-binary=:all:`, checks the resulting version, full extractor registry and offline simulation
on a native host, and writes
`src-tauri/binaries/gallery-dl.sha256`. Native cache identity includes the manifest,
wheel lock and build recipe. Release preparation fails closed. Only development
may opt into `--allow-unavailable`, whose marker the runtime always rejects.
There is no runtime Python install, executable search on PATH, or remote manifest.

## Signing gate

Developer ID signing changes were not approved for this step. The native macOS
smoke build has PyInstaller's ad-hoc signature only. No certificate, entitlement,
notarization, or signing identity was changed. Before shipping a signed release,
the signing owner must sign the sidecar and its nested Mach-O components with the
existing Apple identity, regenerate the bundled checksum **after all sidecar
signature changes**, verify the installed sidecar and checksum, and then sign and
notarize the enclosing app. Tauri re-signing must not leave a stale checksum.
The existing publish secrets/signing flow remains the signing owner's seam.

## Runtime behavior

Single posts and unknown image URLs probe gallery-dl first. Explicit video URLs,
known video services and existing video-playlist fetches keep yt-dlp routing.
A video-only result goes to yt-dlp; mixed posts retain image selection and use
yt-dlp for selected videos. JSON error records are failures even when gallery-dl
exits zero. External queued links and link cards never become image attachments.
The bundled engine lacks Threads, LinkedIn and Telegram extractors. The existing
Rust Threads resolver feeds its attachments into the same selected-save path;
LinkedIn and Telegram ports remain a later gate. This step adds no new site port.

The engine passes `--config-ignore`, bounded timeouts/output, and fixed arguments.
Stderr is fully drained with only a bounded prefix retained. Verified executable
hashes are cached per process and invalidated when checksum/file metadata changes.
It reuses the existing per-host LoginSession consent and accepted browser/profile;
no browser discovery or cookie read occurs before that flow authorizes it.
Cookie-file and proxy overrides are reused. Credentials and proxy URLs live in
a private 0600 staging config passed with `-c`; dropping the runner deletes it.
Username/password settings are scoped to the supported source extractor category.
Custom bearer/header authentication is rejected with a localized explanation:
gallery extractors share their session with CDNs/redirects, so those headers must
not be forwarded. Browser impersonation is
reported as unavailable instead of silently ignored. Diagnostics contain typed
codes, not cookies, source URLs or filenames.

Downloads refresh gallery or Rust-resolver manifests and reject empty or stale
selected IDs. Resolver IDs remain intact; Bluesky blob identities retain their
`did`/`cid` query parameters while expiring signatures are ignored. Only
selected image ranges transfer into an owned private staging directory. Each
transfer's metadata receipt must still match its selected stable identity, so a
reordered post fails safely. Byte validation rejects HTML/error bodies. Offline
yt-dlp filename rendering uses the existing output resolver and template rules;
carousel filenames append the original attachment position before the validated
extension. Finalization never replaces an unrelated file, permits an identical
retry, and cleans staging on cancellation. Hard links provide atomic finalization where supported. FAT/exFAT/network shares
fall back to exclusive `create_new` plus copying, without replacing existing
files; cancelled/incomplete copies are cleaned up. Windows reserved names are
prefixed with `_`. Mixed
items emit completion only after the selected image and video work finishes.
Selected resolver videos use the admitted CDN URL with the original page as
referer. An expired CDN never falls back to downloading the whole post. Saved
images join the existing download-file records under the trusted output root.

## Focused verification

`cargo test --manifest-path src-tauri/Cargo.toml -j2 gallery --lib` exercises real
X metadata fixtures, exit-zero errors, selection receipts, browser/proxy argument
policy, mixed retry requests, naming, cancellation, checksum lookup and file
preservation. `scripts/test_fetch_gallery.py` runs offline bootstrap tests, also
included in the existing Vitest bootstrap test. Media-store tests assert selected
IDs at the Tauri command boundary. A manual live check runs the bundled executable for real extraction and selected
image transfers, and leaves unavailable URLs untested. A version-only smoke is
insufficient: the build must also enumerate the full extractor registry and find
Twitter, Instagram, Reddit, Bluesky and Pinterest before installing the sidecar.
A `-s` simulation of `https://nezzel.invalid/sidecar-smoke.jpg` also instantiates
the real direct-link extractor without DNS/HTTP or saved files.
