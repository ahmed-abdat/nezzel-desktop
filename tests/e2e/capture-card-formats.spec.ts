/**
 * The copied-link card's "Download best ▾" format menu, at the card's real size (360x160, growing with the list),
 * in English and Arabic, with mocked previews: a YouTube video, a photo post, a playlist and the worst case.
 */
import { expect, type Page } from '@playwright/test';
import { test } from './utils/fixtures';
import type { MockData } from './utils/mockData';
import { emptyDetails, type CaptureDetails, type CaptureOffer, type CardChoice } from '../../src/captureTypes';
import type { CaptureCall } from '../utils/mocks/captureHandlers';

const CARD = { width: 360, height: 160 };
const MB = 1024 * 1024;
const video = (height: number, mb: number | null, fps?: number): CardChoice => ({ choice: { kind: 'video', height, ...(fps ? { fps } : {}) }, size: mb && mb * MB });
const audio = (kbps: number, mb: number | null): CardChoice => ({ choice: { kind: 'audio', kbps }, size: mb && mb * MB });
const details = (patch: Partial<CaptureDetails>): CaptureDetails => ({ ...emptyDetails(), ...patch });

const YOUTUBE = {
  url: 'https://www.youtube.com/watch?v=abc', site: 'youtube.com', title: 'How the Strait of Gibraltar was formed',
  details: details({
    mediaKind: 'video', duration: 872, bestHeight: 2160, bestExt: 'mp4', size: 1240 * MB, hasAudio: true, audioSize: 13 * MB,
    choices: [video(2160, 1240), video(1440, 640), video(1080, 312), video(720, 148), video(480, 74), video(360, 41), audio(129, 13.2), audio(48, 5)],
  }),
};
const PHOTO = {
  url: 'https://www.instagram.com/p/abc/', site: 'instagram.com', title: 'غروب على شاطئ نواكشوط',
  details: details({ mediaKind: 'carousel', photos: 6 }),
};
const PLAYLIST = {
  url: 'https://www.youtube.com/playlist?list=PL1', site: 'youtube.com', title: 'دورة الخط العربي للمبتدئين',
  details: details({ mediaKind: null, hasAudio: true }),
};
const WORST = {
  url: 'https://media.bibliotheque-numerique-nationale.example.org/v/1', site: 'media.bibliotheque-numerique-nationale.example.org',
  title: 'المحاضرة الكاملة 🎓 «تاريخ المخطوطات الشنقيطية» — الجزء الثالث مع ترجمة فورية وأسئلة الجمهور ونقاش مطوّل حول الحفظ الرقمي '.repeat(2),
  details: details({
    mediaKind: 'video', duration: 11_564, bestHeight: 4320, bestExt: 'webm', size: 12_800 * MB, hasAudio: true,
    choices: [
      ...[4320, 2160, 1440, 1080, 720].flatMap(h => [video(h, h, 60), video(h, h / 2)]),
      ...[480, 360, 240, 144].map(h => video(h, null)),
      ...[160, 129, 70, 48].map(abr => audio(abr, null)),
    ],
  }),
};

async function openCard(page: Page, locale: 'en' | 'ar', offer: Partial<CaptureOffer>) {
  await page.route(/^https?:\/\/(?!localhost)/, route => route.abort());
  const data: MockData = { capture: { offer }, settings: { appearance: { language: locale, theme: 'light' } } };
  await page.addInitScript((d) => {
    window.E2E = d;
  }, data);
  await page.setViewportSize(CARD);
  await page.goto('/?window=capture-card');
  await expect(page.locator('html')).toHaveAttribute('lang', locale);
  await expect(page.locator('section.capture-panel')).toBeVisible({ timeout: 30_000 }); // first load waits for Vite
}
const calls = (page: Page, cmd: string) =>
  page.evaluate(c => ((window as unknown as { __captureCalls?: CaptureCall[] }).__captureCalls ?? []).filter(call => call.cmd === c), cmd);
/** The native window follows capture_fit; the browser viewport plays that window here. */
async function followFit(page: Page) {
  const fits = await calls(page, 'capture_fit');
  const height = (fits.at(-1)?.args as { height: number }).height;
  await page.setViewportSize({ width: CARD.width, height });
  return height;
}
/** Nothing in the card part spills past the window's sides or below the card. */
async function expectNoOverflow(page: Page) {
  const spill = await page.locator('section.capture-panel').evaluate((panel) => {
    const box = panel.getBoundingClientRect();
    return [...panel.querySelectorAll<HTMLElement>('button, p, bdi')]
      .filter(el => el.getClientRects().length && !el.closest('[data-testid=capture-formats]'))
      .filter((el) => {
        const r = el.getBoundingClientRect();
        return r.left < box.left - 0.5 || r.right > box.right + 0.5 || r.bottom > 159.5;
      })
      .map(el => el.textContent?.trim().slice(0, 30));
  });
  expect(spill).toEqual([]);
}

for (const locale of ['en', 'ar'] as const) {
  test.describe(`card formats (${locale})`, () => {
    test('a video: one tap downloads best, ▾ lists every format and a row downloads that one', async ({ page }) => {
      await openCard(page, locale, YOUTUBE);
      await expect(page.getByTestId('capture-best')).toContainText('2160p');
      await expectNoOverflow(page);
      const toggle = page.getByTestId('capture-formats-toggle');
      await expect(toggle).toHaveAttribute('aria-expanded', 'false');

      // The ▾ sits at the inline end of the split button: right in English, left in Arabic.
      const main = await page.locator('[data-pick=best]').boundingBox();
      const caret = await toggle.boundingBox();
      expect(locale === 'ar' ? caret!.x < main!.x : caret!.x > main!.x).toBe(true);

      await toggle.click();
      await expect(toggle).toHaveAttribute('aria-expanded', 'true');
      const height = await followFit(page);
      expect(height).toBeGreaterThan(CARD.height);
      expect(height).toBeLessThanOrEqual(CARD.height + 300 + 1);
      const list = page.getByTestId('capture-formats');
      await expect(list.getByRole('menuitem')).toHaveCount(8);
      await expect(list.getByRole('group')).toHaveCount(2);
      const listBox = await list.boundingBox();
      expect(listBox!.y + listBox!.height).toBeLessThanOrEqual(height + 0.5);

      await list.locator('[data-choice="video:720"]').click();
      expect((await calls(page, 'capture_action')).at(-1)?.args).toMatchObject({ action: 'download', choice: { kind: 'video', height: 720 } });
      await list.locator('[data-choice="audio:129"]').click();
      expect((await calls(page, 'capture_action')).at(-1)?.args).toMatchObject({ action: 'download', choice: { kind: 'audio', kbps: 129 } });
      await page.locator('[data-pick=best]').click();
      expect((await calls(page, 'capture_action')).at(-1)?.args).toMatchObject({ action: 'download', choice: null });
    });

    test('keyboard: Enter on ▾ focuses the first row, arrows move, Esc closes the list but keeps the card', async ({ page }) => {
      await openCard(page, locale, YOUTUBE);
      const toggle = page.getByTestId('capture-formats-toggle');
      await toggle.focus();
      await page.keyboard.press('Enter');
      const rows = page.getByTestId('capture-formats').getByRole('menuitem');
      await expect(rows.first()).toBeFocused();
      await page.keyboard.press('ArrowDown');
      await expect(rows.nth(1)).toBeFocused();
      await page.keyboard.press('ArrowUp');
      await page.keyboard.press('ArrowUp');
      await expect(rows.last()).toBeFocused();
      await page.keyboard.press('Escape');
      await expect(page.getByTestId('capture-formats')).toHaveCount(0);
      await expect(toggle).toBeFocused();
      expect((await calls(page, 'capture_fit')).at(-1)?.args).toEqual({ height: CARD.height });
      expect((await calls(page, 'capture_action')).filter(c => (c.args as { action: string }).action === 'dismiss')).toEqual([]);
    });

    test('pointer: Escape after a mouse open closes the list, not the card', async ({ page }) => {
      await openCard(page, locale, YOUTUBE);
      const toggle = page.getByTestId('capture-formats-toggle');
      await toggle.click(); // focus stays on ▾, not in the list
      await expect(page.getByTestId('capture-formats')).toBeVisible();
      await page.keyboard.press('Escape');
      await expect(page.getByTestId('capture-formats')).toHaveCount(0);
      expect((await calls(page, 'capture_action')).filter(c => (c.args as { action: string }).action === 'dismiss')).toEqual([]);
      expect((await calls(page, 'capture_fit')).at(-1)?.args).toEqual({ height: CARD.height });
      await page.keyboard.press('Escape'); // closed list: Esc dismisses the card as before
      expect((await calls(page, 'capture_action')).at(-1)?.args).toMatchObject({ action: 'dismiss' });
    });

    test('Options opens the full sheet; Esc dismisses the card with focus on plain text or nowhere', async ({ page }) => {
      await openCard(page, locale, YOUTUBE);
      await page.getByRole('button', { name: locale === 'ar' ? /خيارات/ : 'Options' }).click();
      expect((await calls(page, 'capture_action')).at(-1)?.args).toMatchObject({ action: 'options' });

      await page.getByTestId('capture-best').click(); // focus lands on plain text, not on a control
      await page.keyboard.press('Escape');
      await expect.poll(async () => (await calls(page, 'capture_action')).filter(c => (c.args as { action: string }).action === 'dismiss').length).toBe(1);
    });

    test('a photo post has one plain Download, no menu', async ({ page }) => {
      await openCard(page, locale, PHOTO);
      await expect(page.getByTestId('capture-formats-toggle')).toHaveCount(0);
      await expect(page.locator('[data-pick=best]')).toBeVisible();
      await expectNoOverflow(page);
    });

    test('a playlist offers best for each video, height ceilings and audio', async ({ page }) => {
      await openCard(page, locale, PLAYLIST);
      await page.getByTestId('capture-formats-toggle').click();
      await followFit(page);
      const choices = await page.getByTestId('capture-formats').getByRole('menuitem').evaluateAll(rows => rows.map(r => r.getAttribute('data-choice')));
      expect(choices).toEqual(['best', 'video:1080', 'video:720', 'video:480', 'audio']);
    });

    test('worst case: long site and title stay inside the card, 18 rows scroll inside a capped list', async ({ page }) => {
      await openCard(page, locale, WORST);
      await expectNoOverflow(page);
      await page.getByTestId('capture-formats-toggle').click();
      const height = await followFit(page);
      expect(height).toBeLessThanOrEqual(CARD.height + 300 + 1);
      const list = page.getByTestId('capture-formats');
      await expect(list.getByRole('menuitem')).toHaveCount(18);
      const scroll = await list.evaluate(el => ({ scroll: el.scrollHeight, client: el.clientHeight }));
      expect(scroll.scroll).toBeGreaterThan(scroll.client);
      // Every row's cells fit their columns: nothing cut but the ellipsized details.
      const clipped = await list.getByRole('menuitem').evaluateAll(rows => rows.flatMap(row =>
        [...row.children].filter((cell, i) => i !== 1 && cell.scrollWidth > cell.clientWidth + 1).map(cell => cell.textContent)));
      expect(clipped).toEqual([]);
      await list.getByRole('menuitem').last().scrollIntoViewIfNeeded();
      await expect(list.locator('[data-choice="audio:48"]')).toBeInViewport();
    });
  });
}
