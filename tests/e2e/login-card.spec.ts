import { expect } from '@playwright/test';
import { test } from './utils/fixtures';
import { ROW_SELECTOR } from '../../src/helpers/domIds';

/** The engine asks (media_login_needed); the row shows the card; "Not now" leaves a plain failure. */
test('a login request shows on its row and Not now turns it into a plain failure', async ({ page }) => {
  await page.goto('/');
  await page.fill('#queue-url-input', 'https://www.instagram.com/p/abc/');
  await page.keyboard.press('Enter');
  await expect(page.getByRole('dialog')).toBeVisible();
  await page.keyboard.press('Escape');

  await page.evaluate(async () => {
    // @ts-ignore
    const { useMediaGroupStore } = await import('/src/stores/media/group.ts');
    // @ts-ignore
    const { useMediaStateStore } = await import('/src/stores/media/state.ts');
    // @ts-ignore
    const { useLoginStore } = await import('/src/stores/login.ts');
    // @ts-ignore
    const { useMediaDiagnosticsStore } = await import('/src/stores/media/diagnostics.ts');
    const group = Object.values(useMediaGroupStore().groups)[0] as { id: string; items: Record<string, { id: string }> };
    const itemId = Object.keys(group.items)[0];
    useMediaStateStore().setState(itemId, 'fetching');
    useMediaDiagnosticsStore().processMediaDiagnosticPayload({
      id: itemId, groupId: group.id, code: 'signInRequired', level: 'error', component: 'Reddit',
      message: 'Account authentication is required', raw: 'ERROR: [Reddit] Account authentication is required. Use --cookies', timestamp: 0,
    });
    useLoginStore().receive({
      requestId: 'r1', id: itemId, groupId: group.id, host: 'www.instagram.com', reason: 'signInRequired', status: 'needed',
      candidates: [
        { id: 'chrome:/p/Default', browser: 'chrome', profile: 'Ahmed · Default', signedIn: true },
        { id: 'firefox:/p/x1.default-release', browser: 'firefox', profile: 'Ahmed.work', signedIn: false },
      ],
    });
  });

  const row = page.locator(ROW_SELECTOR);
  await expect(row.getByText('This Instagram post needs you to be logged in')).toBeVisible();
  await expect(row.getByRole('combobox')).toHaveText('Chrome (Ahmed · Default) · logged in to www.instagram.com');
  await row.getByRole('combobox').click();
  await expect(page.getByRole('option', { name: 'Firefox (Ahmed.work)' })).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(row.getByRole('button', { name: 'Use it' })).toBeVisible();
  await row.getByRole('button', { name: 'Details', exact: true }).click();
  await expect(row.getByTestId('login-details')).toHaveText('ERROR: [Reddit] Account authentication is required. Use --cookies');

  await row.getByRole('button', { name: 'Not now' }).click();
  await expect(row.getByTestId('login-needed')).toBeHidden();
  await expect(row.getByTestId('login-failed')).toBeVisible();
  await expect(row.getByText('You skipped the login, so Instagram didn\'t allow this download')).toBeVisible();
  await expect(row.getByRole('button', { name: 'Log in again' })).toBeVisible();
  await expect(row.getByRole('button', { name: 'Try again' })).toHaveCount(0);
  await expect(row.getByRole('button', { name: 'Details', exact: true })).toBeVisible();
  await expect(row.getByTestId('login-details')).toBeVisible();
});
