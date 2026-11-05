import { mount, shallowMount } from '@vue/test-utils';
import { createMemoryHistory, createRouter } from 'vue-router';
import { describe, expect, it } from 'vitest';
import MediaDownloadOptions from '../../src/components/media-card/MediaDownloadOptions.vue';
import { SelectField } from '../../src/components/ui/select';
import { TrackType } from '../../src/tauri/types/media';
import FileTypeIcon from '../../src/components/downloads/FileTypeIcon.vue';
import DownloadRow from '../../src/components/downloads/DownloadRow.vue';
import MediaView from '../../src/views/app/MediaView.vue';
import { i18n } from '../../src/i18n';
import { useMediaGroupStore } from '../../src/stores/media/group';
import { useMediaDiagnosticsStore } from '../../src/stores/media/diagnostics';
import { MediaState, useMediaStateStore } from '../../src/stores/media/state';

describe('download failure presentation', () => {
  it.each([['en', true], ['ar', true], ['fr', true], ['en', false]] as const)('offers recovery instead of Retry for no-audio failure (%s, video: %s)', async (locale, video) => {
    const previous = i18n.global.locale.value;
    i18n.global.locale.value = locale;
    const groups = useMediaGroupStore();
    groups.createGroup({ id: 'silent', url: 'https://t.me/durov/531', title: 'Silent clip', mediaKind: video ? 'video' : undefined, total: 1, processed: 0, errored: 0, isCombined: false,
      audioCodecs: [], formats: video ? [{ id: 'v', height: 720, audioCodecs: [], videoCodecs: ['h264'] }] : [], filesize: 0,
      items: { item: { id: 'item', isLeader: true, url: 'https://t.me/durov/531', audioCodecs: [], formats: [], filesize: 0 } } });
    useMediaDiagnosticsStore().processMediaDiagnosticPayload({ id: 'item', groupId: 'silent', code: 'noAudioTrack', level: 'error', component: null, message: '', raw: '', timestamp: 1 });
    useMediaStateStore().setGroupState('silent', MediaState.error);
    const router = createRouter({ history: createMemoryHistory(), routes: [{ path: '/', component: { template: '<div />' } }] });
    await router.push('/');
    const row = mount(DownloadRow, { props: { group: groups.findGroupById('silent')! }, global: { plugins: [router, i18n] } });
    try {
      expect(row.text()).toContain(i18n.global.t('errors.runner.noAudioTrack.shortMessage'));
      expect(row.get('[data-testid="row-error"]').text()).toBe(i18n.global.t('errors.rowLine', {
        what: i18n.global.t('errors.runner.noAudioTrack.shortMessage'), action: i18n.global.t('errors.runner.noAudioTrack.action') }));
      const actionKey = video ? 'downloads.row.downloadVideoInstead' : 'downloads.row.retryWithBest';
      const actionText = i18n.global.t(actionKey);
      for (const key of ['downloads.row.downloadVideoInstead', 'downloads.row.retryWithBest']) {
        expect(i18n.global.t(key)).not.toBe(key);
        expect(i18n.global.te(key, locale)).toBe(true);
      }
      expect(row.findAll('button').some(b => b.text().includes(actionText))).toBe(true);
      expect(row.findAll('button').some(b => b.text() === i18n.global.t('downloads.row.retry'))).toBe(false);
    } finally {
      row.unmount();
      i18n.global.locale.value = previous;
    }
  });

  it.each([['unknown', TrackType.audio, true], ['none', TrackType.both, false]] as const)('handles %s audio evidence in Advanced without losing a remembered choice', (codec, expectedTrack, offersAudio) => {
    const wrapper = shallowMount(MediaDownloadOptions, { props: {
      formats: [{ id: 'v', height: 720, audioCodecs: [codec], videoCodecs: ['h264'] }],
      modelValue: { trackType: TrackType.audio, id: 'best' },
    }, global: { plugins: [i18n] } });
    try {
      const tracks = wrapper.findComponent(SelectField);
      const props = tracks.props() as { modelValue: TrackType; options: { value: string }[] };
      expect(props.modelValue).toBe(expectedTrack);
      expect(props.options.some(option => option.value === TrackType.audio)).toBe(offersAudio);
    } finally { wrapper.unmount(); }
  });

  it.each([['en', 'report.pdf', 'document'], ['ar', 'backup.zip', 'archive'], ['fr', 'unknown.bin', 'other']] as const)('shows a %s file icon and size for %s', async (locale, title, category) => {
    const previous = i18n.global.locale.value;
    i18n.global.locale.value = locale;
    const groups = useMediaGroupStore();
    groups.createGroup({ id: 'file', title, url: 'https://files.example/a', mediaKind: 'file', total: 1, processed: 0, errored: 0, isCombined: false, audioCodecs: [], formats: [], filesize: 4096,
      items: { item: { id: 'item', isLeader: true, url: 'https://files.example/a', audioCodecs: [], formats: [], filesize: 4096 } } });
    useMediaStateStore().setGroupState('file', MediaState.configure);
    const router = createRouter({ history: createMemoryHistory(), routes: [{ path: '/', component: { template: '<div />' } }] });
    await router.push('/');
    const row = mount(DownloadRow, { props: { group: groups.findGroupById('file')! }, global: { plugins: [router, i18n] } });
    try {
      expect(row.findComponent(FileTypeIcon).props('category')).toBe(category);
      expect(row.text()).toContain(i18n.global.t(`files.category.${category}`));
      expect(row.text()).toMatch(/4[.,]1/);
    } finally {
      row.unmount();
      i18n.global.locale.value = previous;
    }
  });

  it.each([
    ['en', /try again/i],
    ['ar', /أعد المحاولة/],
    ['fr', /réessayez/i],
    ['es', /try again/i], // Locales awaiting this hint use the English fallback.
  ] as const)('gives a useful %s hint and keeps the final engine error in collapsed details', async (locale, hint) => {
    const originalLocale = i18n.global.locale.value;
    i18n.global.locale.value = locale;
    const groups = useMediaGroupStore();
    groups.createGroup({
      id: 'g1', url: 'https://example.com/video', title: 'Video', total: 1,
      processed: 0, errored: 0, isCombined: false, audioCodecs: [], formats: [], filesize: 0,
      items: { item: { id: 'item', isLeader: true, url: 'https://example.com/video', audioCodecs: [], formats: [], filesize: 0 } },
    });
    useMediaStateStore().setGroupState('g1', MediaState.downloading);
    useMediaDiagnosticsStore().processMediaFatalPayload({
      id: 'item', groupId: 'g1', exitCode: 1, internal: false,
      message: 'Download failed', details: 'ERROR: Unexpected server response at [redacted URL]', timestamp: 1,
    });
    const router = createRouter({
      history: createMemoryHistory(),
      routes: [{ path: '/', name: 'home', component: { template: '<div />' } }],
    });
    await router.push('/');
    const global = { plugins: [router, i18n] };
    const row = mount(DownloadRow, { props: { group: groups.findGroupById('g1')! }, global });
    const view = mount(MediaView, { props: { groupId: 'g1' }, global });
    try {
      expect(row.find('.line-clamp-2').text()).toMatch(hint);
      expect(row.text()).not.toContain('Unexpected server response');
      const details = view.find('[data-slot=collapsible]');
      expect(details.attributes('data-state')).toBe('closed');
      expect(details.text()).toContain('ERROR: Unexpected server response at [redacted URL]');
      expect(details.text()).not.toContain('https://example.com/video');
    } finally {
      row.unmount();
      view.unmount();
      i18n.global.locale.value = originalLocale;
    }
  });
});
