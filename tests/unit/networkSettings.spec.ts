import { mount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import { nextTick } from 'vue';
import SettingsNetwork from '../../src/components/settings/SettingsNetwork.vue';
import { i18n } from '../../src/i18n';
import { defaultSettings } from '../../src/tauri/types/config';
import { pickOption as pick } from '../utils/selectField';

describe('network settings', () => {
  it('shows the proxy address only when the proxy is on, and binds the trimmed model', async () => {
    const settings = structuredClone(defaultSettings);
    const wrapper = mount(SettingsNetwork, {
      props: { modelValue: settings },
      global: { plugins: [i18n], stubs: { RouterLink: true } },
    });

    expect(wrapper.find('#proxyUrl').exists()).toBe(false);

    await wrapper.get('#enableProxy').trigger('click');
    await nextTick();
    await wrapper.get('#proxyUrl').setValue('  socks5://127.0.0.1:1080/  ');
    await pick(wrapper, 'impersonate', 'any');

    expect(settings.network.enableProxy).toBe(true);
    expect(settings.network.proxy).toBe('socks5://127.0.0.1:1080/');
    expect(settings.network.impersonate).toBe('any');
    expect(wrapper.get('#proxyUrl').attributes('aria-invalid')).toBeUndefined();

    await wrapper.get('#proxyUrl').setValue('127.0.0.1:1080');
    expect(wrapper.get('#proxyUrl').attributes('aria-invalid')).toBe('true');
  });
});
