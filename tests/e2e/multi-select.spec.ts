import { expect, Page } from '@playwright/test';
import { test } from './utils/fixtures';

/** Set SELECT_SHOTS=<dir> to also save screenshots for a visual check. */
const SHOTS = process.env.SELECT_SHOTS;

async function addUrl(page: Page, url: string) {
  await page.fill('#queue-url-input', url);
  await page.keyboard.press('Enter');
  await expect(page.getByRole('dialog')).toBeVisible();
  await page.keyboard.press('Escape');
}

async function setLocale(page: Page, locale: 'en' | 'ar') {
  await page.evaluate(async (l) => {
    // @ts-ignore
    const { i18n, applyDocumentLocale } = await import('/src/i18n.ts');
    i18n.global.locale.value = l;
    applyDocumentLocale(l);
  }, locale);
}

const copy = {
  en: { count: (n: number) => `${n} selected`, remove: 'Remove from list', download: 'Download', retry: 'Try again', pause: 'Pause', deleteFiles: 'Delete files…' },
  ar: { count: (n: number) => (n === 1 ? 'عنصر واحد محدد' : `المحدد: ${n}`), remove: 'إزالة من القائمة', download: 'تنزيل', retry: 'أعد المحاولة', pause: 'إيقاف مؤقت', deleteFiles: 'حذف الملفات…' },
};

for (const locale of ['en', 'ar'] as const) {
  test(`multi-select journey (${locale})`, async ({ page }) => {
    const t = copy[locale];
    await page.goto('/');
    for (let i = 1; i <= 4; i++) await addUrl(page, `https://example.com/video-select-${i}`);
    const rows = page.locator('li.row');
    await expect(rows).toHaveCount(4);

    // Two ready, one failed, one finished.
    await page.evaluate(async () => {
      // @ts-ignore
      const { useMediaGroupStore } = await import('/src/stores/media/group.ts');
      // @ts-ignore
      const { useMediaStateStore, MediaState } = await import('/src/stores/media/state.ts');
      const ids = Object.keys(useMediaGroupStore().groups);
      useMediaStateStore().setGroupState(ids[2], MediaState.error);
      useMediaStateStore().setGroupState(ids[3], MediaState.done);
    });
    await setLocale(page, locale);
    await expect(page.locator('html')).toHaveAttribute('dir', locale === 'ar' ? 'rtl' : 'ltr');
    await expect(rows).toHaveCount(4);

    const bar = page.getByRole('toolbar');
    const mod = 'Control';
    // Ctrl+click is a context-menu gesture on macOS, so dispatch the click event itself.
    const clickRow = (i: number, modifiers: ('Control' | 'Shift')[]) =>
      modifiers.includes('Control')
        ? rows.nth(i).dispatchEvent('click', { ctrlKey: true, bubbles: true })
        : rows.nth(i).click({ modifiers, position: { x: 400, y: 4 } });
    const selectedRows = page.locator('li.row[aria-selected="true"]');

    await expect(bar).toHaveCount(0);
    // The options panel from the last add must be gone: its close widens the list past the column
    // breakpoint and the header row shifts every row under the pointer.
    await expect(page.getByRole('dialog')).toHaveCount(0);
    await rows.nth(1).hover();
    await expect(rows.nth(1).getByRole('checkbox')).toHaveCSS('opacity', '1');
    if (SHOTS) await page.screenshot({ path: `${SHOTS}/${locale}-hover.png` });

    // Failed rows sort first: rows 0 (failed), 1-2 (ready), 3 (finished).
    // Ctrl+click rows 0 and 1 (failed + ready): Download and Retry, no Pause, no Delete files.
    await clickRow(1, [mod]);
    await clickRow(0, [mod]);
    await expect(bar).toContainText(t.count(2));
    await expect(bar.getByRole('button', { name: t.download })).toBeVisible();
    await expect(bar.getByRole('button', { name: t.retry })).toBeVisible();
    await expect(bar.getByRole('button', { name: t.pause })).toHaveCount(0);
    await expect(bar.getByRole('button', { name: t.deleteFiles })).toHaveCount(0);

    // Shift+click extends from the anchor (row 0) to row 3, then back to row 2.
    await clickRow(3, ['Shift']);
    await expect(bar).toContainText(t.count(4));
    await clickRow(2, ['Shift']);
    await expect(bar).toContainText(t.count(3));
    await expect(selectedRows).toHaveCount(3);
    await expect(bar.getByRole('button', { name: t.retry })).toBeVisible();
    if (SHOTS) {
      await page.mouse.move(600, 600);
      await expect(selectedRows).toHaveCount(3);
      await expect(bar).toBeVisible();
      await page.screenshot({ path: `${SHOTS}/${locale}-selected.png` });
      const bgs = await rows.evaluateAll(els => els.map(e => getComputedStyle(e).backgroundColor));
      console.log(locale, JSON.stringify(bgs));
    }

    // Esc clears; then select two and batch remove.
    await page.keyboard.press('Escape');
    await expect(bar).toHaveCount(0);
    await expect(selectedRows).toHaveCount(0);

    await clickRow(1, [mod]);
    await clickRow(2, [mod]);
    await expect(bar).toContainText(t.count(2));
    await bar.getByRole('button', { name: t.remove }).click();
    await expect(rows).toHaveCount(2);
    await expect(bar).toHaveCount(0);
  });
}
