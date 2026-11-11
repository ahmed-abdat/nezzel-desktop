import { expect, Page } from '@playwright/test';
import { test } from './utils/fixtures';
import { ROW_SELECTOR } from '../../src/helpers/domIds';

async function openPlaylistSelection(page: Page) {
  await page.goto('/');
  await page.fill('#queue-url-input', 'https://example.com/playlist-small');
  await page.keyboard.press('Enter');
  const dialog = page.getByRole('dialog', { name: 'Download options' });
  await expect(dialog.getByRole('button', { name: 'Download 2 videos', exact: true })).toBeVisible();
  return dialog;
}

test('it expands only the checked playlist items', async ({ page }) => {
  const dialog = await openPlaylistSelection(page);
  await dialog.getByRole('region', { name: 'Choose playlist items' }).getByRole('checkbox').nth(1).click();
  await expect(dialog.getByText(/1 of 2 item/)).toBeVisible();
  await dialog.getByRole('button', { name: 'Download 1 video', exact: true }).click();
  await expect(dialog).toBeHidden();

  const cards = page.locator(ROW_SELECTOR);
  await expect(cards).toHaveCount(1);
  await expect(page.getByText('Item 1')).toBeVisible();
});

test('only the sheet contains a picker and keeps checks when reopened', async ({ page }) => {
  const sheet = await openPlaylistSelection(page);
  const row = page.locator(ROW_SELECTOR).first();
  await sheet.getByRole('region').getByRole('checkbox').nth(1).click();
  await expect(row.getByRole('region')).toHaveCount(0);
  await expect(row.getByText('1 of 2 item selected')).toBeVisible();
  await sheet.getByRole('button', { name: 'Not now', exact: true }).click();
  await expect(row.getByRole('region', { name: 'Choose playlist items' })).toHaveCount(0);
  await row.getByRole('button', { name: 'Options', exact: true }).click();
  await expect(sheet.getByRole('region').getByRole('checkbox').nth(1)).not.toBeChecked();
  await expect(sheet.getByRole('button', { name: /^Download/ })).toHaveCount(1);
  await sheet.getByRole('button', { name: 'Select none', exact: true }).click();
  await expect(sheet.getByRole('button', { name: 'Download 0 videos', exact: true })).toBeDisabled();
  await sheet.getByRole('region').focus();
  await page.keyboard.press('ControlOrMeta+a');
  await expect(sheet.getByRole('button', { name: 'Download 2 videos', exact: true })).toBeEnabled();
  await sheet.getByRole('listitem').first().focus();
  await page.keyboard.press('Space');
  await expect(sheet.getByRole('button', { name: 'Download 1 video', exact: true })).toBeEnabled();
});
