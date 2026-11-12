import { flushPromises, mount, shallowMount } from '@vue/test-utils';
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
import { installTauriMock } from '../utils/tauriMock';
import { MediaState, useMediaStateStore } from '../../src/stores/media/state';
import { useLoginStore } from '../../src/stores/login';

describe('download failure presentation', () => {
  it.each(['en', 'ar', 'fr'] as const)('shows the cookie remedy inside a terminal login card in %s', async (locale) => {
    const previous = i18n.global.locale.value;
    i18n.global.locale.value = locale;
    const groups = useMediaGroupStore();
    groups.createGroup({ id: 'cookies', url: 'https://reddit.com/post', title: 'Post', mediaKind: 'video', total: 1, processed: 1, errored: 1, isCombined: false, audioCodecs: [], formats: [], filesize: 0,
      items: { item: { id: 'item', isLeader: true, url: 'https://reddit.com/post', audioCodecs: [], formats: [], filesize: 0 } } });
    useMediaDiagnosticsStore().processMediaDiagnosticPayload({ id: 'item', groupId: 'cookies', code: 'browserDpapiFailed', level: 'error', component: null, message: '', raw: '', timestamp: 1 });
    useMediaStateStore().setGroupState('cookies', MediaState.error);
    const login = useLoginStore();
    login.receive({ requestId: 'cookies', id: 'item', groupId: 'cookies', host: 'reddit.com', status: 'exhausted', reason: 'browserLoginFailed', candidates: [] });
    const router = createRouter({ history: createMemoryHistory(), routes: [{ path: '/', component: { template: '<div />' } }, { path: '/settings/logins', name: 'settings.logins', component: { template: '<div />' } }] });
    await router.push('/');
    const row = mount(DownloadRow, { props: { group: groups.findGroupById('cookies')! }, global: { plugins: [router, i18n] } });
    try {
      expect(row.text()).toContain(i18n.global.t('errors.runner.browserDpapiFailed.action'));
      expect(row.text()).not.toContain(i18n.global.t('downloads.login.noBrowser'));
    } finally {
      row.unmount();
      login.forgetGroup('cookies');
      i18n.global.locale.value = previous;
    }
  });

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
  ] as const)('gives a useful %s hint and preserves engine output only when copied', async (locale, hint) => {
    let copied = '';
    installTauriMock({
      logging_subscribe: () => 'main\nWARNING: engine output',
      capture_write_clipboard: (_cmd, args) => { copied = (args as { text: string }).text; },
    });
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
      routes: [
        { path: '/', name: 'home', component: { template: '<div />' } },
        { path: '/group/:groupId', name: 'group.metadata', component: MediaView, props: true },
      ],
    });
    await router.push('/');
    const global = { plugins: [router, i18n] };
    const row = mount(DownloadRow, { props: { group: groups.findGroupById('g1')! }, global });
    const view = mount(MediaView, { props: { groupId: 'g1' }, global });
    try {
      expect(row.find('.line-clamp-2').text()).toMatch(hint);
      expect(row.text()).not.toContain('Unexpected server response');
      await row.get('[data-testid=row-error-details]').trigger('click');
      await flushPromises();
      expect(router.currentRoute.value).toMatchObject({ name: 'group.metadata', params: { groupId: 'g1' } });
      expect(row.find('[data-testid=row-error-raw]').exists()).toBe(false);
      const details = view.find('[data-slot=collapsible]');
      expect(details.attributes('data-state')).toBe('closed');
      expect(view.text()).not.toContain('Unexpected server response');
      expect(view.text()).not.toContain('WARNING: engine output');
      await details.get('button').trigger('click');
      // The first button expands the disclosure; Copy details is inside it.
      const copy = details.findAll('button').find(button => button.text() === i18n.global.t('media.detail.copyDetails'))!;
      await copy.trigger('click');
      expect(copied).toContain('ERROR: Unexpected server response at [redacted URL]');
      expect(copied).toContain('main\nWARNING: engine output');
      expect(details.text()).not.toContain('https://example.com/video');
    } finally {
      row.unmount();
      view.unmount();
      i18n.global.locale.value = originalLocale;
    }
  });
});
