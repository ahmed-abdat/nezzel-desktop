import { expect } from '@playwright/test';
import { test } from './utils/fixtures';
import { MAIN_PANE_ID } from '../../src/helpers/domIds';

const openSettings = async (page: import('@playwright/test').Page, state = 'settings-about') => {
  await page.goto('/');
  await expect(page.locator(`#${MAIN_PANE_ID}`)).toBeVisible({ timeout: 20_000 });
  await page.evaluate(async (state) => {
    // @ts-expect-error served by the Vite dev server, not resolved by tsc
    const { seed } = await import('/tests/e2e/utils/uiStateSeeds.ts');
    await seed(state);
  }, state);
};

test('Report a problem: an optional note and one Send button', async ({ page }) => {
  await openSettings(page);
  await page.getByTestId('about-report').click();
  const dialog = page.getByTestId('report-dialog');
  await expect(dialog.getByRole('heading')).toHaveText('Report a problem');
  // No link on a general report: nothing to include.
  await expect(dialog.getByTestId('report-include-link')).toHaveCount(0);

  await dialog.getByTestId('report-note').fill('التنزيل توقف');
  await dialog.getByTestId('report-send').click();
  await expect(dialog).toBeHidden();
  await expect(page.getByText('Thanks, your report was sent')).toBeVisible();
  expect(await page.evaluate(() => window.E2E?.reports)).toEqual([
    { note: 'التنزيل توقف', download: null, url: null, extractor: null, includeLink: true },
  ]);
});

test('a failed row shows Details and a Report control without opening a menu', async ({ page }) => {
  await page.goto('/');
  await expect(page.locator(`#${MAIN_PANE_ID}`)).toBeVisible({ timeout: 20_000 });
  await page.evaluate(async () => {
    // @ts-expect-error served by the Vite dev server, not resolved by tsc
    const { seed } = await import('/tests/e2e/utils/uiStateSeeds.ts');
    await seed('row-error-unknown');
  });
  const links = page.getByTestId('row-error-links');
  await links.getByTestId('row-error-details').click();
  await expect(links.getByTestId('row-error-raw')).toBeVisible();
  await links.getByTestId('row-error-report').click();
  await expect(page.getByTestId('report-dialog')).toBeVisible();
});

test.describe('without a report key in the build', () => {
  test.use({ mockData: { reportSend: 'fail' } });

  test('a failed send keeps the form and offers Copy details', async ({ page }) => {
    await openSettings(page);
    await page.getByTestId('about-report').click();
    await page.getByTestId('report-send').click();
    await expect(page.getByText('Couldn\'t send the report')).toBeVisible();
    await expect(page.getByRole('button', { name: 'Copy details' })).toBeVisible();
    await expect(page.getByTestId('report-dialog')).toBeVisible();
  });
});

test.describe('after a crash with reports off', () => {
  test.use({ mockData: { crash: 'pending' } });

  test('the next launch offers to send it once', async ({ page }) => {
    await page.goto('/');
    await expect(page.getByText('Nezzel closed unexpectedly last time')).toBeVisible({ timeout: 20_000 });
    await expect(page.getByRole('button', { name: 'Send report' })).toBeVisible();
    expect(await page.evaluate(() => window.E2E?.crash)).toBeUndefined();
  });
});

test('Settings has the reports switch, on by default', async ({ page }) => {
  await openSettings(page, 'settings-advanced');
  await expect(page.getByTestId('settings-send-reports')).toHaveAttribute('data-state', 'checked');
});
