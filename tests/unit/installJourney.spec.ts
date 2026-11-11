import { flushPromises, mount } from '@vue/test-utils';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { createMemoryHistory, createRouter } from 'vue-router';
import { defineComponent } from 'vue';
import { resetPlatformForTests } from '../../src/composables/usePlatform';
import { Platform } from '../../src/tauri/types/app';
import InstallView from '../../src/views/full/InstallView.vue';
import { useBinariesStore } from '../../src/stores/binaries';
import { i18n } from '../../src/i18n';
import { installTauriMock } from '../utils/tauriMock';
import ToolCard from '../../src/components/tool-card/ToolCard.vue';

const blank = defineComponent({ template: '<div />' });

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe('first-run tool install', () => {
  it('shows the causal error after a wrapper line and reacts to going offline', async () => {
    const online = vi.spyOn(navigator, 'onLine', 'get').mockReturnValue(true);
    const wrapper = mount(ToolCard, { props: { name: 'yt-dlp', tool: { percent: 0, total: 0, received: 0, error: '[download] Tool repair failed\nCaused by:\n  DNS lookup failed' } }, global: { plugins: [i18n] } });
    expect(wrapper.get('[data-testid="tool-error-reason"]').text()).toContain('DNS lookup failed');
    online.mockReturnValue(false);
    window.dispatchEvent(new Event('offline'));
    await flushPromises();
    expect(wrapper.text()).toContain('offline');
    online.mockReturnValue(true);
    await wrapper.setProps({ tool: { percent: 0, total: 0, received: 0, error: '[download] Network is unreachable' } });
    window.dispatchEvent(new Event('online'));
    await flushPromises();
    expect(wrapper.text()).toContain('offline');
    wrapper.unmount();
  });

  it.each([
    [Platform.MacOS, 'ar', '84px'], [Platform.MacOS, 'en', '84px'], [Platform.Linux, 'ar', ''], [Platform.Windows, 'ar', ''],
  ] as const)('keeps install controls clear on %s in %s', async (platform, locale, padding) => {
    const previous = i18n.global.locale.value;
    i18n.global.locale.value = locale;
    resetPlatformForTests(platform);
    installTauriMock({ get_platform: () => platform, binaries_ensure: () => Promise.resolve() });
    const router = createRouter({ history: createMemoryHistory(), routes: [{ path: '/', component: blank }] });
    await router.push('/');
    useBinariesStore().tools = { 'yt-dlp': { percent: 100, received: 100, total: 100 } };
    const wrapper = mount(InstallView, { global: { plugins: [router, i18n] } });
    try {
      await flushPromises();
      expect((wrapper.get('nav').element as HTMLElement).style.paddingLeft).toBe(padding);
    } finally {
      wrapper.unmount();
      resetPlatformForTests();
      i18n.global.locale.value = previous;
    }
  });

  it('shows progress, blocks Continue while installing, then enables it and navigates on', async () => {
    const router = createRouter({ history: createMemoryHistory(), routes: [
      { path: '/', component: blank }, { path: '/install', component: InstallView },
    ] });
    await router.push('/install');
    await router.isReady();
    const tools = useBinariesStore();
    tools.tools = { 'yt-dlp': { percent: 0, received: 0, total: 0 } };
    let finish!: () => void;
    installTauriMock({ binaries_ensure: () => new Promise<void>((resolve) => {
      finish = resolve;
    }) });
    const wrapper = mount(InstallView, { global: { plugins: [router, i18n] }, attachTo: document.body });
    await flushPromises();

    tools.processBinaryDownloadStart({ tool: 'yt-dlp', version: '1.2.3' } as never);
    tools.processBinaryDownloadProgress({ tool: 'yt-dlp', received: 50, total: 100 } as never);
    await flushPromises();
    expect(wrapper.text()).toContain('1.2.3');
    expect(wrapper.text()).toContain('50%');
    const link = () => wrapper.get('a[href="/"]');
    expect(link().text()).toContain('Please wait');
    expect(link().attributes('aria-disabled')).toBe('true');

    tools.processBinaryDownloadComplete({ tool: 'yt-dlp' } as never);
    finish();
    await flushPromises();
    expect(link().text()).toContain('Continue');
    expect(link().text()).not.toContain('Please wait');
    expect(link().attributes('aria-disabled')).toBeUndefined();

    vi.useFakeTimers();
    await link().trigger('click');
    await flushPromises();
    expect(router.currentRoute.value.path).toBe('/');
    wrapper.unmount();
  });
});
