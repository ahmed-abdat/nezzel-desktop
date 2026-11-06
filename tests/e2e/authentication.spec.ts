import { expect, Locator, Page } from '@playwright/test';
import { test } from './utils/fixtures';

const getAuthDetails = async (page: Page): Promise<Locator> => {
  await page.goto('/authentication'); // old route: redirects to Settings > Logins
  const authDetails = page.getByTestId('auth-details');
  await expect(authDetails).toBeVisible();
  return authDetails;
};

// Enabling the vault, its error and Retry: tests/unit/authSettings.spec.ts. The browser choice autosave:
// tests/unit/settingsView.spec.ts (Logins change saved as the full snapshot).
test.describe('when authentication is initialized', () => {
  const headersValue = 'TEST:1';

  test.use({
    mockData: [{
      stronghold: {
        fields: {
          'auth.headers': headersValue,
        },
        status: {
          unlocked: true,
          initError: undefined,
        },
      },
    }, { scope: 'test' }],
  });
  test('it allows authentication secrets to be managed', async ({ page }) => {
    const authDetails = await getAuthDetails(page);
    // A stored value opens the section by itself.
    await expect(authDetails).toHaveAttribute('data-state', 'open');
    const headerInput = authDetails.getByRole('textbox', { name: 'Headers' });
    await expect(headerInput).toHaveValue(headersValue);
    const bearerInput = authDetails.getByRole('textbox', { name: 'Token' });
    const testValue = 'test2';
    await bearerInput.fill(testValue);
    const saveButton = page.getByRole('button', { name: 'Save', exact: true });
    await expect(saveButton).toBeEnabled();
    // Verify keyboard accessibility of BaseSecretInput
    await expect(bearerInput).toHaveAttribute('type', 'password');
    // Reveal the value
    await page.keyboard.press('Tab');
    await page.keyboard.press('Enter');
    await expect(bearerInput).toHaveAttribute('type', 'text');
    // Clear the value
    await page.keyboard.press('Tab');
    await page.keyboard.press('Enter');
    // This should revert the change and disable the save button.
    await expect(bearerInput).not.toHaveValue(testValue);
    await expect(saveButton).toBeDisabled();
    await bearerInput.fill(testValue);
    await saveButton.click();
    // After saving, there should be no more changes.
    await expect(saveButton).toBeDisabled();
  });
  test('a cleared value is deleted on save and stays gone after a reload', async ({ page }) => {
    const authDetails = await getAuthDetails(page);
    const headerInput = authDetails.getByRole('textbox', { name: 'Headers' });
    await headerInput.fill('');
    const saveButton = page.getByRole('button', { name: 'Save', exact: true });
    await saveButton.click();
    await expect(saveButton).toBeDisabled();

    await page.reload();
    await expect(authDetails).toBeVisible();
    // Nothing stored any more, so the section starts closed.
    await expect(authDetails).not.toHaveAttribute('data-state', 'open');
    await authDetails.locator('[data-slot=collapsible-trigger]').click();
    await expect(headerInput).toHaveValue('');
  });
});
