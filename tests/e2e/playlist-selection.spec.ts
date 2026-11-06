import { expect, Page } from '@playwright/test';
import { test } from './utils/fixtures';
import { ROW_SELECTOR } from '../../src/helpers/domIds';

async function openPlaylistSelection(page: Page) {
  await page.goto('/');
  await page.fill('#queue-url-input', 'https://example.com/playlist-small');
  await page.keyboard.press('Enter');
  await expect(page.getByRole('button', { name: 'Download all items' })).toBeVisible();
}

test('it applies a simple playlist range selection', async ({ page }) => {
  await openPlaylistSelection(page);

  await page.locator('#playlist-selection-start').fill('1');
  await page.locator('#playlist-selection-end').fill('1');
  await expect(page.getByText(/1 of 2 item/)).toBeVisible();

  await page.getByRole('button', { name: 'Download selected items' }).click();

  const cards = page.locator(ROW_SELECTOR);
  await expect(cards).toHaveCount(1);
  await expect(page.getByText('Item 1')).toBeVisible();
});
