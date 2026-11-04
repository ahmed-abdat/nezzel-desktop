# Contributing to Nezzel Desktop

Small, focused pull requests are welcome. Check the existing issues first, and open one to describe the problem before you start on a large feature. Scope: public media you have the right to save, processed on your computer, working with zero setup. Contributions are accepted under the project licence, [AGPL-3.0-or-later](LICENSE).

Nezzel Desktop is a fork of [Open Video Downloader](https://github.com/jely2002/youtube-dl-gui). Where a change doesn't need to differ, we keep code close to upstream so upstream fixes still merge. Please don't reformat files you don't otherwise touch.

## Build from source

Tauri 2 (Rust) with a Vue 3 + TypeScript front end. You need:

- Node.js 22 or newer and [pnpm](https://pnpm.io/) 12 (`corepack enable` picks the pinned version). Don't use npm or yarn: the repo has only `pnpm-lock.yaml`.
- Rust stable through [rustup](https://rustup.rs/), plus the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) for your OS.
- Python 3: the build fetches and verifies the bundled gallery-dl and aria2 helpers.

```sh
git clone https://github.com/ahmed-abdat/nezzel-desktop.git && cd nezzel-desktop
pnpm install
pnpm tauri dev      # run the app in development
pnpm tauri build    # build an installer for this OS (also needs cargo-about for the licence file)
```

On first start the app downloads yt-dlp, ffmpeg and its other tools and checks their SHA-256 hashes (see [PRIVACY.md](PRIVACY.md)).

## Project layout

| Path | What it does |
| --- | --- |
| `src/` | Vue 3 + TypeScript UI (Pinia, vue-i18n, Tailwind 4, shadcn-vue in `src/components/ui/`) |
| `src/locales/` | UI translations |
| `src-tauri/src/` | Rust back end: link readers, yt-dlp/gallery-dl runners, download queue, tool downloads, settings |
| `src-tauri/locales/` | Tray menu and notification translations |
| `src-isolation/` | Tauri isolation bundle |
| `tests/` | Unit tests (Vitest) and end-to-end tests (Playwright) |
| `docs/` | Notes on the image engine and the translation glossary |

## Checks

Run these before opening a pull request. CI runs the same.

```sh
pnpm run lint
pnpm exec vue-tsc --noEmit
pnpm run test:unit
pnpm run build
cd src-tauri && cargo fmt --all --check && cargo clippy --all-targets -- -D warnings && cargo test
```

Run `pnpm run test:e2e` (Playwright) when you change a UI flow. Unit tests never use the network.

## Rules for changes

- **Defaults first.** A feature must work with no setup. Power options go under Advanced.
- **Privacy.** No telemetry, crash reports or update checks pointed at third-party accounts. Anything that sends data is opt-in, off by default, and never includes links, file names or cookies.
- **No hard-coded UI text.** Add keys to `src/locales/en.json` and `ar.json` (and `fr.json`). Use logical CSS (`ms-`/`me-`, `start`/`end`), never `left`/`right`, so Arabic stays right-to-left.
- **Tests.** A fix comes with a test that fails without it.
- **Commits.** `type: summary` (feat, fix, chore, docs, refactor, test, build), one concern per commit.

## Translating

Arabic is a first-class language and must read naturally right-to-left. The glossary in [docs/i18n-glossary.md](docs/i18n-glossary.md) lists the word to use for each idea in English, Arabic and French.

1. Run the app with `pnpm tauri dev`.
2. Edit `src/locales/<code>.json` and `src-tauri/locales/<code>.json` (the second holds the tray menu and notifications). The UI reloads when you save.
3. Don't rename keys or change the JSON structure. Keep placeholders such as `{count}`, `{error}` and `{version}` exactly as written, and keep markup like `<strong>` around the translated text.
4. Leave product and tool names in Latin script: Nezzel, yt-dlp, ffmpeg, SponsorBlock, MP4, MP3.

## Reporting problems

- A link that doesn't work: use the **Site support** issue form. No link is required, and never paste cookies, tokens or raw logs.
- A security issue: follow [SECURITY.md](SECURITY.md), not a public issue.
- Everyone taking part follows the [Code of Conduct](CODE_OF_CONDUCT.md).
