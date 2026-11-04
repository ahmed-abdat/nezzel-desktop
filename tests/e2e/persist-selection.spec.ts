import { expect } from '@playwright/test';
import { test } from './utils/fixtures';

test('it keeps an advanced format choice after viewing details', async ({ page }) => {
  await page.goto('/');
  await page.fill('#queue-url-input', 'https://example.com/video');
  await page.keyboard.press('Enter');
  const sheet = page.getByRole('dialog');
  await sheet.getByText('All formats and extras').click();
  await sheet.getByRole('combobox', { name: 'Select track to download' }).click();
  await page.getByRole('option', { name: 'Audio', exact: true }).click();
  await page.keyboard.press('Escape');

  const row = page.locator('li.row').first();
  await row.getByRole('button', { name: 'More actions' }).click();
  await page.getByRole('menuitem', { name: 'Details' }).click();
  await expect(page.getByText('Technical details')).toBeVisible();
  await page.goBack();

  await page.locator('li.row').first().getByRole('button', { name: 'Options' }).click();
  await sheet.getByText('All formats and extras').click();
  await expect(sheet.getByRole('combobox', { name: 'Select track to download' })).toHaveText('Audio');
});

test('it keeps carousel selections when reopening options and requires at least one item', async ({ page }) => {
  await page.goto('/');
  await page.fill('#queue-url-input', 'https://example.com/image-carousel');
  await page.keyboard.press('Enter');
  const sheet = page.getByRole('dialog');
  await expect(sheet.getByLabel('Photo 2', { exact: true })).toBeChecked();
  await sheet.getByLabel('Photo 2', { exact: true }).uncheck();
  await sheet.getByRole('button', { name: 'Not now', exact: true }).click();
  await page.locator('li.row').first().getByRole('button', { name: 'Options', exact: true }).click();
  await expect(sheet.getByLabel('Photo 2', { exact: true })).not.toBeChecked();
  await sheet.getByLabel('Select all', { exact: true }).check();
  await expect(sheet.getByLabel('Photo 2', { exact: true })).toBeChecked();
  await sheet.getByLabel('Select all', { exact: true }).uncheck();
  await expect(sheet.getByRole('button', { name: 'Download', exact: true })).toBeDisabled();
  await sheet.getByLabel('Photo 1', { exact: true }).check();
  await expect(sheet.getByRole('button', { name: 'Download', exact: true })).toBeEnabled();
});
