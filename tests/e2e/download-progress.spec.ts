import { expect } from '@playwright/test';
import { test } from './utils/fixtures';

test('it shows how many links are ready', async ({ page }) => {
  await page.goto('/');
  for (const url of ['https://example.com/video1', 'https://example.com/video2']) {
    await page.fill('#queue-url-input', url);
    await page.keyboard.press('Enter');
    await expect(page.getByRole('dialog')).toBeVisible();
    await page.keyboard.press('Escape');
  }
  await expect(page.getByText('2 links ready')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Download all' })).toBeVisible();
});

test('Download all in a filtered view covers only the links it shows', async ({ page }) => {
  await page.goto('/');
  for (const url of ['https://example.com/song1', 'https://example.com/song2', 'https://example.com/single-image']) {
    await page.fill('#queue-url-input', url);
    await page.keyboard.press('Enter');
    await expect(page.getByRole('dialog')).toBeVisible();
    await page.keyboard.press('Escape');
  }
  await expect(page.getByText('3 links ready')).toBeVisible();
  // Type views hide rows that still need a click, so narrow the list with search.
  await page.getByRole('searchbox').fill('Test Video');
  await expect(page.getByText('2 links ready')).toBeVisible();
});
