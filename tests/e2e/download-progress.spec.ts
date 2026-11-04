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
  for (const url of ['https://example.com/song1', 'https://example.com/song2', 'https://example.com/clip']) {
    await page.fill('#queue-url-input', url);
    await page.keyboard.press('Enter');
    await expect(page.getByRole('dialog')).toBeVisible();
    await page.keyboard.press('Escape');
  }
  await page.evaluate(async () => {
    // @ts-ignore
    const { useMediaGroupStore } = await import('/src/stores/media/group.ts');
    // @ts-ignore
    const { useMediaOptionsStore } = await import('/src/stores/media/options.ts');
    const [song1, song2] = Object.keys(useMediaGroupStore().groups);
    for (const id of [song1, song2]) useMediaOptionsStore().setOptions(id, { trackType: 'audio', id: 'audio' });
  });
  await expect(page.getByText('3 links ready')).toBeVisible();
  await page.getByRole('link', { name: /Audio/ }).click();
  await expect(page.getByText('2 links ready')).toBeVisible();
});
