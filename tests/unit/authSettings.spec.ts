import { flushPromises, mount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import CredentialsConfig from '../../src/components/authentication/CredentialsConfig.vue';
import { i18n } from '../../src/i18n';
import { useSettingsStore } from '../../src/stores/settings';
import { defaultAuthSettings } from '../../src/tauri/types/config';
import { installTauriMock } from '../utils/tauriMock';

describe('authentication settings', () => {
  it('defaults to automatic detection with browser logins on (owner decision 2026-11-12)', () => {
    expect(defaultAuthSettings.cookieBrowser).toBe('automatic');
    expect(defaultAuthSettings.alwaysUseBrowserLogins).toBe(true);
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
  it('saved accounts stay locked until the vault is enabled; a failure shows its error and Retry', async () => {
    const results = [{ unlocked: false, initError: 'Failed to initialize' }, { unlocked: true }];
    installTauriMock({ stronghold_init: () => results.shift(), stronghold_keys: () => [] });
    const fields = { username: null, password: null, videoPassword: null, bearer: null, headers: null };
    const wrapper = mount(CredentialsConfig, { props: { modelValue: fields }, global: { plugins: [i18n] } });
    const alert = () => wrapper.find('[data-testid="auth-init-alert"]');
    const button = (label: string) => alert().findAll('button').find(b => b.text() === label);

    expect(wrapper.find('#basic-auth-username').exists()).toBe(false);
    await button('Enable')!.trigger('click');
    await flushPromises();
    expect(alert().text()).toContain('Failed to initialize');
    await button('Retry')!.trigger('click');
    await flushPromises();
    expect(alert().exists()).toBe(false);
    expect(wrapper.find('#basic-auth-username').exists()).toBe(true);
    wrapper.unmount();
  });
});
