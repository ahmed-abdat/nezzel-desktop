/**
 * Settings switches must read at a glance in both directions (owner, 2026-11-05: "all teal, knob on the left,
 * can't tell"). On = teal track, knob at the inline end; off = neutral grey track, knob at the inline start.
 */
import { expect, type Page } from '@playwright/test';
import { test } from './utils/fixtures';
import { MAIN_PANE_ID } from '../../src/helpers/domIds';
import { SETTINGS_INDEX } from '../../src/components/settings/settingsIndex';
import { readFileSync } from 'node:fs';

const locale = (name: string) => JSON.parse(readFileSync(new URL(`../../src/locales/${name}.json`, import.meta.url), 'utf8')) as Record<string, unknown>;
const en = locale('en');
const ar = locale('ar');

async function openGeneral(page: Page, language: 'en' | 'ar') {
  await page.route(/^https?:\/\/(?!localhost)/, route => route.abort());
  await page.addInitScript((d) => {
    window.E2E = d;
    localStorage.clear();
  }, { settings: { appearance: { language, theme: 'light' } } });
  await page.goto('/');
  await expect(page.locator('html')).toHaveAttribute('lang', language);
  await expect(page.locator(`#${MAIN_PANE_ID}`)).toBeVisible({ timeout: 30_000 }); // first load waits for Vite
  await page.evaluate(async () => {
    // @ts-expect-error served by the Vite dev server, not resolved by tsc
    const { seed } = await import('/tests/e2e/utils/uiStateSeeds.ts');
    await seed('settings-general');
  });
}

/** Track size, fill saturation, and where the knob sits: 'start' or 'end' of the inline axis. */
function readSwitch(page: Page, id: string) {
  return page.locator(`#${id}`).evaluate((el) => {
    const track = el.getBoundingClientRect();
    const knob = el.querySelector('[data-slot=switch-thumb]')!.getBoundingClientRect();
    const rtl = getComputedStyle(el).direction === 'rtl';
    const knobCenter = knob.left + knob.width / 2;
    const atLeft = knobCenter < track.left + track.width / 2;
    // Resolve the background to sRGB through a canvas, then measure how coloured it is.
    const ctx = document.createElement('canvas').getContext('2d')!;
    ctx.fillStyle = '#fff';
    ctx.fillRect(0, 0, 1, 1);
    ctx.fillStyle = getComputedStyle(el).backgroundColor;
    ctx.fillRect(0, 0, 1, 1);
    const [r, g, b] = ctx.getImageData(0, 0, 1, 1).data;
    return {
      width: track.width, height: track.height, chroma: Math.max(r, g, b) - Math.min(r, g, b),
      knob: (atLeft !== rtl) ? 'start' : 'end', checked: el.getAttribute('aria-checked'),
    };
  });
}

for (const language of ['en', 'ar'] as const) {
  test(`switch on/off reads clearly (${language})`, async ({ page }) => {
    await openGeneral(page, language);
    const id = 'pref-tray';
    await expect(page.locator(`#${id}`)).toHaveAttribute('aria-checked', 'true');

    const on = await readSwitch(page, id);
    expect(on.width, 'switch is desktop-readable, not a 26x15 sliver').toBeGreaterThanOrEqual(32);
    expect(on.height).toBeGreaterThanOrEqual(18);
    expect(on.knob).toBe('end');
    expect(on.chroma, 'on is the teal accent').toBeGreaterThan(60);

    await page.locator(`#${id}`).click();
    await expect(page.locator(`#${id}`)).toHaveAttribute('aria-checked', 'false');
    // Polled: knob and track colour both transition.
    await expect.poll(async () => (await readSwitch(page, id)).knob, 'knob slides to the start').toBe('start');
    await expect.poll(async () => (await readSwitch(page, id)).chroma, 'off is neutral grey, not tinted').toBeLessThan(16);
  });
}

for (const language of ['en', 'ar'] as const) {
  test(`every settings search pick scrolls its pane only, never the window shell (${language})`, async ({ page }) => {
    await openGeneral(page, language);
    await page.setViewportSize({ width: 880, height: 560 });
    // Anything taller than the window (seen in the installed Mac app) gave the shell room to scroll, and a pick
    // then pushed the sidebar and toolbar out of the window with no way back.
    await page.evaluate(() => {
      for (const host of [document.body, document.getElementById('app')!]) {
        const tall = document.createElement('div');
        tall.style.cssText = 'position:absolute;top:0;inset-inline-start:0;width:1px;height:300vh;pointer-events:none';
        host.append(tall);
      }
      const w = window as Window & { __shellScrolls?: number };
      w.__shellScrolls = 0;
      document.addEventListener('scroll', (e) => {
        if (e.target === document || e.target === document.body) w.__shellScrolls!++;
      }, true);
    });
    const shellScrolls = () => page.evaluate(() => (window as Window & { __shellScrolls?: number }).__shellScrolls);
    const messages = language === 'ar' ? ar : en;
    const text = (key: string) => key.split('.').reduce<unknown>((node, part) => (node as Record<string, unknown>)?.[part], messages) as string;
    const search = page.getByTestId('settings-search');
    for (const entry of SETTINGS_INDEX.filter(e => !e.macOnly)) {
      await search.fill(text(entry.label));
      await page.keyboard.press('Enter');
      // The row itself, or the row that stands in for it when it only shows in some states.
      const ids = [entry.id, entry.parent].filter(Boolean).map(id => `[data-setting-id="${id}"][data-search-hit]`);
      await expect(page.locator(ids.join(', ')).first(), `"${entry.id}" is revealed`).toBeAttached();
      expect(await shellScrolls(), `picking "${entry.id}"`).toBe(0);
    }
    // Whatever else moves the page is put back at once.
    await page.evaluate(() => document.documentElement.scrollTo({ top: 200 }));
    await expect.poll(() => page.evaluate(() => document.documentElement.scrollTop)).toBe(0);
  });
}
