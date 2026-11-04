import { expect } from '@playwright/test';
import { test } from './utils/fixtures';

test('the choice made in the sheet is stored on the item', async ({ page }) => {
  await page.goto('/');
  await page.fill('#queue-url-input', 'https://example.com/video1');
  await page.keyboard.press('Enter');
  const sheet = page.getByRole('dialog');
  await expect(sheet.getByRole('radio', { name: /Best quality/ })).toHaveAttribute('aria-checked', 'true');
  await sheet.getByRole('radio', { name: /Smaller file/ }).click();
  await page.keyboard.press('Escape');

  const row = page.locator('li.row').first();
  await expect(row).toContainText('720p');
  await row.getByRole('button', { name: 'Options' }).click();
  await expect(sheet.getByRole('radio', { name: /Smaller file/ })).toHaveAttribute('aria-checked', 'true');
});
