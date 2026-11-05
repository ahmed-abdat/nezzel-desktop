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
  // The focused button went away with the sheet: focus lands on the main content, not the page body.
  await expect(page.locator('#nz-main')).toBeFocused();
});

// #36, Fedora KDE Wayland 1280×800: 704 is the window that fits above the panel, 600 a smaller screen.
for (const height of [704, 600]) {
  test.describe(`1280×${height} window`, () => {
    test.use({ viewport: { width: 1280, height } });

    test('the window shows only once the page is drawn; every Tab stop is outlined, unclipped and in view', async ({ page, browserName }) => {
      // Mac WebKit tabs only to text fields; Option+Tab reaches every control, as Tab does in WebKitGTK.
      const tab = browserName === 'webkit' ? 'Alt+Tab' : 'Tab';
      await page.goto('/');
      const welcome = page.getByTestId('welcome');
      await expect(welcome).toBeFocused({ timeout: 20_000 });
      // Shown with an empty page, WebKitGTK kept the window grey until a click.
      await expect.poll(() => page.evaluate(() => (window as Window & { __appReadyRendered?: boolean }).__appReadyRendered)).toBe(true);
      await expect(page.getByTestId('welcome-start-button')).toBeInViewport({ ratio: 1 });
      // Mouse first, then programmatic focus: the path where WebKit left :focus-visible unmatched.
      await page.locator('#welcome-title').click();
      await welcome.focus();

      type Stop = { name: string; problems: string[] };
      const stops: Stop[] = [];
      for (let i = 0; i < 20; i++) {
        await page.keyboard.press(tab);
        const stop = await page.evaluate((): Stop => {
          const el = document.activeElement as HTMLElement;
          const name = el.dataset.testid || el.id || el.className.toString().split(' ')[0] || el.tagName;
          const problems: string[] = [];
          if (!el.closest('[data-testid="welcome"]') || el.dataset.testid === 'welcome') return { name: `outside:${el.tagName}`, problems: ['focus left the sheet'] };
          const cs = getComputedStyle(el);
          const width = parseFloat(cs.outlineWidth);
          if (cs.outlineStyle !== 'solid' || width < 2 || /rgba\(0, 0, 0, 0\)|transparent/.test(cs.outlineColor)) problems.push(`outline ${cs.outlineStyle} ${cs.outlineWidth} ${cs.outlineColor}`);
          // The ring's box: the element grown by offset + width. Each clipping ancestor must contain it.
          const grow = width + Math.max(0, parseFloat(cs.outlineOffset) || 0);
          const r = el.getBoundingClientRect();
          const ring = { l: r.left - grow, t: r.top - grow, r: r.right + grow, b: r.bottom + grow };
          const inside = (c: { left: number; top: number; right: number; bottom: number }) => ring.l >= c.left - 0.5 && ring.t >= c.top - 0.5 && ring.r <= c.right + 0.5 && ring.b <= c.bottom + 0.5;
          if (!inside({ left: 0, top: 0, right: innerWidth, bottom: innerHeight })) problems.push('ring outside the window');
          for (let a = el.parentElement; a; a = a.parentElement) {
            const s = getComputedStyle(a);
            const clips = s.overflowX !== 'visible' || s.overflowY !== 'visible' || /paint|strict|content/.test(s.contain) || s.clipPath !== 'none';
            if (clips && a !== document.documentElement && a !== document.body && !inside(a.getBoundingClientRect())) problems.push(`clipped by ${a.tagName}.${a.classList[0] ?? ''}`);
          }
          return { name, problems };
        });
        if (stops.some(s => s.name === stop.name)) break;
        stops.push(stop);
      }
      expect(stops.map(s => s.name)).toEqual(['welcome-card', 'welcome-start', 'welcome-reports', 'welcome-start-button', 'welcome-replay', 'wl-link', 'welcome-copy']);
      expect(stops.filter(s => s.problems.length)).toEqual([]);
      // Shift+Tab from the first stop wraps to the last, inside the sheet.
      await page.getByTestId('welcome-card').focus();
      await page.keyboard.press(`Shift+${tab}`);
      await expect(page.getByTestId('welcome-copy')).toBeFocused();
      await expect(welcome).toHaveAttribute('data-keyboard', 'true');
      await page.locator('#welcome-title').click();
      await expect(welcome).not.toHaveAttribute('data-keyboard');
    });
  });
}
