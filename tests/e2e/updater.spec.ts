import { expect } from '@playwright/test';
import { test } from './utils/fixtures';

// Show, Later and "no update" are covered in tests/unit/updater.spec.ts; this is the one real journey.
test.describe('when it has an update available', () => {
  test.use({
    mockData: [{
      updater: {
        available: true,
        availableVersion: '1.2.3',
        currentVersion: '1.2.2',
      },
    }, { scope: 'test' }],
  });

  test('it downloads with progress and installs after confirmation', async ({ page }) => {
    await page.clock.install({ time: new Date('2025-09-15T21:30:00') });

    await page.goto('/');
    const updateAlert = page.getByTestId('updater-alert');

    await page.clock.pauseAt(new Date('2025-09-15T21:45:00'));
    await updateAlert.getByRole('button', { name: 'Download' }).click();
    await page.clock.runFor(1000);
    await expect(page.getByText('50%')).toBeVisible();
    await page.clock.runFor(1000);
    await expect(page.getByText('100%')).toBeVisible();
    await page.clock.runFor(1000);
    await page.clock.resume();

    await updateAlert.getByRole('button', { name: 'Install' }).click();
    await expect(updateAlert).not.toBeVisible();
  });
});
