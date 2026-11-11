# Usage counts (desktop)

Status: APPROVED by the lead 2026-11-05 (with the challenge-1 rulings folded in). Owner decision 2026-11-05 (AGENTS.md "Usage counts").
Model: Nezzel Android's usage counts (0.5 reports spec §5/§6, `usage/` package). Desktop speaks the same
`POST /api/usage` contract, plus a `platform` field that the server does not have yet (see "Server changes").

## Goal
Daily anonymous totals so we know which versions are live, which features and sites are used, and where links
fail, without an ID, links, titles, file names, paths or account names.

## 1. Wire contract
- `POST $NEZZEL_USAGE_URL` (build-time env, e.g. `https://nezzel.hululsoft.mr/api/usage`; unset or empty = the
  module is off: nothing counted, nothing sent). `Content-Type: application/json`, body ≤ 16 KB,
  `User-Agent: Nezzel-Desktop/<version>`, no cookies, no redirects, no retries, 15 s timeout. Only a `204` is success.
- Body: `{"platform":"desktop","app_version":"0.1.0-beta.1","counts":{"<key>":n}}`.
  `counts`: 1–200 keys, `n` integer 1–10000, key ≤ 80 chars (the server's limits; the client limits below sit
  inside them).
- `app_version` is the semver string (the server column is TEXT already). Android keeps sending its integer
  versionCode and no `platform` (server default `android`). Rejected alternative: mapping semver into 500–9999;
  it needs a lookup table and prereleases (`-beta.1`) don't fit.
- Until the server change is deployed, a desktop ping gets 400: counters stay (7-day cap). The release
  workflow sets `NEZZEL_USAGE_URL` only once the server is live, so nothing is sent before.

## 2. Client rules (Rust `src-tauri/src/usage/`)
- `keys.rs`: the one allow-list (fixed set + patterned `site:`/`sitein:`/`sitefail:`/`opt:`/`failcode:`). Unknown keys are
  dropped, never an error. Domain = `diagnostics::link` registrable-domain logic (shared helper, read-only
  reuse), lowercase, regex `[a-z0-9-]{1,63}(\.[a-z0-9-]{1,63})+`, ≤ 64 chars, numeric last label (IP) dropped.
- `store.rs`: `usage.json` in the app data dir: `{since, usage_day, counts, today:{day, per_key}, installed_at,
  last_version, ended_jobs}`. Atomic write (tmp + rename). Never logged.
- Caps: ≤ 50 distinct keys per open prefix between sends (Android's MAX_SITES; extra `site:` → `site.other`,
  extra `sitein:`/`sitefail:`/`opt:` dropped). **Desktop-only client limit:** each key counts at most 500 per UTC
  day (Android's loop bug: `dl.cancelled.video` 11,358 vs 217 started); over it the count is dropped and
  `usage.capped` is counted once that day. Compatible with the server's n ≤ 10000. Payload: once-keys first, fixed keys, then open keys by
  count, cut at 200.
- One end per job: `ended_jobs` (job id → origin, local only, pruned when the job leaves the list, cap 500)
  so a retried, resumed or replayed job never counts a second end.
- Send: `sender.rs`, a tokio task: first check 60 s after start, then every 6 h. Sends when the UTC day differs
  from `usage_day`. On 204: subtract the sent counts (counts added meanwhile stay), set `usage_day`, `since`.
  No separate network check (Android's WorkManager "network required"): an offline attempt is the equivalent:
  it fails, keeps the counters and is retried on the next tick. On any failure: keep; counters older than 7 days (`since`) are dropped before a send. Transport is a trait
  (`UsagePost`) so tests use a mock; no network in tests.
- Switch off: clear the counters (keep `usage_day`, `installed_at`, `last_version`), bump a generation so an
  in-flight send can't write back, count nothing until on again.
- Never sends: debug builds (`cfg!(debug_assertions)`), unit tests (mock transport only), e2e (front-end only,
  Tauri mocked), any build without `NEZZEL_USAGE_URL`.
- Front end: one command `usage_count(key)` for UI-only events: `paste`, `pause`, `pause_all`, `retry`,
  `report.opened` (Rust accepts only these). `media_download` gets an `origin` (tap/batch/resume) from the store. Download, funnel, site, update and engine keys are counted in Rust where the event happens.

## 3. Keys (A = same key as Android, D = desktop-only, new on the server)
| Key | Where counted |
|---|---|
| `active` (A) | once per ping |
| `open` (A) | app start + window shown from tray/background (`window.rs`, `tray.rs`) |
| `install.new` (D) | first run: `installed_at` absent (once per install, lesson 1) |
| `update.installed` (A) | first start with `last_version` ≠ current version |
| `update.offered/tapped/ready` (A), `update.failed.network/verify/install` (A) | updater commands (`updater_check/download/install`) |
| `engine.update.ok/failed` (A), `engine.fresh.d0-7/d8-30/d31up/bundled` (A, once-key) | `binaries_manager.rs` |
| `paste` (A) | link pasted/typed into the input |
| `clip.shown`, `clip.accepted` (A) | copied-link card shown (clipboard watcher) / Download or Options on it (`capture_action`) |
| `funnel.link` (A) | a link received (any source) |
| `funnel.options` (A) | options shown after a fetch (`fetch_pipeline.rs`) |
| `funnel.tapped` (A) | one Download on a single item |
| `funnel.batch` (A) | one Download on a playlist or several links (playlist key) |
| `funnel.finished/failed` (A) | end of a job whose origin is a `funnel.tapped` (lesson 3) |
| `resume.finished/failed` (D) | end of a resumed/retried job; `resume`, `retry`, `pause`, `pause_all` (A) for the taps |
| `dl.<started/finished/failed/cancelled>.<video/audio/photo>.<tier>` (A) | `download_pipeline.rs` start + `finish_download`, tiers as Android (`low…uhd/unknown`, `mp3/original`, `any`) |
| `options.ytdlp.<t0-2…t10up>` (A) | time to options when yt-dlp read the link |
| `options.own.<t0-2…t10up>` (A) | time to options when Nezzel's own reader or the direct-file path read it |
| `options.gallery.<t0-2…t10up>` (D) | time to options when gallery-dl read the link |
| `opt:<bucket>:<domain>` (A) | time to options per site |
| `speed.<s0-100k…s5mup>` (A) | average speed of a finished download |
| `site:<domain>` (A, meaning kept; registrable domain only) | a download ended (finished or failed) |
| `sitein:<domain>` (D) | a link received for that site |
| `sitefail:<domain>` (D) | a download failed; success rate = (site − sitefail)/site (lesson 2) |
| `failcode:<code>` (D) | once when a download fails, alongside `sitefail`; only fixed app error codes from `keys::FAILURE_CODES`, anything else → `unknown`; never engine text, links, titles or paths; same 500-per-key daily cap |
| `site.other` (A) | past the 40-site cap |
| `quick.download/audio/smaller/options` (D) | Quick bar / copied-link card actions (`capture_action`) |
| `card.format.video/audio` (D) | A row of the copied-link card's format menu (`capture_action`; height or bitrate itself not sent) |
| `tray.hidden` (D) | window closed to the tray/background |
| `shortcut.used` (D) | global shortcut fired |
| `report.opened` (D) | Report a problem opened |
| `login.profile` (D) | a download used a browser login profile |
| `usage.capped` (D) | a key hit the daily cap |
| once per ping (snapshot): `os.<macos/windows/linux>`, `arch.<x64/arm64>` (D); `set.lang.<auto/code>`, `lang.<code>` (A, desktop codes added: de es it ko nb nl pt-br pt-pt ru tr zh-tw); `set.theme.<system/light/dark>` (A); `set.parallel.<1/2/3/4up>` (A, `4up` D); `set.tray/.clipboard/.autostart/.reports.<on/off>` (D); `age.<day0/d1-7/d8-30/d31up>` (A) | `snapshot.rs` at send time |

Not sent (no key can form them): links, titles, file names, paths, cookies, account or profile names, device ID,
OS version, hostname. Failures with links stay in Sentry (unchanged, `diagnostics/` read only).

## 4. UI and copy
- Settings → Advanced, under "Send crash and error reports": switch **Help improve Nezzel** (`privacy.sendUsage`,
  default on), help "Sends anonymous daily totals: features used and sites (domain only). No links or file names."
- Welcome: nothing (lead ruling, as Android: no notice or onboarding). Separate from the crash-reports switch.
- Keys: `usage.setting.label/help` in en + ar (real Arabic), other locales fall back to en.
- PRIVACY.md: drop "no analytics"; add "Usage counts" near the end in Android's wording. README FAQ updated.

## 5. Server changes (private Worker; separate lane, not this one)
1. Migration: rebuild `usage_daily` with `platform TEXT NOT NULL DEFAULT 'android'`, PK
   `(day, platform, app_version, key)`; copy rows; `usage_budget` PK `(day, platform, class)` so a desktop
   flood can't starve Android; trigger classes `site:`, `sitein:`, `sitefail:`, `opt:`, `brand:` as open.
2. `/api/usage`: `platform` ∈ {android, desktop}, default android. Desktop `app_version` =
   `^\d+\.\d+\.\d+(-[a-z]+\.\d+)?$`, ≤ 24 chars; Android unchanged (int 500–9999).
3. Allow-list per platform: Android's FIXED set unchanged; a new DESKTOP_FIXED set (table below). Patterned
   keys are shared with the same syntax rules: `site:`, `opt:` (existing), `sitein:`, `sitefail:` (new; same
   domain regex, ≤ 64-char domain, numeric last label rejected). `failcode:<code>` is desktop-only:
   `^failcode:[a-zA-Z0-9]{1,48}$`, further restricted to the fixed app error-code allow-list in
   `keys::FAILURE_CODES`; anything unclassified counts as `failcode:unknown`. `sdk:`/`brand:` stay android-only.
4. Dashboard: platform toggle (Android / Desktop / All) on every card; desktop cards: OS/arch share, versions
   (semver), install.new per day, quick picks, copied-link card shown vs used, tray/shortcut, success rate per
   site from `site`/`sitefail`.
5. Order: D1 migration, then deploy; deploy needs the owner's OK. Then set `NEZZEL_USAGE_URL` in the release
   workflow (lead; `.github/workflows/` is outside this lane).

## 5b. Review round 1 (2026-11-05)
- Switch-off: `Usage.on` is read and written under the state lock; off clears, bumps the generation and fires a
  Notify that drops an in-flight POST (`prepare` → `deliver`). No count lands after off, no payload is sent after.
- Delivery is at-least-once: a crash after the server's 204 but before the file is saved can send those counts
  again the next day. Counts are approximate; accepted (lead ruling C4).
- Leftover keys past the 200-key / 16 KB cut keep their age (`since`), so they still expire after 7 days. Keys a
  newer build no longer allows, or that clash with a once-key, are dropped.
- Sites: the PSL registrable domain only (`drive.google.com` → `google.com`); private-section suffixes step to the
  nearest ICANN suffix + one label (`…compute-1.amazonaws.com` → `amazonaws.com`); no listed suffix = not counted.
- `funnel.batch`: once per user action (the store counts it; each link's `media_download` says origin `batch`).
- Job records go when the item leaves the list, and when its link is read again (Retry reuses the ID).
- Send: first check 10 s after start, every 6 h, and a best-effort send on quit (≤ 2 s).

## 6. Not counted (decided while building)
- `pick.*` (Android's quality-vs-preselected): desktop's options sheet has no single preselected row to compare
  against reliably; left out rather than shipping a key that never fires (lesson 4).
- `options.cache.*`: desktop has no reused-inspection path.
- A retry re-reads the link, so it counts `funnel.link` again; `retry` counts the tap itself.

## 6b. Done-check
- `cargo test usage::`: allow-list table (accepts/rejects, domain cap, IP), every key fires (each hook fn →
  key; and every fixed key is produced by some hook, lesson 4), per-key daily cap + `usage.capped`, payload
  shape and 200-key cut, day rollover, 204 resets / failure keeps / 7-day drop, switch-off clears + generation,
  one end per job, install.new once, update.installed once per version.
- vitest: Settings switch patches `privacy.sendUsage`; UI keys call `usage_count` with the right key.
- lint, vue-tsc, cargo fmt/clippy `-D warnings`; one payload dump in the report.

## 7. Desktop allow-list (the server's DESKTOP_FIXED + shared patterns)
Fixed (generated the same way as Android's, `src-tauri/src/usage/keys.rs` is the source):
- `active`, `open`, `install.new`, `update.installed`, `update.offered`, `update.tapped`, `update.ready`,
  `update.failed.network`, `update.failed.verify`, `update.failed.install`, `engine.update.ok`, `engine.update.failed`,
  `engine.fresh.{d0-7,d8-30,d31up,bundled}`, `paste`, `clip.shown`, `clip.accepted`, `pause`, `pause_all`, `resume`,
  `retry`, `resume.finished`, `resume.failed`, `funnel.{link,options,tapped,batch,finished,failed}`,
  `site.other`, `quick.{download,audio,smaller,options}`, `card.format.{video,audio}`, `tray.hidden`,
  `shortcut.used`, `report.opened`, `login.profile`, `usage.capped`
- `dl.{started,finished,failed,cancelled}.video.{low,sd,hd,fhd,qhd,uhd,unknown}`, `…audio.{mp3,original}`,
  `…photo.any`
- `options.{ytdlp,own,gallery}.{t0-2,t2-5,t5-10,t10up}`, `speed.{s0-100k,s100k-1m,s1m-5m,s5mup}`
- `os.{macos,windows,linux}`, `arch.{x64,arm64}`, `set.lang.{auto,<locale>}`, `lang.<locale>` with locale ∈
  {ar,de,en,es,fr,it,ko,nb,nl,pt-br,pt-pt,ru,tr,zh-tw}, `set.theme.{system,light,dark}`,
  `set.parallel.{1,2,3,4up}`, `set.{tray,clipboard,autostart,reports}.{on,off}`, `age.{day0,d1-7,d8-30,d31up}`
Patterned (shared): `site:<domain>`, `sitein:<domain>`, `sitefail:<domain>`, `opt:<t0-2|t2-5|t5-10|t10up>:<domain>`.

Patterned (desktop-only): `failcode:<code>` (`^failcode:[a-zA-Z0-9]{1,48}$` and `code` ∈
`keys::FAILURE_CODES`; unknown/unclassified → `unknown`). Counts once per failed job, including when no
site domain is available; the same 500-per-key UTC-day client limit applies.
