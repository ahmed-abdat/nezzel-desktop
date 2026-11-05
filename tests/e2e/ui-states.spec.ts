/**
 * UI state gallery: renders every screen state in en+ar × light+dark × default+minimum window and
 * saves screenshots outside the repo (reviewed by eye, never compared). Slow (~700 shots), so it is
 * nightly only: skipped in CI unless NZ_NIGHTLY=1; locally it runs when named:
 *   pnpm exec playwright test tests/e2e/ui-states.spec.ts --workers=1
 * Output: $NZ_UI_STATES_DIR, default /private/tmp/nezzel-ui-states (macOS) or <tmp>/nezzel-ui-states.
 * NZ_RESPONSIVE=1: only the main screens (RESPONSIVE_STATES), light theme, at the common screen sizes.
 */
import { expect, type Page } from '@playwright/test';
import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import { test } from './utils/fixtures';
import type { MockData } from './utils/mockData';

test.skip(!!process.env.CI && !process.env.NZ_NIGHTLY, 'Nightly gallery: set NZ_NIGHTLY=1');
test.describe.configure({ timeout: 180_000 });

const OUT = process.env.NZ_UI_STATES_DIR
  ?? (process.platform === 'darwin' ? '/private/tmp/nezzel-ui-states' : path.join(os.tmpdir(), 'nezzel-ui-states'));
const FRAGMENTS = path.join(OUT, '.index');

const LOCALES = ['en', 'ar'] as const;
const THEMES = ['light', 'dark'] as const;
/** tauri.conf.json: default 800x900, minimum 750x650. */
const SIZES = { default: { width: 800, height: 900 }, min: { width: 750, height: 650 } } as const;
/** Wide windows (owner shots were 2000 px full screen): list columns and the Settings start anchor only show there. */
const WIDE_SIZES = { mid: { width: 1100, height: 720 }, wide: { width: 2000, height: 1100 } } as const;
const WIDE_STATES = new Set(['list-empty', 'list-filter-all', 'row-reading', 'row-downloading', 'row-done-video', 'long-titles', 'rows-100',
  'settings-general', 'settings-advanced', 'settings-logins-auto']);
/** Owner priority: fully responsive. Common laptop/desktop screens, smallest supported first. */
const RESPONSIVE = !!process.env.NZ_RESPONSIVE;
const RESPONSIVE_SIZES = { s1024: { width: 1024, height: 640 }, s1280: { width: 1280, height: 800 }, s1440: { width: 1440, height: 900 }, s2000: { width: 2000, height: 1100 } } as const;
const RESPONSIVE_STATES = new Set(['welcome', 'list-empty', 'row-downloading', 'row-error-rate', 'long-titles', 'rows-100', 'multi-select', 'playlist-selection',
  'sheet-video', 'media-detail', 'settings-general', 'settings-advanced', 'settings-logins-auto', 'settings-about-idle', 'report-dialog', 'install-failed']);
const CAPTURE_SIZES = { 'capture-bar': { width: 620, height: 230 }, 'capture-card': { width: 360, height: 180 } } as const;

type Locale = typeof LOCALES[number];
interface GalleryState {
  name: string;
  /** Seed from tests/e2e/utils/uiStateSeeds.ts; defaults to `name` for main-window states. */
  seed?: string | false;
  path?: string;
  window?: keyof typeof CAPTURE_SIZES;
  mockData?: MockData;
  /** Fake time to let pass after seeding (slow hints, countdowns). */
  advanceMs?: number;
  act?: (page: Page, locale: Locale) => Promise<void>;
}

const seedsOnly = (names: string[]): GalleryState[] => names.map(name => ({ name }));
const fill = (selector: string, value: string) => async (page: Page) => {
  await page.fill(selector, value);
};
const scrollMain = (top: number) => async (page: Page) => {
  await page.locator('#nz-main').evaluate((el, y) => el.scrollTo(0, y), top);
};
const aboutCheck = async (page: Page) => {
  const button = page.getByTestId('about-check');
  await button.scrollIntoViewIfNeeded();
  await button.click();
};

const capture = (window: keyof typeof CAPTURE_SIZES, name: string, data: MockData['capture'], extra: Partial<GalleryState> = {}): GalleryState => ({
  name: `${window}-${name}`, window, seed: false, mockData: { capture: data }, ...extra,
});
const VIDEO_OFFER = {
  url: 'https://www.youtube.com/watch?v=dQw4w9WgXcQ', site: 'youtube.com', title: 'How to make bread at home — full recipe',
  details: { thumbnail: '/src/assets/placeholder.png', mediaKind: 'video', photos: null, duration: 754, uploader: 'Nezzel Kitchen', views: 120_000, bestHeight: 1080, bestExt: 'mp4', size: 120_000_000, hasAudio: true, audioSize: 4_200_000, smallHeight: 720, smallSize: 48_000_000 },
};
const PHOTO_OFFER = {
  url: 'https://www.instagram.com/p/abc123/', site: 'instagram.com', title: 'صور من حفل التخرج',
  details: { thumbnail: null, mediaKind: 'carousel', photos: 4, duration: null, uploader: '@nezzel', views: null, bestHeight: null, bestExt: '', size: null, hasAudio: false, audioSize: null, smallHeight: null, smallSize: null },
};
const FILE_OFFER = {
  url: 'https://example.com/annual-report-2026.pdf', site: 'example.com', title: 'annual-report-2026.pdf',
  details: { thumbnail: null, mediaKind: 'file', photos: null, duration: null, uploader: '', views: null, bestHeight: null, bestExt: '', size: 2_400_000, hasAudio: false, audioSize: null, smallHeight: null, smallSize: null },
};
const SINGLE_OFFER = { ...VIDEO_OFFER, title: 'A short clip in one quality', details: { ...VIDEO_OFFER.details, bestHeight: 720, smallHeight: null, smallSize: null } };
const AUDIO_OFFER = {
  url: 'https://soundcloud.com/nezzel/episode-12', site: 'soundcloud.com', title: 'الحلقة ١٢: كيف نحفظ الصوت',
  details: { ...VIDEO_OFFER.details, thumbnail: null, mediaKind: 'audio', bestHeight: null, bestExt: 'm4a', size: 18_000_000, audioSize: 18_000_000, smallHeight: null, smallSize: null },
};
const LONG_OFFER = {
  ...VIDEO_OFFER,
  title: 'A very long video title that keeps going and going so the card has to clamp it — مع نص عربي طويل جدًا في المنتصف (full uncut version, 4K)',
};
const LOADING_OFFER = { url: 'https://www.youtube.com/watch?v=dQw4w9WgXcQ', site: 'youtube.com' };

const ALL_STATES: GalleryState[] = [
  // First run
  { name: 'welcome', seed: false, mockData: { preferences: { onboarding: { welcomeDone: false, closeNoteDone: false } } } },
  // Main list
  ...seedsOnly(['list-empty', 'list-filter-all', 'list-filter-active', 'list-filter-done', 'list-filter-failed']),
  { name: 'list-cleared', act: async page => page.locator('section[aria-labelledby="ledger-finished"] header button').click() },
  { name: 'list-search-empty', act: fill('#list-search', 'zzzz') },
  { name: 'row-reading' },
  { name: 'row-reading-slow', advanceMs: 12_000 },
  ...seedsOnly([
    'row-ready', 'row-downloading', 'row-paused', 'row-resuming', 'row-merging',
    'row-done-video', 'row-done-audio', 'row-done-photo', 'row-done-carousel', 'row-done-file', 'row-done-missing',
    'row-login-pending', 'row-login-accepted', 'row-login-refused', 'row-login-exhausted', 'row-login-dismissed', 'row-login-expired', 'row-login-timeout',
    'row-error-members', 'row-error-removed', 'row-error-drm', 'row-error-geo', 'row-error-rate', 'row-error-network',
    'row-error-noaudio', 'row-error-notpublic', 'row-error-unsupported', 'row-error-feed', 'row-error-unknown',
    'playlist-reading', 'playlist-selection', 'multi-select', 'long-titles', 'rows-100',
  ]),
  { name: 'rows-100-scrolled', seed: 'rows-100', act: scrollMain(2400) },
  { name: 'row-menu', seed: 'row-done-video', act: async page => page.locator('li.row').first().getByRole('button').last().click() },
  { name: 'media-detail', seed: 'row-done-video', act: async (page) => {
    await page.locator('li.row').first().click({ button: 'right' });
    await page.getByRole('menuitem', { name: /^(Details|التفاصيل)$/ }).click();
  } },

  // Download options sheet
  ...seedsOnly(['sheet-video', 'sheet-audio', 'sheet-playlist', 'sheet-waiting']),

  // Settings
  ...seedsOnly(['settings-general', 'settings-advanced', 'settings-logins-auto', 'settings-logins-none', 'settings-logins-browser', 'settings-accounts']),
  { name: 'settings-about-idle', seed: 'settings-about', act: async page => page.getByTestId('about-check').scrollIntoViewIfNeeded() },
  { name: 'settings-about-checking', seed: 'settings-about', mockData: { updater: { check: 'pending' } }, act: aboutCheck },
  { name: 'settings-about-latest', seed: 'settings-about', act: aboutCheck },
  { name: 'settings-about-available', seed: 'settings-about', mockData: { updater: { available: true, currentVersion: '1.0.0', availableVersion: '1.1.0' } }, act: aboutCheck },
  { name: 'settings-about-error', seed: 'settings-about', mockData: { updater: { check: 'fail' } }, act: aboutCheck },
  { name: 'report-dialog', seed: 'settings-about', act: async page => page.getByTestId('about-report').click() },
  { name: 'report-crash', seed: false, mockData: { crash: 'pending' } },
  { name: 'report-failed-row', seed: 'row-error-geo', act: async (page) => {
    await page.locator('li.row').first().click({ button: 'right' });
    await page.getByRole('menuitem', { name: /Report a problem|الإبلاغ عن مشكلة/ }).click();
  } },

  // First-run tools
  { name: 'install-progress', seed: false, mockData: { binaries: { check: ['yt-dlp', 'ffmpeg'], ensure: true } }, advanceMs: 1_200 },
  { name: 'install-failed', seed: false, mockData: { binaries: { check: ['yt-dlp', 'ffmpeg'], fail: true } } },
  { name: 'install-done', seed: false, mockData: { binaries: { check: ['yt-dlp', 'ffmpeg'], ensure: true } }, advanceMs: 4_200 },

  // Updater card, toasts, drop overlay
  ...seedsOnly(['updater-available', 'updater-downloading', 'updater-restart', 'updater-error', 'toast-success', 'toast-error', 'toast-undo', 'drop-overlay']),

  // Capture bar (quick bar) and copied-link card
  ...(['capture-bar', 'capture-card'] as const).flatMap(window => [
    capture(window, 'loading', { offer: LOADING_OFFER }),
    capture(window, 'slow', { offer: LOADING_OFFER }, { advanceMs: 6_000 }),
    capture(window, 'ready-video', { offer: VIDEO_OFFER }),
    capture(window, 'ready-photo', { offer: PHOTO_OFFER }),
    capture(window, 'ready-file', { offer: FILE_OFFER }),
    capture(window, 'ready-single', { offer: SINGLE_OFFER }),
    capture(window, 'ready-audio', { offer: AUDIO_OFFER }),
    capture(window, 'ready-long', { offer: LONG_OFFER }),
    capture(window, 'feed', { offer: { url: 'https://www.instagram.com/nezzel/', site: 'instagram.com', feed: true } }),
    capture(window, 'error', { offer: { url: 'https://example.com/x', site: 'example.com', failed: true } }),
    ...(['login', 'unsupported', 'protected', 'slow'] as const).map(reason =>
      capture(window, `error-${reason}`, { offer: { url: 'https://www.instagram.com/p/abc/', site: 'instagram.com', failed: true, reason } })),
    capture(window, 'busy', { offer: VIDEO_OFFER, action: 'pending' }, {
      act: async page => page.locator(window === 'capture-bar' ? 'button[type=submit]' : 'section button').last().click(),
    }),
  ]),
  capture('capture-bar', 'unsupported', { offer: {} }, { act: fill('#capture-url', 'just some words') }),
  capture('capture-bar', 'empty', { offer: {} }),
];

const STATES = RESPONSIVE ? ALL_STATES.filter(state => RESPONSIVE_STATES.has(state.name)) : ALL_STATES;

interface IndexEntry { state: string; locale: string; theme: string; size: string; file: string }

async function shoot(page: Page, state: GalleryState, locale: Locale, entries: IndexEntry[]) {
  const sizes = RESPONSIVE
    ? RESPONSIVE_SIZES
    : state.window ? { native: CAPTURE_SIZES[state.window] } : WIDE_STATES.has(state.name) ? { ...SIZES, ...WIDE_SIZES } : SIZES;
  for (const theme of RESPONSIVE ? (['light'] as const) : THEMES) {
    await page.evaluate(t => document.body.setAttribute('data-theme', t), theme);
    for (const [size, viewport] of Object.entries(sizes)) {
      await page.setViewportSize(viewport);
      await page.evaluate(() => document.fonts.ready);
      if (!state.window) {
        // The window body never scrolls (Linux showed a blank band under the status bar): panes scroll inside.
        const overflow = await page.evaluate(() => document.scrollingElement!.scrollHeight - window.innerHeight);
        expect(overflow, `${state.name} ${size}: page scrolls`).toBeLessThanOrEqual(0);
      }
      const file = `${state.name}-${locale}-${theme}-${size}.png`;
      await page.screenshot({ path: path.join(OUT, file), animations: 'disabled', caret: 'hide' });
      entries.push({ state: state.name, locale, theme, size, file });
    }
  }
}

test.beforeAll(() => {
  fs.mkdirSync(FRAGMENTS, { recursive: true });
});

test.afterAll(() => {
  const entries = fs.readdirSync(FRAGMENTS).flatMap(name => JSON.parse(fs.readFileSync(path.join(FRAGMENTS, name), 'utf8')) as IndexEntry[]);
  entries.sort((a, b) => a.file.localeCompare(b.file));
  fs.writeFileSync(path.join(OUT, 'index.json'), JSON.stringify({ generatedAt: new Date().toISOString(), count: entries.length, shots: entries }, null, 2));
});

for (const state of STATES) {
  test(`gallery: ${state.name}`, async ({ page }) => {
    const entries: IndexEntry[] = [];
    // Thumbnails and favicons point at real sites: never leave the machine.
    await page.route(/^https?:\/\/(?!localhost)/, route => route.abort());
    if (state.advanceMs) await page.clock.install();

    for (const locale of LOCALES) {
      const data: MockData = { ...state.mockData, settings: { appearance: { language: locale, theme: 'light' } } };
      await page.addInitScript((d) => {
        window.E2E = d;
        localStorage.clear(); // each load starts fresh (cleared rows, UI prefs persist otherwise)
      }, data);
      await page.setViewportSize(state.window ? CAPTURE_SIZES[state.window] : SIZES.default);
      await page.goto(state.window ? `/?window=${state.window}` : (state.path ?? '/'));
      await expect(page.locator('html')).toHaveAttribute('lang', locale);
      if (state.window) await expect(page.locator('section.capture-panel')).toBeVisible();
      else if (!state.name.startsWith('install')) await expect(page.locator('#nz-main')).toBeVisible({ timeout: 30_000 }); // first load waits for Vite

      if (state.seed !== false) {
        await page.evaluate(async (name) => {
          // @ts-expect-error served by the Vite dev server, not resolved by tsc
          const { seed } = await import('/tests/e2e/utils/uiStateSeeds.ts');
          await seed(name);
        }, state.seed ?? state.name);
      }
      await state.act?.(page, locale);
      // The install page mounts after a redirect: its timers must exist before fake time runs.
      if (state.name.startsWith('install')) await expect(page.getByRole('heading').first()).toBeVisible();
      if (state.advanceMs) await page.clock.runFor(state.advanceMs);
      await page.waitForTimeout(250); // let entry transitions settle; screenshots, not assertions
      await shoot(page, state, locale, entries);
    }
    fs.writeFileSync(path.join(FRAGMENTS, `${state.name}.json`), JSON.stringify(entries));
  });
}

/** A photo or a single file has nothing to choose: the options sheet must not stay open. */
for (const link of ['https://example.com/single-image.jpg', 'https://example.com/single-file.pdf']) {
  test(`gallery: options sheet skips ${link.split('/').pop()}`, async ({ page }) => {
    await page.route(/^https?:\/\/(?!localhost)/, route => route.abort());
    await page.goto('/');
    await page.fill('#queue-url-input', link);
    await page.keyboard.press('Enter');
    await expect(page.locator('li.row')).toHaveCount(1);
    await expect(page.locator('li.row')).not.toHaveAttribute('data-phase', 'reading');
    await expect(page.getByRole('dialog')).toHaveCount(0);
  });
}
