import { expect } from '@playwright/test';
import { test } from './utils/fixtures';

test('a pasted link opens the options sheet with the three plain choices', async ({ page }) => {
  await page.goto('/');
  await page.fill('#queue-url-input', 'https://example.com/video');
  await page.getByRole('button', { name: 'Download', exact: true }).click();
  const sheet = page.getByRole('dialog');
  await expect(sheet.getByText('Test Video')).toBeVisible();
  for (const choice of ['Best quality', 'Audio only', 'Smaller file']) {
    await expect(sheet.getByRole('radio', { name: new RegExp(choice) })).toBeVisible();
  }
  await page.keyboard.press('Escape');
  await expect(sheet).toBeHidden();
  await expect(page.locator('li.row').getByText('Test Video')).toBeVisible();
});

test('Cmd/Ctrl+V anywhere in the window pastes the copied link into the field', async ({ page, context }) => {
  await context.grantPermissions(['clipboard-read', 'clipboard-write']);
  await page.goto('/');
  await expect(page.locator('#queue-url-input')).toBeVisible();
  await page.evaluate(() => navigator.clipboard.writeText('https://example.com/pasted'));
  await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur());
  await page.keyboard.press('ControlOrMeta+V');
  await expect(page.locator('#queue-url-input')).toHaveValue('https://example.com/pasted');
});

test('Ctrl+V in the prefilled quick bar keeps one link, not a duplicate', async ({ page, context }) => {
  const url = 'https://www.youtube.com/watch?v=dQw4w9WgXcQ';
  await context.grantPermissions(['clipboard-read', 'clipboard-write']);
  await page.addInitScript((offer) => {
    window.E2E = { capture: { offer } };
  }, { url, site: 'youtube.com' });
  await page.goto('/?window=capture-bar');
  const field = page.locator('#capture-url');
  await expect(field).toHaveValue(url);
  await expect(field).toBeFocused();
  await page.evaluate(link => navigator.clipboard.writeText(link), url);
  await page.keyboard.press('ControlOrMeta+V');
  await expect(field).toHaveValue(url);
  // A different link replaces the prefilled one.
  await page.evaluate(() => navigator.clipboard.writeText('https://example.com/other'));
  await page.keyboard.press('ControlOrMeta+V');
  await expect(field).toHaveValue('https://example.com/other');
});
