import { expect } from '@playwright/test';
import { test } from './utils/fixtures';

test.use({ mockData: { preferences: { onboarding: { welcomeDone: false, closeNoteDone: false } } } });

test('fresh profile: welcome, Get started, then the main list', async ({ page, context }) => {
  await context.grantPermissions(['clipboard-read', 'clipboard-write']);
  await page.goto('/');
  const welcome = page.getByTestId('welcome');
  // The first page of a cold dev server compiles every module.
  await expect(welcome).toBeVisible({ timeout: 20_000 });
  await expect(welcome).toHaveAccessibleName('Save videos, audio, photos and files from any link');
  await expect(welcome).toBeFocused();
  await expect(welcome.getByRole('switch', { name: 'Show a card when I copy a link' })).toBeChecked();
  await expect(welcome.getByRole('switch', { name: 'Start with my computer' })).toBeChecked();
  await expect(welcome.getByTestId('welcome-shortcut')).toContainText('Copy a link anywhere, then press');

  // Try it writes the sample link; it never reads the clipboard.
  await welcome.getByTestId('welcome-copy').click();
  await expect(welcome.getByTestId('welcome-copy')).toHaveText('Copied');
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe('https://www.youtube.com/watch?v=jNQXAC9IVRw');

  // With the card off there is nothing to try.
  await welcome.getByRole('switch', { name: 'Show a card when I copy a link' }).click();
  await expect(welcome.getByTestId('welcome-copy')).toBeDisabled();

  await page.getByTestId('welcome-start-button').click();
  await expect(welcome).toBeHidden();
  await expect(page.locator('#queue-url-input')).toBeVisible();
});

test('keyboard: Tab goes through the switches to Get started, with a visible focus ring', async ({ page }) => {
  await page.goto('/');
  const welcome = page.getByTestId('welcome');
  await expect(welcome).toBeFocused({ timeout: 20_000 });
  const order: string[] = [];
  for (let i = 0; i < 8 && !order.includes('welcome-start-button'); i++) {
    await page.keyboard.press('Tab');
    order.push(await page.evaluate(() => (document.activeElement as HTMLElement).dataset.testid ?? ''));
  }
  expect(order).toEqual(['welcome-card', 'welcome-start', 'welcome-reports', 'welcome-start-button']);
  const start = page.getByTestId('welcome-start-button');
  await expect(start).toBeFocused();
  // The ring is the primary hue: without an offset it melts into the button.
  expect(await start.evaluate(el => getComputedStyle(el).getPropertyValue('--tw-ring-offset-width').trim())).toBe('2px');
  await page.keyboard.press('Enter');
  await expect(welcome).toBeHidden();
});
