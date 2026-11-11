import { flushPromises, mount } from '@vue/test-utils';
import { createMemoryHistory, createRouter } from 'vue-router';
import { defineComponent, reactive, ref } from 'vue';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import LinkField from '../../src/components/downloads/LinkField.vue';
import { useDownloadSheetStore } from '../../src/stores/downloadSheet';
import { useUiPrefs } from '../../src/composables/useUiPrefs';
import { i18n } from '../../src/i18n';
import { useDragDropStore } from '../../src/stores/dragDrop';
import { downloadSearchQuery } from '../../src/composables/useDownloadList';
import { createDefaultInputFilterSettings } from '../../src/helpers/inputFilters';

const invoke = vi.hoisted(() => vi.fn());
vi.mock('@tauri-apps/api/core', () => ({ invoke }));

const dispatchMediaInfoFetch = vi.fn();
const addUrlBatch = vi.fn();
const hasRecentCapture = vi.fn(() => false);
const addUrlBatchAndDownload = vi.fn();
const downloadPreparedGroup = vi.fn();
const captureShortcut = reactive({ active: 'CommandOrControl+Alt+K' as string | null, failed: false });
vi.mock('../../src/composables/useCaptureShortcut', () => ({ useCaptureShortcut: () => captureShortcut }));
const showToast = vi.fn();
const toggleClipboard = vi.fn();
const markSeen = vi.fn();
const poll = vi.fn();
const clipboardContent = ref('');
const watchClipboardState = reactive({
  enabled: false,
});
const settingsState = reactive({
  input: {
    autoFillClipboard: false,
    preferVideoInMixedLinks: false,
    globalShortcuts: true,
  },
  inputFilters: createDefaultInputFilterSettings(),
});

vi.mock('../../src/stores/media/media', () => ({
  useMediaStore: () => ({
    dispatchMediaInfoFetch,
    addUrlBatch,
    hasRecentCapture,
    addUrlBatchAndDownload,
    downloadPreparedGroup,
  }),
}));

vi.mock('../../src/stores/toast', () => ({
  useToastStore: () => ({
    showToast,
  }),
}));

vi.mock('../../src/stores/watchClipboard', () => ({
  useWatchClipboardStore: () => ({
    get enabled() {
      return watchClipboardState.enabled;
    },
    toggle: toggleClipboard,
    hasSeen: vi.fn(() => false),
    markSeen,
  }),
}));

vi.mock('../../src/stores/settings', () => ({
  useSettingsStore: () => ({
    settings: settingsState,
  }),
}));

vi.mock('../../src/composables/useClipboard', () => ({
  useClipboard: () => ({
    content: clipboardContent,
    poll,
  }),
}));

function createBlankView() {
  return defineComponent({
    template: '<div />',
  });
}

describe('LinkField', () => {
  beforeEach(() => {
    invoke.mockReset().mockRejectedValue(new Error('no backend'));
    dispatchMediaInfoFetch.mockReset();
    addUrlBatch.mockReset();
    hasRecentCapture.mockReset().mockReturnValue(false);
    addUrlBatch.mockResolvedValue([]);
    addUrlBatchAndDownload.mockReset();
    downloadPreparedGroup.mockReset().mockResolvedValue('downloaded');
    captureShortcut.active = 'CommandOrControl+Alt+K';
    captureShortcut.failed = false;
    showToast.mockReset();
    toggleClipboard.mockReset();
    markSeen.mockReset();
    poll.mockReset();
    clipboardContent.value = '';
    watchClipboardState.enabled = false;
    settingsState.input.autoFillClipboard = false;
    settingsState.inputFilters = createDefaultInputFilterSettings();
    useUiPrefs().skipOptionsSheet.value = false;
    downloadSearchQuery.value = '';
  });

  async function mountField() {
    const router = createRouter({
      history: createMemoryHistory(),
      routes: [
        { path: '/', name: 'home', component: createBlankView() },
        { path: '/input-filters', name: 'input-filters', component: createBlankView() },
      ],
    });
    await router.push('/');
    await router.isReady();
    const wrapper = mount(LinkField, { global: { plugins: [router, i18n], stubs: { Tooltip: { template: '<slot />' } } } });
    return { router, wrapper };
  }

  const submitButton = (wrapper: Awaited<ReturnType<typeof mountField>>['wrapper']) =>
    wrapper.findAll('button').find(button => button.text().trim() === 'Download')!;

  it('keeps invalid input and the current page, with a warning', async () => {
    const { router, wrapper } = await mountField();
    await router.push('/input-filters');
    await wrapper.get('#queue-url-input').setValue('hello');
    await wrapper.get('form').trigger('submit');
    await flushPromises();
    expect((wrapper.get('#queue-url-input').element as HTMLInputElement).value).toBe('hello');
    expect(router.currentRoute.value.name).toBe('input-filters');
    expect(showToast).toHaveBeenCalledWith('That doesn\'t look like a link', { style: 'warning' });
    expect(addUrlBatch).not.toHaveBeenCalled();
  });

  it.each(['field', 'drop'])('clears search immediately when accepting a new link via %s', async (source) => {
    const { wrapper } = await mountField();
    downloadSearchQuery.value = 'unrelated';
    if (source === 'drop') {
      await useDragDropStore().handleDrop({ urls: ['https://example.com/new'], files: [] });
    } else {
      await wrapper.get('#queue-url-input').setValue('https://example.com/new');
      await wrapper.get('form').trigger('submit');
    }
    expect(downloadSearchQuery.value).toBe('');
  });

  it('queues several links without opening the options sheet', async () => {
    const { wrapper } = await mountField();
    await wrapper.get('#queue-url-input').setValue('https://example.com/a https://example.com/b');
    await wrapper.get('form').trigger('submit');
    await flushPromises();

    expect(addUrlBatch).toHaveBeenCalledWith(['https://example.com/a', 'https://example.com/b']);
    expect(useDownloadSheetStore().open).toBe(false);
    expect((wrapper.get('#queue-url-input').element as HTMLInputElement).value).toBe('');
  });

  it('opens the options sheet for a single link', async () => {
    addUrlBatch.mockResolvedValue(['group-1']);
    const { wrapper } = await mountField();
    await wrapper.get('#queue-url-input').setValue('https://example.com/a');
    await wrapper.get('form').trigger('submit');
    await flushPromises();

    const sheet = useDownloadSheetStore();
    expect(sheet.open).toBe(true);
    expect(sheet.groupId).toBe('group-1');
  });

  it.each(['native', 'backend'])('preserves all links in a multiline %s paste', async (source) => {
    const text = 'https://example.com/a\r\nhttps://example.com/b';
    const { wrapper } = await mountField();
    if (source === 'native') await wrapper.get('#queue-url-input').trigger('paste', { clipboardData: { getData: () => text } });
    else {
      invoke.mockResolvedValue(text);
      await (wrapper.vm as unknown as { paste: () => Promise<void> }).paste();
    }
    await wrapper.get('form').trigger('submit');
    await flushPromises();
    expect(addUrlBatch).toHaveBeenCalledWith(['https://example.com/a', 'https://example.com/b']);
    wrapper.unmount();
  });

  it.each([
    ['en', 'Copied-link card: off. Quick download from any app: Ctrl+Alt+K', 'Copied-link card: on. Set up quick download in Settings → Keyboard shortcuts'],
    ['ar', 'بطاقة الروابط المنسوخة: معطّلة؛ تنزيل سريع من أي تطبيق: Ctrl+Alt+K', 'بطاقة الروابط المنسوخة: مفعّلة؛ اضبط التنزيل السريع في الإعدادات ← اختصارات لوحة المفاتيح'],
  ] as const)('localizes the clipboard state and shortcut sentence together in %s', async (locale, active, failed) => {
    const oldLocale = i18n.global.locale.value;
    i18n.global.locale.value = locale;
    const { wrapper } = await mountField();
    try {
      const indicator = () => wrapper.get('[role="img"]').attributes('aria-label');
      expect(indicator()).toBe(active);
      captureShortcut.failed = true;
      watchClipboardState.enabled = true;
      await flushPromises();
      expect(indicator()).toBe(failed);
    } finally {
      wrapper.unmount();
      i18n.global.locale.value = oldLocale;
    }
  });

  it('offers to download only the dropped batch', async () => {
    addUrlBatch.mockResolvedValue(['drop-a', 'drop-b']);
    await useDragDropStore().handleDrop({ urls: ['https://example.com/a', 'https://example.com/b'], files: [] });
    const options = showToast.mock.calls.at(-1)?.[1];
    expect(options.action?.label).toBe('Download all');
    await options.action.run();
    expect(downloadPreparedGroup.mock.calls.map(call => call[0])).toEqual(['drop-a', 'drop-b']);
  });

  it.each([
    ['en', 1, '1 playlist needs you to pick items'],
    ['en', 2, '2 playlists need you to pick items'],
    ['ar', 1, 'قائمة تشغيل واحدة تحتاج إلى اختيار عناصرها'],
    ['ar', 2, 'قائمتا تشغيل تحتاجان إلى اختيار عناصرهما'],
    ['fr', 1, '1 playlist nécessite de choisir des éléments'],
    ['fr', 2, '2 playlists nécessitent de choisir des éléments'],
  ] as const)('reports pending playlist selection in %s (%i)', async (locale, count, message) => {
    const oldLocale = i18n.global.locale.value;
    i18n.global.locale.value = locale;
    try {
      const playlists = Array.from({ length: count }, (_, i) => `playlist-${i}`);
      addUrlBatch.mockResolvedValue(['video', ...playlists, 'failed']);
      downloadPreparedGroup.mockImplementation(async (id: string) => {
        if (id === 'failed') throw new Error('download failed');
        return id === 'video' ? 'downloaded' : 'playlist';
      });
      await useDragDropStore().handleDrop({ urls: ['https://example.com/a', 'https://example.com/b'], files: [] });
      await showToast.mock.calls.at(-1)![1].action.run();
      expect(downloadPreparedGroup).toHaveBeenCalledTimes(count + 2);
      expect(showToast).toHaveBeenCalledWith(i18n.global.t('feedback.downloadFailed'), { style: 'error' });
      expect(showToast).toHaveBeenCalledWith(message, { style: 'info' });
    } finally {
      i18n.global.locale.value = oldLocale;
    }
  });

  it.each([
    ['shift-click', true, false],
    ['the skip-options preference', false, true],
  ])('downloads straight away with %s', async (_case, shift, skip) => {
    useUiPrefs().skipOptionsSheet.value = skip;
    const { wrapper } = await mountField();
    await wrapper.get('#queue-url-input').setValue('https://example.com/a');
    if (shift) await submitButton(wrapper).trigger('click', { shiftKey: true });
    else await wrapper.get('form').trigger('submit');
    await flushPromises();

    expect(addUrlBatchAndDownload).toHaveBeenCalledWith(['https://example.com/a'], false, true);
    expect(addUrlBatch).not.toHaveBeenCalled();
  });

  it('uses clipboard content as the placeholder when it contains a valid url', async () => {
    clipboardContent.value = 'https://example.com/from-clipboard';
    const { wrapper } = await mountField();
    expect((wrapper.get('#queue-url-input').element as HTMLInputElement).placeholder)
      .toBe('https://example.com/from-clipboard');
  });

  it('does not submit a recently captured clipboard placeholder but allows an explicit draft', async () => {
    clipboardContent.value = 'https://example.com/from-clipboard';
    hasRecentCapture.mockReturnValue(true);
    const { wrapper } = await mountField();
    await wrapper.get('form').trigger('submit');
    expect(addUrlBatch).not.toHaveBeenCalled();
    await wrapper.get('#queue-url-input').setValue(clipboardContent.value);
    await wrapper.get('form').trigger('submit');
    expect(addUrlBatch).toHaveBeenCalledWith([clipboardContent.value]);
  });

  it('pastes native clipboard text without submitting or adding it twice', async () => {
    invoke.mockImplementation((cmd: string) => cmd === 'capture_read_clipboard'
      ? Promise.resolve('https://example.com/pasted')
      : Promise.reject(new Error('unexpected')));
    const { wrapper } = await mountField();
    await wrapper.get('#queue-url-input').setValue('https://example.com/old');
    await (wrapper.vm as unknown as { paste: () => Promise<void> }).paste();
    expect((wrapper.get('#queue-url-input').element as HTMLInputElement).value).toBe('https://example.com/pasted');
    expect(addUrlBatch).not.toHaveBeenCalled();
  });

  it('pastes through the one backend reader (GTK on Linux Wayland)', async () => {
    invoke.mockImplementation((cmd: string) => cmd === 'capture_read_clipboard'
      ? Promise.resolve('https://example.com/gtk')
      : Promise.reject(new Error('unexpected')));
    const { wrapper } = await mountField();
    await (wrapper.vm as unknown as { paste: () => Promise<void> }).paste();
    expect((wrapper.get('#queue-url-input').element as HTMLInputElement).value).toBe('https://example.com/gtk');
  });

  it('fills the field itself only when the webview hands it an empty native paste', async () => {
    invoke.mockResolvedValue('https://example.com/wayland');
    const { wrapper } = await mountField();
    const field = wrapper.get('#queue-url-input');
    await field.trigger('paste', { clipboardData: { getData: () => 'https://example.com/native' } });
    await flushPromises();
    expect(invoke).not.toHaveBeenCalledWith('capture_read_clipboard');
    await field.trigger('paste', { clipboardData: { getData: () => '' } });
    await flushPromises();
    expect((field.element as HTMLInputElement).value).toBe('https://example.com/wayland');
  });

  it('keeps the draft when clipboard access fails', async () => {
    invoke.mockRejectedValue(new Error('denied'));
    const { wrapper } = await mountField();
    await wrapper.get('#queue-url-input').setValue('draft');
    await (wrapper.vm as unknown as { paste: () => Promise<void> }).paste();
    expect((wrapper.get('#queue-url-input').element as HTMLInputElement).value).toBe('draft');
    expect(showToast).toHaveBeenCalledWith('Couldn\'t read the clipboard', { style: 'error' });
  });

  it('marks active link filters and navigates to them from the menu', async () => {
    settingsState.inputFilters.matchFilters = '!is_live';
    const { router, wrapper } = await mountField();
    await wrapper.get('button[aria-haspopup="menu"]').trigger('click');

    const filtersButton = wrapper.findAll('button').find(button => button.text().includes(i18n.global.t('downloads.field.filters')));
    expect(filtersButton?.classes()).toContain('is-on');
    await filtersButton!.trigger('click');
    await flushPromises();

    expect(router.currentRoute.value.name).toBe('input-filters');
  });
});
