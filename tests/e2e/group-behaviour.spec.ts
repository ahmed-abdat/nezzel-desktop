import { expect, Page } from '@playwright/test';
import { test } from './utils/fixtures';
import { ROW_SELECTOR } from '../../src/helpers/domIds';

async function addPlaylistAndApply(page: Page, url: string) {
  await page.goto('/');
  await page.fill('#queue-url-input', url);
  await page.keyboard.press('Enter');
  await page.getByRole('dialog', { name: 'Download options' }).getByRole('button', { name: 'Download 50 videos', exact: true }).click();
}

test('it consolidates large playlists into a single group', async ({ page }) => {
  await addPlaylistAndApply(page, 'https://example.com/playlist-large');
  const cards = page.locator(ROW_SELECTOR);
  await expect(cards).toHaveCount(1);
  await expect(cards.getByText('Playlist')).toBeVisible();
});
