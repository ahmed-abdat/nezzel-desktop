import { expect, Page } from '@playwright/test';
import { test } from './utils/fixtures';
import { MAIN_PANE_ID, ROW_SELECTOR } from '../../src/helpers/domIds';

async function addPlaylistAndApply(page: Page, url: string) {
  await page.goto('/');
  await page.fill('#queue-url-input', url);
  await page.keyboard.press('Enter');
  await page.getByRole('dialog', { name: 'Download options' }).getByRole('button', { name: 'Download 50 videos', exact: true }).click();
}

test('a large playlist shows as one playlist row over its downloads', async ({ page }) => {
  await addPlaylistAndApply(page, 'https://example.com/playlist-large');
  const parent = page.locator(`${ROW_SELECTOR}[data-playlist]`);
  await expect(parent).toHaveCount(1);
  await expect(parent.getByText('Playlist')).toBeVisible();
  // Items sit under it (folded: only the next few waiting ones show), never as separate top-level rows.
  await expect(page.locator(`${ROW_SELECTOR}[aria-level="2"]`).first()).toBeVisible();
});

test('a playlist row expands, pauses all and offers Resume all', async ({ page }) => {
  await page.goto('/');
  await expect(page.locator(`#${MAIN_PANE_ID}`)).toBeVisible();
  await page.evaluate(async () => {
    // @ts-expect-error served by the Vite dev server, not resolved by tsc
    const { seed } = await import('/tests/e2e/utils/uiStateSeeds.ts');
    await seed('playlist-collapsed');
  });
  const toggle = page.getByTestId('playlist-toggle');
  await expect(toggle).toBeVisible();
  await toggle.click();
  await expect(page.getByText('Knife skills: lesson 11')).toBeVisible();
  await page.getByRole('button', { name: 'Pause all', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Resume all', exact: true })).toBeVisible();
});

test('a saved photo post opens in place and opens the chosen photo, in Arabic too', async ({ page }) => {
  await page.goto('/');
  await expect(page.locator(`#${MAIN_PANE_ID}`)).toBeVisible();
  await page.evaluate(async () => {
    // @ts-expect-error served by the Vite dev server, not resolved by tsc
    const { addRow, appearance } = await import('/tests/e2e/utils/uiStateSeeds.ts');
    // @ts-expect-error served by the Vite dev server, not resolved by tsc
    const { useMediaDestinationStore } = await import('/src/stores/media/destination.ts');
    // @ts-expect-error served by the Vite dev server, not resolved by tsc
    const { MediaState } = await import('/src/stores/media/state.ts');
    await appearance('ar', 'light');
    const { groupId } = addRow({ title: 'Instagram post', url: 'https://www.instagram.com/p/abc/', extractor: 'instagram', kind: 'carousel', state: MediaState.done });
    for (const n of [1, 2, 3]) useMediaDestinationStore().addDestination(groupId, { path: `/Users/me/Downloads/Nezzel/post_${n}.jpg`, confidence: 100 });
    // Record what the opener is asked to open (the e2e backend answers every other call as before).
    const internals = (window as unknown as { __TAURI_INTERNALS__: { invoke: (cmd: string, args?: unknown, options?: unknown) => unknown } }).__TAURI_INTERNALS__;
    const invoke = internals.invoke;
    const opened: string[] = [];
    (window as unknown as { __opened: string[] }).__opened = opened;
    internals.invoke = (cmd, args, options) => {
      if (cmd === 'plugin:opener|open_path') opened.push((args as { path: string }).path);
      return invoke(cmd, args, options);
    };
  });
  await expect(page.locator('html')).toHaveAttribute('dir', 'rtl');
  const toggle = page.getByTestId('files-toggle');
  await expect(toggle).toHaveAttribute('aria-expanded', 'false');
  await toggle.click();
  await expect(toggle).toHaveAttribute('aria-expanded', 'true');
  const lines = page.getByTestId('saved-file');
  await expect(lines).toHaveCount(3);
  await expect(lines.nth(1)).toContainText('post_2.jpg');
  // RTL: the icon sits at the start (right) of the line, the actions at its end (left).
  const second = lines.nth(1);
  const icon = await second.getByTestId('saved-file-open').locator('span').first().boundingBox();
  const name = await second.locator('bdi').boundingBox();
  const open = await second.getByRole('button', { name: 'فتح', exact: true }).boundingBox();
  expect(icon!.x).toBeGreaterThan(name!.x);
  expect(name!.x).toBeGreaterThan(open!.x);
  await second.getByRole('button', { name: 'فتح', exact: true }).click();
  await expect.poll(() => page.evaluate(() => (window as unknown as { __opened: string[] }).__opened)).toEqual(['/Users/me/Downloads/Nezzel/post_2.jpg']);
});
