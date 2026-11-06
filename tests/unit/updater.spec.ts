import { flushPromises, mount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import TheUpdater from '../../src/components/TheUpdater.vue';
import { useUpdaterStore } from '../../src/stores/updater';
import { useSettingsStore } from '../../src/stores/settings';
import { i18n } from '../../src/i18n';
import { installTauriMock } from '../utils/tauriMock';

const card = '[data-testid="updater-alert"]';

async function mountAfterCheck(available: boolean) {
  installTauriMock({ updater_check: () => ({ available, currentVersion: '1.2.2', availableVersion: available ? '1.2.3' : undefined }) });
  const wrapper = mount(TheUpdater, { global: { plugins: [i18n] } });
  await useUpdaterStore().check();
  await flushPromises();
  return wrapper;
}

describe('update card', () => {
  it('shows an available update until Later is clicked', async () => {
    const wrapper = await mountAfterCheck(true);
    expect(wrapper.find(card).exists()).toBe(true);
    await wrapper.findAll('button').find(b => b.text() === 'Later')!.trigger('click');
    expect(wrapper.find(card).exists()).toBe(false);
    wrapper.unmount();
  });

  it.each([
    ['there is no update', false, true],
    ['the background check is switched off', true, false],
  ])('shows nothing when %s', async (_case, available, updateApp) => {
    useSettingsStore().settings.update.updateApp = updateApp;
    const wrapper = await mountAfterCheck(available);
    expect(wrapper.find(card).exists()).toBe(false);
    wrapper.unmount();
  });
});
