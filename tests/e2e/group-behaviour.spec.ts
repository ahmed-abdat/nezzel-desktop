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
