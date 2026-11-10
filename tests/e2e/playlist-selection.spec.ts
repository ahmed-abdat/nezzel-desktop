import { expect, Page } from '@playwright/test';
import { test } from './utils/fixtures';
import { ROW_SELECTOR } from '../../src/helpers/domIds';

async function openPlaylistSelection(page: Page) {
  await page.goto('/');
  await page.fill('#queue-url-input', 'https://example.com/playlist-small');
  await page.keyboard.press('Enter');
  const dialog = page.getByRole('dialog', { name: 'Download options' });
  await expect(dialog.getByRole('button', { name: 'Download all items' })).toBeVisible();
  return dialog;
}

test('it applies a simple playlist range selection', async ({ page }) => {
  const dialog = await openPlaylistSelection(page);

  await dialog.locator('#inspector-playlist-selection-start').fill('1');
  await dialog.locator('#inspector-playlist-selection-end').fill('1');
  await expect(dialog.getByText(/1 of 2 item/)).toBeVisible();

  await dialog.getByRole('button', { name: 'Download selected items' }).click();

  const cards = page.locator(ROW_SELECTOR);
  await expect(cards).toHaveCount(1);
  await expect(page.getByText('Item 1')).toBeVisible();
});
