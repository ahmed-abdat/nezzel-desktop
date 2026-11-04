import { expect, Page } from '@playwright/test';
import { test } from './utils/fixtures';

async function addUrl(page: Page, url: string) {
  await page.fill('#queue-url-input', url);
  await page.keyboard.press('Enter');
  await expect(page.getByRole('dialog')).toBeVisible();
  await page.keyboard.press('Escape');
}

test('status filters, search, remove from list and clear finished with undo', async ({ page }) => {
  await page.goto('/');
  for (let i = 1; i <= 3; i++) await addUrl(page, `https://example.com/video-clear-${i}`);
  const rows = page.locator('li.row');
  await expect(rows).toHaveCount(3);

  await page.evaluate(async () => {
    // @ts-ignore
    const { useMediaGroupStore } = await import('/src/stores/media/group.ts');
    // @ts-ignore
    const { useMediaStateStore, MediaState } = await import('/src/stores/media/state.ts');
    // @ts-ignore
    const { useMediaDestinationStore } = await import('/src/stores/media/destination.ts');
    const [doneId, errorId, activeId] = Object.keys(useMediaGroupStore().groups);
    useMediaDestinationStore().addDestination(doneId, { path: '/Downloads/video-clear-1.mp4', confidence: 100 });
    useMediaStateStore().setGroupState(doneId, MediaState.done);
    useMediaStateStore().setGroupState(errorId, MediaState.error);
    useMediaStateStore().setGroupState(activeId, MediaState.downloading);
  });

  // A running download stays running: rendering its row must not pause it.
  await expect(page.getByRole('button', { name: 'Pause', exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Resume', exact: true })).toHaveCount(0);

  await expect(page.getByRole('heading', { name: 'In progress' })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Completed' })).toBeVisible();

  // Status chips above the list (the sidebar holds types), with counts.
  const chips = page.getByRole('group', { name: 'Show' });
  await chips.getByRole('link', { name: /^Failed/ }).click();
  await expect(rows).toHaveCount(1);
  await chips.getByRole('link', { name: 'All', exact: true }).click();

  // Search narrows the list; Esc clears it.
  const search = page.getByRole('searchbox', { name: 'Search downloads' });
  await search.fill('no such title');
  await expect(rows).toHaveCount(0);
  await search.press('Escape');
  await expect(rows).toHaveCount(3);

  const errorRow = rows.filter({ hasText: 'Try again' });
  await errorRow.getByRole('button', { name: 'More actions' }).click();
  await page.getByRole('menuitem', { name: 'Remove from list' }).click();
  await expect(rows).toHaveCount(2);

  // "Remove from list" takes done rows off the list (files and history stay); Undo brings them back.
  const removeDone = page.getByRole('button', { name: 'Remove from list' });
  await removeDone.click();
  await expect(rows).toHaveCount(1);
  await page.getByRole('button', { name: 'Undo' }).last().click();
  await expect(rows).toHaveCount(2);
  await removeDone.click();
  await expect(rows).toHaveCount(1);

  // Removed rows are still under Completed.
  await chips.getByRole('link', { name: /^Completed/ }).click();
  await expect(rows).toHaveCount(1);
  await expect(page.getByRole('button', { name: 'Remove from list' })).toHaveCount(0);
  await chips.getByRole('link', { name: 'All', exact: true }).click();
  await expect(rows).toHaveCount(1);

  // Removing the last row keeps the list (not onboarding) on screen while Undo is open.
  await rows.filter({ has: page.getByRole('button', { name: 'Pause', exact: true }) }).getByRole('button', { name: 'More actions' }).click();
  await page.getByRole('menuitem', { name: 'Remove from list' }).click();
  await expect(rows).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Undo' }).last()).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Save anything from a link' })).toHaveCount(0);
});
