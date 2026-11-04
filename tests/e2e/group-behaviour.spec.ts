import { expect, Page } from '@playwright/test';
import { test } from './utils/fixtures';

async function addPlaylistAndApply(page: Page, url: string) {
  await page.goto('/');
  await page.fill('#queue-url-input', url);
  await page.keyboard.press('Enter');
  await page.getByRole('button', { name: 'Download all items' }).click();
}

test('it consolidates large playlists into a single group', async ({ page }) => {
  await addPlaylistAndApply(page, 'https://example.com/playlist-large');
  const cards = page.locator('li.row');
  await expect(cards).toHaveCount(1);
  await expect(cards.getByText('Playlist')).toBeVisible();
});
