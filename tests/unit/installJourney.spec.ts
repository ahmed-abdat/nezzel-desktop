import { flushPromises, mount } from '@vue/test-utils';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { createMemoryHistory, createRouter } from 'vue-router';
import { defineComponent } from 'vue';
import InstallView from '../../src/views/full/InstallView.vue';
import { useBinariesStore } from '../../src/stores/binaries';
import { i18n } from '../../src/i18n';
import { installTauriMock } from '../utils/tauriMock';

const blank = defineComponent({ template: '<div />' });

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe('first-run tool install', () => {
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
