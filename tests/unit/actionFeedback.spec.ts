import { flushPromises, mount } from '@vue/test-utils';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { createMemoryHistory, createRouter } from 'vue-router';
import { defineComponent } from 'vue';
import TheToaster from '../../src/components/TheToaster.vue';
import TheUpdater from '../../src/components/TheUpdater.vue';
import InstallView from '../../src/views/full/InstallView.vue';
import AppStatusBar from '../../src/components/shell/AppStatusBar.vue';
import App from '../../src/App.vue';
import { useUpdaterStore } from '../../src/stores/updater';
import { useBinariesStore } from '../../src/stores/binaries';
import { useSettingsStore } from '../../src/stores/settings';
import { useToastStore } from '../../src/stores/toast';
import { i18n } from '../../src/i18n';
import { installTauriMock } from '../utils/tauriMock';

const blank = defineComponent({ template: '<div />' });
async function routerAt(path = '/') {
  const router = createRouter({ history: createMemoryHistory(), routes: [
    { path: '/', component: blank }, { path: '/install', component: InstallView },
  ] });
  await router.push(path);
  await router.isReady();
  return router;
}

afterEach(() => {
  i18n.global.locale.value = 'en';
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe('action failure feedback', () => {
  it.each(['en', 'ar', 'fr'] as const)('shows one quiet stale-tools notice and retries immediately in %s', async (locale) => {
    i18n.global.locale.value = locale;
    const store = useBinariesStore();
    const toasts = useToastStore();
    const calls: string[] = [];
    installTauriMock({ binaries_update_now: () => {
      calls.push('update');
    } });
    const wrapper = mount(TheToaster, { global: { plugins: [i18n] } });
    store.processUpdateStale(true);
    store.processUpdateStale(true);
    await flushPromises();
    expect(toasts.toasts).toHaveLength(1);
    expect(toasts.toasts[0].duration).toBe(0);
    expect(wrapper.get('[role="status"]').text()).toContain(i18n.global.t('toolsUpdate.stale'));
    await wrapper.get('button').trigger('click');
    await flushPromises();
    expect(calls).toEqual(['update']);
    expect(toasts.toasts).toHaveLength(0);
    store.processUpdateStale(true);
    expect(toasts.toasts).toHaveLength(0);
    wrapper.unmount();
  });

  it('keeps a manual tool-update failure visible with a concrete next step', async () => {
    installTauriMock({ binaries_update_now: () => Promise.reject('/Users/PrivatePerson/tools/file.zip') });
    const store = useBinariesStore();
    const wrapper = mount(TheToaster, { global: { plugins: [i18n] } });
    store.processUpdateStale(true);
    await flushPromises();
    await wrapper.get('button').trigger('click');
    await flushPromises();
    expect(wrapper.get('[role="status"]').text()).toContain(i18n.global.t('toolsUpdate.failed'));
    expect(wrapper.text()).not.toContain('PrivatePerson');
    wrapper.unmount();
  });

  it('hides the stale-tools notice offline and removes it after recovery', () => {
    const store = useBinariesStore();
    const toasts = useToastStore();
    const online = vi.spyOn(navigator, 'onLine', 'get');
    online.mockReturnValue(false);
    store.processUpdateStale(true);
    expect(toasts.toasts).toHaveLength(0);
    online.mockReturnValue(true);
    store.refreshUpdateNotice();
    expect(toasts.toasts).toHaveLength(1);
    online.mockReturnValue(false);
    store.refreshUpdateNotice();
    expect(toasts.toasts).toHaveLength(0);
    online.mockReturnValue(true);
    store.refreshUpdateNotice();
    expect(toasts.toasts).toHaveLength(1);
    store.processUpdateStale(false);
    online.mockReturnValue(true);
    store.refreshUpdateNotice();
    expect(toasts.toasts).toHaveLength(0);
    store.processUpdateStale(true);
    expect(toasts.toasts).toHaveLength(1);
  });

  it.each([
    ['download', 'Download', 'Couldn\'t download the update'],
    ['install', 'Install', 'Couldn\'t install the update'],
  ] as const)('keeps updater %s errors visible and retries the operation', async (operation, label, message) => {
    const store = useUpdaterStore();
    store.checkResult = { available: true, currentVersion: '1', availableVersion: '2' };
    store.isNeedingRestart = operation === 'install';
    installTauriMock({ [`updater_${operation}`]: () => Promise.reject('offline') });
    const wrapper = mount(TheUpdater, { global: { plugins: [i18n] } });
    await wrapper.findAll('button').find(b => b.text() === label)!.trigger('click');
    await flushPromises();
    expect(wrapper.get('[role="alert"]').text()).toContain(message);
    const calls: string[] = [];
    installTauriMock({
      updater_download: () => { calls.push('download'); },
      updater_install: () => { calls.push('install'); },
    });
    await wrapper.findAll('button').find(b => b.text() === 'Retry')!.trigger('click');
    await flushPromises();
    expect(calls).toEqual(operation === 'install' ? ['download', 'install'] : ['download']);
    expect(wrapper.text()).not.toContain(message);
    wrapper.unmount();
  });

  it('keeps an updater error event visible instead of dismissing the card', async () => {
    const store = useUpdaterStore();
    store.checkResult = { available: true, currentVersion: '1' };
    store.isUpdating = true;
    const wrapper = mount(TheUpdater, { global: { plugins: [i18n] } });
    store.processUpdaterError('connection lost');
    await flushPromises();
    expect(wrapper.get('[role="alert"]').text()).toContain('Couldn\'t download the update');
  });

  it('retries only failed tools and clears stale errors before starting', async () => {
    vi.useFakeTimers();
    const tools = useBinariesStore();
    tools.tools = {
      'yt-dlp': { percent: 0, received: 0, total: 1, error: 'offline' },
      'ffmpeg': { percent: 100, received: 1, total: 1 },
    };
    installTauriMock({ binaries_ensure: () => Promise.reject('offline') });
    const router = await routerAt('/install');
    const wrapper = mount(InstallView, { global: { plugins: [router, i18n] } });
    await flushPromises();
    const calls: unknown[] = [];
    let finish!: () => void;
    installTauriMock({ binaries_ensure: (_cmd, args) => {
      calls.push(args);
      return new Promise<void>((resolve) => {
        finish = resolve;
      });
    } });
    await wrapper.findAll('button').find(b => b.text() === 'Retry')!.trigger('click');
    expect(calls).toEqual([{ tools: ['yt-dlp'] }]);
    expect(tools.tools['yt-dlp'].error).toBeUndefined();
    finish();
    await flushPromises();
    expect(wrapper.text()).toContain('Ready to download');
    wrapper.unmount();
  });

  it('routes a startup tool-check rejection to a visible failure with Retry', async () => {
    installTauriMock({
      binaries_check: () => Promise.reject('check failed'),
      stronghold_status: () => ({ unlocked: false }),
      updater_check: () => ({ available: false, currentVersion: '1' }),
    });
    useSettingsStore().settings.update.updateBinaries = true;
    const router = await routerAt();
    const wrapper = mount(App, { global: { plugins: [router, i18n] } });
    await flushPromises();
    expect(router.currentRoute.value.path).toBe('/install');
    expect(wrapper.text()).toContain('Couldn\'t check the download tools');
    expect(wrapper.findAll('button').some(b => b.text() === 'Retry')).toBe(true);
    wrapper.unmount();
  });

  it.each([
    ['en', 'Couldn\'t open the downloads folder'],
    ['ar', 'تعذّر فتح مجلد التنزيلات'],
    ['fr', 'Impossible d’ouvrir le dossier des téléchargements'],
  ] as const)('reports a folder failure in %s when both locations fail', async (locale, message) => {
    i18n.global.locale.value = locale;
    useSettingsStore().settings.output.downloadDir = '/d/Nezzel';
    installTauriMock({ 'plugin:opener|open_path': () => Promise.reject('missing') });
    const wrapper = mount(AppStatusBar, { global: { plugins: [i18n], stubs: { Tooltip: { template: '<span><slot /></span>' } } } });
    await wrapper.get('button').trigger('click');
    await flushPromises();
    expect(useToastStore().toasts.at(-1)).toMatchObject({ message, style: 'error' });
    wrapper.unmount();
  });
});
