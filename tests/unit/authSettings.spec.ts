import { describe, expect, it } from 'vitest';
import { useSettingsStore } from '../../src/stores/settings';
import { defaultAuthSettings } from '../../src/tauri/types/config';

describe('authentication settings', () => {
  it('defaults to automatic detection without forcing browser logins on public links', () => {
    expect(defaultAuthSettings.cookieBrowser).toBe('automatic');
    expect(defaultAuthSettings.alwaysUseBrowserLogins).toBe(false);
  });
  it('only marks explicit cookie overrides as configured', () => {
    const store = useSettingsStore();
    store.settings.auth = structuredClone(defaultAuthSettings);
    store.settings.auth.cookieBrowser = 'automatic';
    expect(store.hasAuthConfigured()).toBe(false);
    store.settings.auth.cookieBrowser = 'none';
    expect(store.hasAuthConfigured()).toBe(false);
    store.settings.auth.cookieBrowser = 'chrome';
    expect(store.hasAuthConfigured()).toBe(true);
    store.settings.auth.cookieBrowser = 'automatic';
    store.settings.auth.cookieFile = '/tmp/manual-cookies.txt';
    expect(store.hasAuthConfigured()).toBe(true);
  });
});
