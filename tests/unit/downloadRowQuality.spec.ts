import { mount, flushPromises } from '@vue/test-utils';
import { defineComponent, ref } from 'vue';
import { createMemoryHistory, createRouter } from 'vue-router';
import { describe, expect, it, vi } from 'vitest';
import { useDownloadRow } from '../../src/composables/useDownloadRow';
import { useMediaStore } from '../../src/stores/media/media';
import { useMediaProgressStore } from '../../src/stores/media/progress';
import { ProgressCategory, ProgressStage } from '../../src/tauri/types/progress';
import { installTauriMock } from '../utils/tauriMock';
import { phaseOf } from '../../src/helpers/groupView';
import { useMediaDiagnosticsStore } from '../../src/stores/media/diagnostics';
import DownloadRow from '../../src/components/downloads/DownloadRow.vue';
import DownloadInspector from '../../src/components/downloads/DownloadInspector.vue';
import { useDownloadSheetStore } from '../../src/stores/downloadSheet';
import { useSettingsStore } from '../../src/stores/settings';
import { nextTick } from 'vue';
import { i18n } from '../../src/i18n';
import { useMediaGroupStore } from '../../src/stores/media/group';
import { useMediaOptionsStore } from '../../src/stores/media/options';
import { MediaState, useMediaStateStore } from '../../src/stores/media/state';
import { TrackType, type MediaFormat } from '../../src/tauri/types/media';
import { useUiPrefs } from '../../src/composables/useUiPrefs';
import { useToastStore } from '../../src/stores/toast';
import { useMediaSizeStore } from '../../src/stores/media/size';

const f = (id: string, height: number, fps: number): MediaFormat => ({ id, height, fps, audioCodecs: ['mp4a'], videoCodecs: ['h264'] });

describe('inspector quick choices', () => {
  async function inspectorAt(heights: number[]) {
    const formats = heights.map(height => f(`v${height}`, height, 30));
    const item = { id: 'choice-item', isLeader: true, url: 'https://example.com/v', audioCodecs: [], formats, filesize: 0 };
    useMediaGroupStore().createGroup({ ...item, id: 'choices', title: 'Clip', mediaKind: 'video', total: 1, processed: 1, errored: 0, isCombined: false, items: { [item.id]: item } });
    useMediaStateStore().setGroupState('choices', MediaState.configure);
    useDownloadSheetStore().show('choices');
    const router = createRouter({ history: createMemoryHistory(), routes: [{ path: '/', component: { template: '<div />' } }] });
    await router.push('/');
    const panel = mount(DownloadInspector, { global: { plugins: [router, i18n] } });
    await flushPromises();
    return panel;
  }

  it('names the smaller height and explains when best is already the smallest format', async () => {
    const panel = await inspectorAt([720]);
    try {
      const small = panel.get('[role="radio"][value="small"]');
      expect(small.text()).toContain('720p');
      expect(small.text()).toContain('No smaller');
      expect(small.attributes('disabled')).toBeDefined();
    } finally { panel.unmount(); }
  });

  it.each(['en', 'ar', 'fr'] as const)('isolates the reported smaller size beside its choice in %s', async (locale) => {
    const previousLocale = i18n.global.locale.value;
    i18n.global.locale.value = locale;
    const panel = await inspectorAt([1080, 720]);
    try {
      useMediaSizeStore().processMediaSizePayload({ item: { id: 'choice-item', filesize: 2097152 }, format: { trackType: TrackType.both, height: 720, fps: 30 } } as never);
      await nextTick();
      expect(panel.get('[role="radio"][value="small"] bdi[dir=ltr]').text()).toMatch(locale === 'fr' ? /^2,1\sMo$/ : /^2\.1 MB$/);
    } finally {
      panel.unmount();
      i18n.global.locale.value = previousLocale;
    }
  });

  it('explains how to undo remembering a choice only once', async () => {
    const prefs = useUiPrefs();
    prefs.skipOptionsSheet.value = false;
    prefs.rememberNoticeShown.value = false;
    const panel = await inspectorAt([1080, 720]);
    try {
      for (let attempt = 0; attempt < 2; attempt++) {
        useDownloadSheetStore().show('choices');
        await nextTick();
        await panel.get('[role="checkbox"]').trigger('click');
        await panel.findAll('button').find(button => button.text() === 'Download')!.trigger('click');
        await flushPromises();
      }
      expect(useToastStore().toasts.filter(toast => toast.message.includes('Settings'))).toHaveLength(1);
      expect(prefs.skipOptionsSheet.value).toBe(true);
    } finally {
      panel.unmount();
      prefs.skipOptionsSheet.value = false;
      prefs.rememberNoticeShown.value = false;
    }
  });
});

describe('row quality label', () => {
  it.each(['urlNotSupported', 'drmProtected', 'networkNameResolutionFailed'])('offers only useful recovery for %s', async (code) => {
    const groups = useMediaGroupStore();
    groups.createGroup({ id: 'refusal', url: 'https://example.com/v', title: 'Clip', mediaKind: 'video', total: 1, processed: 1, errored: 1, isCombined: false,
      audioCodecs: [], formats: [], filesize: 0, items: { item: { id: 'refusal-item', isLeader: true, url: 'https://example.com/v', audioCodecs: [], formats: [], filesize: 0 } } });
    useMediaStateStore().setGroupState('refusal', MediaState.error);
    useMediaDiagnosticsStore().processMediaDiagnosticPayload({ id: 'refusal-item', groupId: 'refusal', level: 'error', code, component: null, message: '', raw: '', timestamp: 1 });
    const router = createRouter({ history: createMemoryHistory(), routes: [{ path: '/', component: { template: '<div />' } }] });
    await router.push('/');
    const row = mount(DownloadRow, { props: { group: groups.findGroupById('refusal')! }, global: { plugins: [router, i18n] } });
    try {
      const labels = row.findAll('button').map(button => button.text());
      expect(labels.some(label => label.includes('Try again'))).toBe(code === 'networkNameResolutionFailed');
      if (code === 'urlNotSupported') {
        const open = row.findAll('button').find(button => button.text().includes('Open in browser'));
        expect(open).toBeDefined();
      }
    } finally { row.unmount(); }
  });
  it.each([
    ['best', { height: 1080, fps: 60 }, '1080p60'],
    ['small', { height: 720, fps: 60 }, '720p60'],
    ['best', { height: 1080, fps: 30 }, '1080p'],
  ] as const)('a %s pick names the frame rate it saves (%j -> %s)', async (_choice, picked, label) => {
    const groups = useMediaGroupStore();
    const id = `q-${label}`;
    const formats = [f('v1080', 1080, picked.height === 1080 ? picked.fps : 30), f('v720', 720, picked.height === 720 ? picked.fps : 30)];
    groups.createGroup({ id, url: 'https://example.com/v', title: 'Clip', mediaKind: 'video', total: 1, processed: 1, errored: 0, isCombined: false,
      audioCodecs: [], formats, filesize: 0, items: { item: { id: `${id}-item`, isLeader: true, url: 'https://example.com/v', audioCodecs: [], formats: [], filesize: 0 } } });
    useMediaOptionsStore().setOptions(id, { trackType: TrackType.both, ...picked });
    useMediaStateStore().setGroupState(id, MediaState.done);
    const router = createRouter({ history: createMemoryHistory(), routes: [{ path: '/', component: { template: '<div />' } }] });
    await router.push('/');
    const row = mount(DownloadRow, { props: { group: groups.findGroupById(id)! }, global: { plugins: [router, i18n] } });
    try {
      expect(row.text()).toContain(label);
      if (label === '1080p') expect(row.text()).not.toContain('1080p30');
    } finally {
      row.unmount();
    }
  });
});

describe('row phase mapping', () => {
  it.each([
    [MediaState.fetching, 'reading'], [MediaState.fetchingList, 'reading'], [MediaState.playlistSelection, 'playlist'],
    [MediaState.configure, 'ready'], [MediaState.downloading, 'downloading'], [MediaState.downloadingList, 'downloading'],
    [MediaState.paused, 'paused'], [MediaState.pausedList, 'paused'], [MediaState.done, 'done'], [MediaState.error, 'error'],
  ] as const)('%s -> %s', (state, phase) => {
    expect(phaseOf(state)).toBe(phase);
  });
});

describe('row login hint', () => {
  function needsLogin(id: string, code: string | undefined, state: MediaState) {
    const groups = useMediaGroupStore();
    groups.createGroup({ id, url: 'https://example.com/v', title: 'Clip', mediaKind: 'video', total: 1, processed: 1, errored: 0, isCombined: false,
      audioCodecs: [], formats: [], filesize: 0, items: { item: { id: `${id}-item`, isLeader: true, url: 'https://example.com/v', audioCodecs: [], formats: [], filesize: 0 } } });
    useMediaStateStore().setGroupState(id, state);
    if (code) useMediaDiagnosticsStore().processMediaDiagnosticPayload({ id: `${id}-item`, groupId: id, level: 'error', code, component: null, message: '', raw: '', timestamp: 1 });
    const host = mount(defineComponent({ setup: () => useDownloadRow(ref(groups.findGroupById(id)!)), render: () => null }), { global: { plugins: [i18n] } });
    try {
      return (host.vm as unknown as { needsLogin: boolean }).needsLogin;
    } finally {
      host.unmount();
    }
  }

  it.each([
    ['signInRequired', MediaState.error, true],
    ['signInRequiredForBotDetection', MediaState.error, true],
    ['signInRequired', MediaState.done, false],
    ['geoBlocked', MediaState.error, false],
    [undefined, MediaState.error, false],
    // a login fixes these too: the row agrees with the capture card's "login" reason
    ['membersOnly', MediaState.error, true],
    ['videoPasswordRequired', MediaState.error, true],
  ] as const)('%s while %s -> %s', (code, state, expected) => {
    expect(needsLogin(`login-${code}-${state}`, code, state)).toBe(expected);
  });
});

describe('row thumbnail privacy', () => {
  it('removes remote images when the proxy is enabled and keeps local previews', async () => {
    const groups = useMediaGroupStore();
    const id = 'proxy-thumbnail';
    groups.createGroup({ id, url: 'https://example.com/v', title: 'Clip', thumbnail: 'https://cdn.example.com/thumb.jpg', mediaKind: 'video', total: 1, processed: 1, errored: 0, isCombined: false,
      audioCodecs: [], formats: [], filesize: 0, items: {} });
    useMediaStateStore().setGroupState(id, MediaState.done);
    const settings = useSettingsStore().settings.network;
    const before = { ...settings };
    settings.enableProxy = false;
    const router = createRouter({ history: createMemoryHistory(), routes: [{ path: '/', component: { template: '<div />' } }] });
    await router.push('/');
    const group = groups.findGroupById(id)!;
    const row = mount(DownloadRow, { props: { group }, global: { plugins: [router, i18n] } });
    try {
      expect(row.find('img').exists()).toBe(true);
      settings.enableProxy = true;
      settings.proxy = '';
      await nextTick();
      expect(row.find('img').exists()).toBe(false);
      group.thumbnail = 'data:image/png;base64,AA==';
      await nextTick();
      expect(row.get('img').attributes('src')).toBe(group.thumbnail);
    } finally {
      row.unmount();
      Object.assign(settings, before);
    }
  });
});

describe('inspector thumbnail privacy', () => {
  it('hides the remote thumbnail while the proxy is enabled', async () => {
    const groups = useMediaGroupStore();
    const id = 'proxy-thumbnail-inspector';
    groups.createGroup({ id, url: 'https://example.com/v2', title: 'Clip', thumbnail: 'https://cdn.example.com/thumb.jpg', mediaKind: 'video', total: 1, processed: 1, errored: 0, isCombined: false,
      audioCodecs: [], formats: [], filesize: 0, items: {} });
    useMediaStateStore().setGroupState(id, MediaState.done);
    useDownloadSheetStore().show(id);
    const settings = useSettingsStore().settings.network;
    const before = { ...settings };
    settings.enableProxy = false;
    const router = createRouter({ history: createMemoryHistory(), routes: [{ path: '/', component: { template: '<div />' } }] });
    await router.push('/');
    const inspector = mount(DownloadInspector, { global: { plugins: [router, i18n] } });
    try {
      await nextTick();
      expect(inspector.find('figure img').exists()).toBe(true);
      settings.enableProxy = true;
      await nextTick();
      expect(inspector.find('figure img').exists()).toBe(false);
    } finally {
      inspector.unmount();
      Object.assign(settings, before);
    }
  });
});

describe('inspector playlist selection', () => {
  it('lets a captured playlist choose a subset before any download', async () => {
    const expand = vi.fn();
    const download = vi.fn();
    const size = vi.fn();
    installTauriMock({ media_playlist_expand: expand, media_download: download, media_size: size });
    const entries = [{ index: 0, videoUrl: 'https://example.com/one' }, { index: 1, videoUrl: 'https://example.com/two' }];
    const item = { id: 'playlist-leader', isLeader: true, url: 'https://example.com/list', audioCodecs: [], formats: [], filesize: 0, entries };
    const groups = useMediaGroupStore();
    groups.createGroup({ ...item, id: 'capture-playlist', title: 'Playlist', total: 2, processed: 0, errored: 0, isCombined: false, items: { [item.id]: item } });
    useMediaStateStore().setGroupState('capture-playlist', MediaState.fetching);
    const router = createRouter({ history: createMemoryHistory(), routes: [{ path: '/', component: { template: '<div />' } }] });
    await router.push('/');
    const row = mount(DownloadRow, { props: { group: groups.findGroupById('capture-playlist')! }, global: { plugins: [router, i18n] } });
    await flushPromises();
    useDownloadSheetStore().show('capture-playlist');
    const inspector = mount(DownloadInspector, { global: { plugins: [router, i18n] } });
    try {
      await nextTick();
      useSettingsStore().settings.performance.autoLoadSize = true;
      useMediaOptionsStore().setOptions('capture-playlist', { trackType: TrackType.both, height: 720 });
      useMediaStateStore().setGroupState('capture-playlist', MediaState.playlistSelection);
      await flushPromises();
      expect(useDownloadSheetStore().open).toBe(true);
      expect(expand).not.toHaveBeenCalled();
      expect(download).not.toHaveBeenCalled();
      expect(size).not.toHaveBeenCalled();
      await inspector.get('#inspector-playlist-selection-item-1').trigger('click');
      expect(row.find('[role=region]').exists()).toBe(false);
      expect(row.text()).toContain('1 of 2 item selected');
      expect(inspector.get('#inspector-playlist-selection-item-1').attributes('aria-checked')).toBe('false');
      expect(inspector.findAll('button').filter(button => button.text().startsWith('Download'))).toHaveLength(1);
      const none = inspector.findAll('button').find(button => button.text() === 'Select none')!;
      await none.trigger('click');
      const emptyAction = inspector.findAll('button').find(button => button.text() === 'Download 0 videos')!;
      expect(emptyAction.attributes('disabled')).toBeDefined();
      expect(row.text()).toContain('No items selected');
      await inspector.get('#inspector-playlist-selection-item-0').trigger('click');
      const apply = inspector.findAll('button').find(button => button.text() === 'Download 1 video');
      await apply!.trigger('click');
      expect(expand).toHaveBeenCalledWith('media_playlist_expand', expect.objectContaining({ groupId: 'capture-playlist', entries: [entries[0]] }));
      expect(download).not.toHaveBeenCalled();
    } finally {
      row.unmount();
      inspector.unmount();
    }
  });
});
describe('row queue label', () => {
  it('keeps the Arabic remaining-time clock in LTR order beside Latin speed units', async () => {
    const previousLocale = i18n.global.locale.value;
    i18n.global.locale.value = 'ar';
    const groups = useMediaGroupStore();
    groups.createGroup({ id: 'arabic-progress', url: 'https://example.com/v', total: 1, processed: 1, errored: 0, isCombined: false,
      audioCodecs: [], formats: [], filesize: 0, items: { item: { id: 'item', isLeader: true, url: 'https://example.com/v', audioCodecs: [], formats: [], filesize: 0 } } });
    useMediaStateStore().setGroupState('arabic-progress', MediaState.downloading);
    useMediaProgressStore().processMediaProgressPayload({ id: 'item', groupId: 'arabic-progress', category: ProgressCategory.video, percentage: 42, speedBps: 3_400_000, etaSecs: 75 });
    useMediaProgressStore().processMediaProgressStagePayload({ id: 'item', groupId: 'arabic-progress', stage: ProgressStage.downloading });
    const host = mount(defineComponent({ setup: () => useDownloadRow(ref(groups.findGroupById('arabic-progress')!)), render: () => null }), { global: { plugins: [i18n] } });
    try {
      const vm = host.vm as unknown as { metaLine: string[] };
      expect(vm.metaLine).toContain('3.4 MB/s');
      expect(vm.metaLine).toContain('\u20661:15\u2069 متبقٍّ');
    } finally {
      host.unmount();
      i18n.global.locale.value = previousLocale;
    }
  });
  it.each([[false, 'en', 'Queued', 'Initializing'], [true, 'ar', 'في الانتظار', 'جارٍ التهيئة'], [false, 'fr', 'En attente', 'Initialisation']] as const)('labels a waiting row until a job starts (playlist: %s, locale: %s)', async (combined, locale, queued, initializing) => {
    const previousLocale = i18n.global.locale.value;
    i18n.global.locale.value = locale;
    installTauriMock({});
    const groups = useMediaGroupStore();
    const group = { id: 'waiting', url: 'https://example.com/v', total: combined ? 2 : 1, processed: 1, errored: 0, isCombined: combined,
      audioCodecs: [], formats: [], filesize: 0, items: {
        leader: { id: 'leader', isLeader: true, url: 'https://example.com/v', audioCodecs: [], formats: [], filesize: 0,
          ...(combined ? { entries: [{ index: 0, videoUrl: 'https://example.com/child' }] } : {}) },
        ...(combined ? { child: { id: 'child', url: 'https://example.com/child', audioCodecs: [], formats: [], filesize: 0 } } : {}),
      } };
    groups.createGroup(group);
    const host = mount(defineComponent({ setup: () => useDownloadRow(ref(groups.findGroupById(group.id)!)), render: () => null }), { global: { plugins: [i18n] } });
    try {
      await useMediaStore().downloadGroup(group.id, { trackType: TrackType.both });
      await nextTick();
      const vm = host.vm as unknown as { metaLine: string[]; percent: number | null };
      expect(vm.metaLine).toContain(queued);
      expect(vm.metaLine).not.toContain(initializing);
      expect(vm.percent).toBeNull();
      useMediaProgressStore().processMediaProgressStagePayload({ id: combined ? 'child' : 'leader', groupId: group.id, stage: ProgressStage.initializing });
      await nextTick();
      expect(vm.metaLine).not.toContain(queued);
      if (!combined) expect(vm.metaLine).toContain(initializing);
    } finally {
      host.unmount();
      i18n.global.locale.value = previousLocale;
    }
  });
});

describe('retained download progress', () => {
  it('shows playlist completion between children waiting for a slot', async () => {
    installTauriMock({});
    const groups = useMediaGroupStore();
    groups.createGroup({ id: 'between-children', url: 'https://example.com/list', total: 2, processed: 2, errored: 0, isCombined: true,
      audioCodecs: [], formats: [], filesize: 0, items: {
        leader: { id: 'leader', isLeader: true, entries: [], url: 'https://example.com/list', audioCodecs: [], formats: [], filesize: 0 },
        first: { id: 'first', url: 'https://example.com/first', audioCodecs: [], formats: [], filesize: 0 },
        second: { id: 'second', url: 'https://example.com/second', audioCodecs: [], formats: [], filesize: 0 },
      } });
    const states = useMediaStateStore();
    states.setGroupState('between-children', MediaState.downloadingList);
    states.setState('first', MediaState.done);
    states.setState('second', MediaState.downloadingList);
    useMediaProgressStore().processMediaProgressStagePayload({ id: 'second', groupId: 'between-children', stage: ProgressStage.queued });
    const host = mount(defineComponent({ setup: () => useDownloadRow(ref(groups.findGroupById('between-children'))), render: () => null }), { global: { plugins: [i18n] } });
    try {
      const vm = host.vm as unknown as { percent: number | null; metaLine: string[] };
      expect(vm.percent).toBe(50);
      expect(vm.metaLine).toContain('1/2');
      expect(vm.metaLine).not.toContain('Queued');
    } finally { host.unmount(); }
  });

  it('retains a paused percentage while queued and resuming, then accepts new progress', async () => {
    installTauriMock({});
    const groups = useMediaGroupStore();
    groups.createGroup({ id: 'resumed', url: 'https://example.com/video', total: 1, processed: 1, errored: 0, isCombined: false,
      audioCodecs: [], formats: [], filesize: 0, items: { item: { id: 'item', isLeader: true, url: 'https://example.com/video', audioCodecs: [], formats: [], filesize: 0 } } });
    const progress = useMediaProgressStore();
    const states = useMediaStateStore();
    states.setGroupState('resumed', MediaState.paused);
    progress.processMediaProgressStagePayload({ id: 'item', groupId: 'resumed', stage: ProgressStage.downloading });
    progress.processMediaProgressPayload({ id: 'item', groupId: 'resumed', category: ProgressCategory.video, percentage: 36 });
    const host = mount(defineComponent({ setup: () => useDownloadRow(ref(groups.findGroupById('resumed'))), render: () => null }), { global: { plugins: [i18n] } });
    try {
      const vm = host.vm as unknown as { percent: number | null; metaLine: string[] };
      expect(vm.percent).toBe(36);
      await useMediaStore().downloadGroup('resumed', { trackType: TrackType.both });
      await nextTick();
      expect(progress.findDownloadProgress('resumed')?.percentage).toBe(36);
      expect(vm.percent).toBe(36);
      expect(vm.metaLine).toContain('Queued');
      progress.processMediaProgressStagePayload({ id: 'item', groupId: 'resumed', stage: ProgressStage.initializing });
      await nextTick();
      expect(vm.percent).toBe(36);
      expect(vm.metaLine).toContain('Resuming…');
      progress.processMediaProgressStagePayload({ id: 'item', groupId: 'resumed', stage: ProgressStage.downloading });
      progress.processMediaProgressPayload({ id: 'item', groupId: 'resumed', category: ProgressCategory.video, percentage: 40 });
      await nextTick();
      expect(vm.percent).toBe(40);
      expect(vm.metaLine).not.toContain('Resuming…');
    } finally { host.unmount(); }
  });
});

describe('row cancellation controls', () => {
  it.each(['button', 'Delete'] as const)('cancels immediately using %s', async (gesture) => {
    const cancel = vi.fn();
    installTauriMock({ group_cancel: cancel });
    const groups = useMediaGroupStore();
    groups.createGroup({ id: 'cancel-row', url: 'https://example.com/v', total: 1, processed: 1, errored: 0, isCombined: false,
      audioCodecs: [], formats: [], filesize: 0, items: { item: { id: 'item', isLeader: true, url: 'https://example.com/v', audioCodecs: [], formats: [], filesize: 0 } } });
    useMediaStateStore().setGroupState('cancel-row', MediaState.downloading);
    const router = createRouter({ history: createMemoryHistory(), routes: [{ path: '/', component: { template: '<div />' } }] });
    await router.push('/');
    const row = mount(DownloadRow, { props: { group: groups.findGroupById('cancel-row') }, global: { plugins: [router, i18n] } });
    try {
      expect(row.find('button[aria-label="Pause"]').exists()).toBe(true);
      if (gesture === 'button') await row.get('button[aria-label="Cancel download"]').trigger('click');
      else await row.get('li').trigger('keydown', { key: 'Delete' });
      expect(cancel).toHaveBeenCalledWith('group_cancel', { groupId: 'cancel-row', discardFiles: true });
      expect(groups.findGroupById('cancel-row')).toBeUndefined();
    } finally { row.unmount(); }
  });
});
