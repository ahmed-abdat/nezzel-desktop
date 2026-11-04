import { mount, flushPromises } from '@vue/test-utils';
import { afterEach, describe, expect, it } from 'vitest';
import { createPinia } from 'pinia';
import { clearMocks } from '@tauri-apps/api/mocks';
import { installTauriMock } from '../utils/tauriMock';
import DownloadInspector from '../../src/components/downloads/DownloadInspector.vue';
import { i18n, applyDocumentLocale } from '../../src/i18n';
import { useMediaGroupStore } from '../../src/stores/media/group';
import { useMediaOptionsStore } from '../../src/stores/media/options';
import { useMediaStateStore, MediaState } from '../../src/stores/media/state';
import { useDownloadSheetStore } from '../../src/stores/downloadSheet';
import type { Group } from '../../src/tauri/types/group';
import { TrackType } from '../../src/tauri/types/media';

describe('attachment picker in the options panel', () => {
  afterEach(() => {
    clearMocks();
    i18n.global.locale.value = 'en';
    applyDocumentLocale('en');
  });

  it('downloads a video carousel with both tracks when no quality list is available', async () => {
    const requests: unknown[] = [];
    installTauriMock({ media_download: (_command, args) => {
      requests.push(args);
      return 'ok';
    } });
    const pinia = createPinia();
    const attachments = ['first', 'second'].map(id => ({ id, kind: 'video' as const, url: `https://cdn.example/${id}.mp4` }));
    const item = {
      id: 'post', isLeader: true, url: 'https://www.reddit.com/comments/abc123/', title: 'Post',
      mediaKind: 'carousel' as const, attachments, audioCodecs: [], formats: [], filesize: 0,
    };
    const group: Group = { ...item, total: 1, processed: 1, errored: 0, isCombined: false, items: { post: item } };
    useMediaGroupStore(pinia).createGroup(group);
    useMediaStateStore(pinia).setGroupState(group.id, MediaState.configure);
    useDownloadSheetStore(pinia).show(group.id);
    const wrapper = mount(DownloadInspector, { global: { plugins: [pinia, i18n], stubs: { RouterLink: true } } });
    try {
      await flushPromises();
      await wrapper.findAll('button').find(button => button.text() === i18n.global.t('sheet.download'))!.trigger('click');
      await flushPromises();
      expect(requests).toEqual([expect.objectContaining({
        items: [expect.objectContaining({ format: expect.objectContaining({ trackType: TrackType.both }), attachmentIds: ['first', 'second'] })],
      })]);
    } finally {
      wrapper.unmount();
    }
  });

  it.each(['en', 'ar'] as const)('lets a File download without media format controls in %s', async (locale) => {
    i18n.global.locale.value = locale;
    applyDocumentLocale(locale);
    const pinia = createPinia();
    const group: Group = {
      id: 'file', url: 'https://example.com/report.pdf', title: 'report.pdf', total: 1,
      processed: 1, errored: 0, isCombined: false, mediaKind: 'file',
      items: { file: { id: 'file', isLeader: true, url: 'https://example.com/report.pdf', mediaKind: 'file', audioCodecs: [], formats: [], filesize: 1024 } },
      audioCodecs: [], formats: [], filesize: 1024,
    };
    useMediaGroupStore(pinia).createGroup(group);
    useMediaStateStore(pinia).setGroupState(group.id, MediaState.configure);
    useDownloadSheetStore(pinia).show(group.id);
    const wrapper = mount(DownloadInspector, { global: { plugins: [pinia, i18n], stubs: { RouterLink: true } } });
    await flushPromises();
    expect(wrapper.findAll('[role="radio"]')).toHaveLength(0);
    expect(wrapper.findAll('button').find(button => button.text() === i18n.global.t('sheet.download'))!.attributes('disabled')).toBeUndefined();
    wrapper.unmount();
  });

  it.each(['en', 'ar'] as const)('keeps selections across reopening and prevents empty selections in %s', async (locale) => {
    i18n.global.locale.value = locale;
    applyDocumentLocale(locale);
    const pinia = createPinia();
    const group: Group = {
      id: 'photos', url: 'https://example.com/post', title: 'Photos', total: 1,
      processed: 1, errored: 0, isCombined: false,
      items: { leader: { id: 'leader', isLeader: true, url: 'https://example.com/post', audioCodecs: [], formats: [], filesize: 0 } },
      audioCodecs: [], formats: [], filesize: 0,
      mediaKind: 'carousel',
      attachments: [
        { id: 'photo-1', kind: 'image', url: 'https://example.com/1.jpg' },
        { id: 'photo-2', kind: 'image', url: 'https://example.com/2.jpg' },
      ],
    };
    useMediaGroupStore(pinia).createGroup(group);
    useMediaStateStore(pinia).setGroupState(group.id, MediaState.configure);
    const sheet = useDownloadSheetStore(pinia);
    sheet.show(group.id);
    const options = useMediaOptionsStore(pinia);
    const render = () => mount(DownloadInspector, { global: { plugins: [pinia, i18n], stubs: { RouterLink: true } } });
    let wrapper = render();
    await flushPromises();

    expect(wrapper.findAll('fieldset [role="checkbox"]')).toHaveLength(3);
    expect(wrapper.findAll('[role="radio"]')).toHaveLength(0);
    await wrapper.findAll('fieldset [role="checkbox"]')[2].trigger('click');
    expect(options.getAttachmentSelection(group.id)).toEqual(['photo-1']);
    sheet.close();
    wrapper.unmount();
    sheet.show(group.id);
    wrapper = render();
    await flushPromises();
    expect(wrapper.findAll('fieldset [role="checkbox"]')[2].attributes('aria-checked')).toBe('false');
    await wrapper.findAll('fieldset [role="checkbox"]')[1].trigger('click');
    expect(options.getAttachmentSelection(group.id)).toEqual([]);
    const download = () => wrapper.findAll('button').find(button => button.text() === i18n.global.t('sheet.download'))!;
    expect(download().attributes('disabled')).toBeDefined();
    await wrapper.findAll('fieldset [role="checkbox"]')[0].trigger('click');
    expect(options.getAttachmentSelection(group.id)).toEqual(['photo-1', 'photo-2']);
    expect(download().attributes('disabled')).toBeUndefined();
    wrapper.unmount();
  });
});
