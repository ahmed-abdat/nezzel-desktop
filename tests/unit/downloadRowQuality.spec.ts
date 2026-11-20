import HomeView from '../../src/views/app/HomeView.vue';
import { downloadSearchQuery } from '../../src/composables/useDownloadList';
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
import { useMediaDestinationStore } from '../../src/stores/media/destination';
import { siteLogo } from '../../src/helpers/siteLogo';

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
  it.each([['en', 103, 'Trimmed 1:43 (SponsorBlock)'], ['ar', 103, 'قُصّ \u20661:43\u2069 (SponsorBlock)'], ['fr', 103, '1:43 retirées (SponsorBlock)'], ['en', undefined, undefined]])('SponsorBlock note in %s for %s s', async (locale, secs, copy) => {
    i18n.global.locale.value = locale as 'en' | 'ar' | 'fr';
    const groups = useMediaGroupStore();
    const id = `sb-${locale}-${secs}`;
    groups.createGroup({ id, url: 'https://example.com/v', title: 'Clip', mediaKind: 'video', total: 1, processed: 1, errored: 0, isCombined: false, sponsorRemovedSecs: secs,
      audioCodecs: [], formats: [], filesize: 0, items: { item: { id: `${id}-item`, isLeader: true, url: 'https://example.com/v', audioCodecs: [], formats: [], filesize: 0 } } });
    useMediaStateStore().setGroupState(id, MediaState.done);
    const router = createRouter({ history: createMemoryHistory(), routes: [{ path: '/', component: { template: '<div />' } }] });
    await router.push('/');
    const row = mount(DownloadRow, { props: { group: groups.findGroupById(id)! }, global: { plugins: [router, i18n] } });
    try {
      const note = row.find('[data-testid="row-sponsor-note"]');
      expect(note.exists() ? note.text() : undefined).toBe(copy);
    } finally {
      row.unmount();
      i18n.global.locale.value = 'en';
    }
  });
  it.each([['en', 'Showing the first 100 items'], ['ar', 'أول 100 عنصر فقط'], ['fr', 'Affichage des 100 premiers éléments']])('shows a listing cap in %s', async (locale, copy) => {
    i18n.global.locale.value = locale as 'en' | 'ar' | 'fr';
    const entries = Array.from({ length: 100 }, (_, index) => ({ index, videoUrl: `https://example.com/${index}` }));
    const item = { id: 'cap-item', isLeader: true, url: 'https://example.com/list', audioCodecs: [], formats: [], filesize: 0, entries };
    const group = { ...item, id: 'cap', title: 'Listing', total: 100, processed: 0, errored: 0, isCombined: false, items: { [item.id]: item } };
    useMediaGroupStore().createGroup(group);
    useMediaStateStore().setGroupState('cap', MediaState.playlistSelection);
    const router = createRouter({ history: createMemoryHistory(), routes: [{ path: '/', component: { template: '<div />' } }] });
    await router.push('/');
    const row = mount(DownloadRow, { props: { group }, global: { plugins: [router, i18n] } });
    try {
      expect(row.text()).toContain(copy);
      await row.setProps({ group: { ...group, entries: entries.slice(0, 2) } });
      expect(row.text()).not.toContain(copy);
    } finally {
      row.unmount();
      i18n.global.locale.value = 'en';
    }
  });
  it.each(['urlNotSupported', 'drmProtected', 'networkNameResolutionFailed', 'telegramAppOnly'])('offers only useful recovery for %s', async (code) => {
    const open = vi.fn();
    installTauriMock({ 'plugin:opener|open_url': open });
    const groups = useMediaGroupStore();
    groups.createGroup({ id: 'refusal', url: 'https://t.me/bbcpersian/295504', title: 'Clip', mediaKind: 'video', total: 1, processed: 1, errored: 1, isCombined: false,
      audioCodecs: [], formats: [], filesize: 0, items: { item: { id: 'refusal-item', isLeader: true, url: 'https://example.com/v', audioCodecs: [], formats: [], filesize: 0 } } });
    useMediaStateStore().setGroupState('refusal', MediaState.error);
    useMediaDiagnosticsStore().processMediaDiagnosticPayload({ id: 'refusal-item', groupId: 'refusal', level: 'error', code, component: null, message: '', raw: '', timestamp: 1 });
    const router = createRouter({ history: createMemoryHistory(), routes: [{ path: '/', component: { template: '<div />' } }] });
    await router.push('/');
    const row = mount(DownloadRow, { props: { group: groups.findGroupById('refusal')! }, global: { plugins: [router, i18n] } });
    try {
      const labels = row.findAll('button').map(button => button.text());
      expect(labels.some(label => label.includes('Try again'))).toBe(code === 'networkNameResolutionFailed');
      if (code === 'telegramAppOnly') {
        const button = row.findAll('button').find(button => button.text().includes('Open in Telegram'));
        expect(button).toBeDefined();
        await button!.trigger('click');
        expect(open).toHaveBeenCalledWith('plugin:opener|open_url', expect.objectContaining({ url: 'https://t.me/bbcpersian/295504' }));
      }
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
    [MediaState.paused, 'paused'], [MediaState.pausedList, 'paused'], [MediaState.done, 'done'], [MediaState.error, 'error'], [MediaState.cancelled, 'cancelled'],
  ] as const)('%s -> %s', (state, phase) => {
    expect(phaseOf(state)).toBe(phase);
  });
});

describe('row saved files', () => {
  function mockPhotoVisibility() {
    let callback: IntersectionObserverCallback;
    const observe = vi.fn();
    const disconnect = vi.fn();
    const unobserve = vi.fn();
    vi.stubGlobal('IntersectionObserver', class {
      constructor(cb: IntersectionObserverCallback) { callback = cb; }
      observe = observe;
      unobserve = unobserve;
      disconnect = disconnect;
    });
    const createObjectURL = vi.fn<(blob: Blob) => string>(() => 'blob:saved-photo');
    const revokeObjectURL = vi.fn();
    vi.stubGlobal('URL', class extends URL {
      static createObjectURL = createObjectURL;
      static revokeObjectURL = revokeObjectURL;
    });
    return {
      observe, disconnect, createObjectURL, revokeObjectURL,
      visible(target: Element, isIntersecting: boolean) {
        callback([{ target, isIntersecting } as IntersectionObserverEntry], {} as IntersectionObserver);
      },
    };
  }

  async function savedRow(id: string, paths: string[], confidence = 100) {
    const groups = useMediaGroupStore();
    const item = { id: `${id}-item`, isLeader: true, url: 'https://www.instagram.com/p/x/', audioCodecs: [], formats: [], filesize: 0 };
    groups.createGroup({ ...item, id, title: 'Post', mediaKind: 'carousel', total: 1, processed: 1, errored: 0, isCombined: false, items: { [item.id]: item } });
    useMediaStateStore().setGroupState(id, MediaState.done);
    const destinations = useMediaDestinationStore();
    // An intermediate path the engine reported first, with less confidence: never a saved file.
    destinations.addDestination(id, { path: '/tmp/part', confidence: 60 });
    for (const path of paths) destinations.addDestination(id, { path, confidence });
    const router = createRouter({ history: createMemoryHistory(), routes: [{ path: '/', component: { template: '<div />' } }] });
    await router.push('/');
    return mount(DownloadRow, { props: { group: groups.findGroupById(id)! }, global: { plugins: [router, i18n] } });
  }

  it.each([false, true])('requests only visible saved photos once; rejection=%s keeps the placeholder', async (rejects) => {
    const visibility = mockPhotoVisibility();
    const thumbnail = vi.fn(() => rejects ? Promise.reject('unavailable') : Promise.resolve([255, 216, 255, 217]));
    installTauriMock({ saved_photo_thumbnail: thumbnail });
    const row = await savedRow('files-preview', ['/d/صورة.jpg', '/d/unseen.png', '/d/video.mp4']);
    try {
      expect(thumbnail).not.toHaveBeenCalled();
      await row.get('[data-testid="files-toggle"]').trigger('click');
      const lines = row.findAll('[data-testid="saved-file"]');
      expect(visibility.observe.mock.calls.map(([el]) => el)).toEqual([lines[0].element, lines[1].element]);
      visibility.visible(lines[0].element, false);
      await flushPromises();
      expect(thumbnail).not.toHaveBeenCalled();
      visibility.visible(lines[0].element, true);
      visibility.visible(lines[0].element, true);
      await flushPromises();
      expect(thumbnail).toHaveBeenCalledExactlyOnceWith('saved_photo_thumbnail', { groupId: 'files-preview', path: '/d/صورة.jpg' });
      const box = lines[0].get('[data-testid="saved-file-open"] > span');
      expect(box.find('img').exists()).toBe(!rejects);
      expect(box.find('svg').exists()).toBe(rejects);
      expect(lines[1].find('img').exists()).toBe(false);
      if (!rejects) {
        expect(box.get('img').attributes('src')).toBe('blob:saved-photo');
        expect(box.get('img').attributes('alt')).toBe('صورة.jpg');
        expect(visibility.createObjectURL.mock.calls[0][0]).toMatchObject({ type: 'image/jpeg', size: 4 });
      }
      // Disclosure changes do not fetch the same photo a second time.
      await row.get('[data-testid="files-toggle"]').trigger('click');
      await row.get('[data-testid="files-toggle"]').trigger('click');
      visibility.visible(row.get('[data-testid="saved-file"]').element, true);
      expect(thumbnail).toHaveBeenCalledTimes(1);
    } finally {
      row.unmount();
      expect(visibility.disconnect).toHaveBeenCalledOnce();
      expect(visibility.revokeObjectURL.mock.calls).toEqual(rejects ? [] : [['blob:saved-photo']]);
      vi.unstubAllGlobals();
    }
  });

  it('does not create a blob URL if the photo resolves after unmount', async () => {
    const visibility = mockPhotoVisibility();
    let resolve!: (bytes: number[]) => void;
    installTauriMock({
      saved_photo_thumbnail: () => new Promise<number[]>((done) => { resolve = done; }),
    });
    const row = await savedRow('files-late-preview', ['/d/a.jpg', '/d/b.jpg']);
    try {
      await row.get('[data-testid="files-toggle"]').trigger('click');
      visibility.visible(row.get('[data-testid="saved-file"]').element, true);
      await flushPromises();
      row.unmount();
      resolve([255, 216, 255, 217]);
      await flushPromises();
      expect(visibility.createObjectURL).not.toHaveBeenCalled();
    } finally {
      row.unmount();
      vi.unstubAllGlobals();
    }
  });

  it('lists every saved file in saved order once, for a multi-file row only', async () => {
    const model = (id: string) => {
      let files: unknown;
      const probe = defineComponent({
        setup() {
          files = useDownloadRow(ref(useMediaGroupStore().findGroupById(id)!)).savedFiles.value;
          return () => null;
        },
      });
      mount(probe, { global: { plugins: [i18n] } }).unmount();
      return files;
    };
    const multi = await savedRow('files-multi', ['/d/p 1.jpg', '/d/p 2.jpg', '/d/p 1.jpg', '/d/p 3.mp4']);
    const single = await savedRow('files-single', ['/d/only.jpg']);
    try {
      expect(model('files-multi')).toEqual([
        { path: '/d/p 1.jpg', name: 'p 1.jpg', kind: 'image' },
        { path: '/d/p 2.jpg', name: 'p 2.jpg', kind: 'image' },
        { path: '/d/p 3.mp4', name: 'p 3.mp4', kind: 'video' },
      ]);
      const toggle = multi.get('[data-testid="files-toggle"]');
      expect(toggle.attributes('aria-expanded')).toBe('false');
      expect(multi.findAll('[data-testid="saved-file"]')).toHaveLength(0);
      expect(single.find('[data-testid="files-toggle"]').exists()).toBe(false);
    } finally {
      multi.unmount();
      single.unmount();
    }
  });

  it('never launches a script or installer that came with a post: its folder opens instead', async () => {
    const opened: string[] = [];
    const revealed: string[] = [];
    installTauriMock({
      'plugin:opener|open_path': (_cmd, args) => { opened.push((args as { path: string }).path); },
      'plugin:opener|reveal_item_in_dir': (_cmd, args) => { revealed.push(JSON.stringify(args)); },
    });
    const row = await savedRow('files-unsafe', ['/d/a.jpg', '/d/run.command', '/d/setup.exe']);
    try {
      await row.get('[data-testid="files-toggle"]').trigger('click');
      for (const line of row.findAll('[data-testid="saved-file"]')) {
        await line.get('[data-testid="saved-file-open"]').trigger('click');
        await flushPromises();
      }
      expect(opened).toEqual(['/d/a.jpg']);
      expect(revealed).toHaveLength(2);
      expect(revealed.join()).toContain('run.command');
      expect(revealed.join()).toContain('setup.exe');
    } finally {
      row.unmount();
    }
  });

  it('opens and shows the chosen file, and marks a missing one', async () => {
    const opened: string[] = [];
    installTauriMock({
      'plugin:opener|open_path': (_cmd, args) => { opened.push((args as { path: string }).path); },
      'plugin:opener|reveal_item_in_dir': () => Promise.reject('missing'),
    });
    const row = await savedRow('files-open', ['/d/a.jpg', '/d/b.jpg', '/d/c.jpg']);
    try {
      await row.get('[data-testid="files-toggle"]').trigger('click');
      const lines = row.findAll('[data-testid="saved-file"]');
      expect(lines.map(line => line.text())).toEqual(['a.jpgOpen', 'b.jpgOpen', 'c.jpgOpen']);
      const open = (line: (typeof lines)[number]) => line.findAll('button').find(button => button.text() === 'Open')!;
      await open(lines[1]).trigger('click');
      // The thumbnail and the name open the same file as Open.
      await lines[0].get('[data-testid="saved-file-open"]').trigger('click');
      await flushPromises();
      expect(opened).toEqual(['/d/b.jpg', '/d/a.jpg']);
      // Show in folder cannot find the third file: its line says so and stops offering Open.
      await lines[2].get('[aria-label="Show in folder"]').trigger('click');
      await flushPromises();
      const third = row.findAll('[data-testid="saved-file"]')[2];
      expect(third.text()).toContain('File not found');
      expect(open(third).attributes('disabled')).toBeDefined();
      expect(third.get('[data-testid="saved-file-open"]').attributes('disabled')).toBeDefined();
    } finally { row.unmount(); }
  });

  it('labels a numbered photo as the isolated title and its number, keeping the full name in the tooltip', async () => {
    const name = 'خمس عادات - (1920p) - 002.jpg';
    const row = await savedRow('files-label', [`/d/${name}`, '/d/plain.jpg']);
    try {
      useMediaGroupStore().findGroupById('files-label')!.title = 'خمس عادات';
      await row.get('[data-testid="files-toggle"]').trigger('click');
      const [numbered, plain] = row.findAll('[data-testid="saved-file-name"]');
      const parts = numbered.findAll('bdi');
      expect(parts.map(part => [part.attributes('dir'), part.text()])).toEqual([['auto', 'خمس عادات'], ['ltr', '002']]);
      expect(numbered.text()).not.toContain('1920p');
      expect(row.findAll('[data-testid="saved-file-open"]')[0].attributes('title')).toBe(name);
      // A name without a post number is shown whole, still isolated.
      expect(plain.findAll('bdi').map(part => [part.attributes('dir'), part.text()])).toEqual([['auto', 'plain.jpg']]);
    } finally { row.unmount(); }
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
      // Bilibili and Reddit image hosts refuse hotlinks that carry a foreign Referer.
      expect(row.get('img').attributes('referrerpolicy')).toBe('no-referrer');
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
      expect(inspector.get('figure img').attributes('referrerpolicy')).toBe('no-referrer');
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
    useDownloadSheetStore().show('capture-playlist');
    const inspector = mount(DownloadInspector, { global: { plugins: [router, i18n] } });
    try {
      await nextTick();
      useSettingsStore().settings.performance.autoLoadSize = true;
      useMediaOptionsStore().setOptions('capture-playlist', { trackType: TrackType.both, height: 720 });
      useMediaStateStore().setGroupState('capture-playlist', MediaState.playlistSelection);
      await vi.waitFor(() => expect(inspector.find('#inspector-playlist-selection-item-1').exists()).toBe(true));
      expect(useDownloadSheetStore().open).toBe(true);
      expect(expand).not.toHaveBeenCalled();
      expect(download).not.toHaveBeenCalled();
      expect(size).not.toHaveBeenCalled();
      await inspector.get('#inspector-playlist-selection-item-1').trigger('click');
      await vi.waitFor(() => expect(row.text()).toContain('1 of 2 item selected'));
      expect(row.find('[role=region]').exists()).toBe(false);
      expect(inspector.get('#inspector-playlist-selection-item-1').attributes('aria-checked')).toBe('false');
      expect(inspector.findAll('button').filter(button => button.text().startsWith('Download'))).toHaveLength(1);
      const none = inspector.findAll('button').find(button => button.text() === 'Select none')!;
      await none.trigger('click');
      const emptyAction = inspector.findAll('button').find(button => button.text() === 'Download 0 videos')!;
      expect(emptyAction.attributes('disabled')).toBeDefined();
      await vi.waitFor(() => expect(row.text()).toContain('No items selected'));
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
  it.each(['en', 'ar', 'fr'] as const)('shows saved-file reuse and cancellation in %s', async (locale) => {
    const previous = i18n.global.locale.value;
    i18n.global.locale.value = locale;
    const open = vi.fn();
    const reveal = vi.fn();
    installTauriMock({ 'plugin:opener|open_path': open, 'plugin:opener|reveal_item_in_dir': reveal });
    const groups = useMediaGroupStore();
    const id = 'already-saved';
    const url = 'https://youtu.be/abc';
    groups.createGroup({ id, url, title: 'فيديو — saved clip', mediaKind: 'video', total: 1, processed: 1, errored: 0, isCombined: false,
      audioCodecs: [], formats: [], filesize: 0, items: { item: { id: 'saved-item', isLeader: true, url, audioCodecs: [], formats: [], filesize: 0 } } });
    const states = useMediaStateStore();
    states.setGroupState(id, MediaState.done);
    const destinations = useMediaDestinationStore();
    destinations.addDestination(id, { path: '/saved.mp4', confidence: 100, alreadyDownloaded: true });
    const router = createRouter({ history: createMemoryHistory(), routes: [{ path: '/', component: { template: '<div />' } }] });
    await router.push('/');
    const row = mount(DownloadRow, { props: { group: groups.findGroupById(id) }, global: { plugins: [router, i18n] } });
    try {
      const { t } = i18n.global;
      expect(row.text()).toContain(t('downloads.row.alreadySaved'));
      expect(row.get('figure svg path').attributes('d')).toBe(siteLogo(url));
      const openButton = row.findAll('button').find(button => button.text() === t('downloads.row.open'))!;
      expect(openButton.attributes('disabled')).toBeUndefined();
      await openButton.trigger('click');
      await row.get(`button[aria-label="${t('downloads.row.show')}"]`).trigger('click');
      expect(open).toHaveBeenCalledWith('plugin:opener|open_path', expect.objectContaining({ path: '/saved.mp4' }));
      expect(reveal).toHaveBeenCalled();
      // A multi-file result with even one newly saved file is not an all-reused download.
      destinations.addDestination(id, { path: '/new.mp4', confidence: 100, alreadyDownloaded: false });
      await nextTick();
      expect(row.text()).not.toContain(t('downloads.row.alreadySaved'));
      // A reused stream before a fresh final merge must not label the new file as reused.
      destinations.deleteDestinations(id);
      destinations.addDestination(id, { path: '/stream.mp4', confidence: 90, alreadyDownloaded: true });
      destinations.addDestination(id, { path: '/fresh.mp4', confidence: 100, alreadyDownloaded: false });
      await nextTick();
      expect(row.text()).not.toContain(t('downloads.row.alreadySaved'));
      states.setGroupState(id, MediaState.cancelled);
      await nextTick();
      expect(row.text()).toContain(t('downloads.row.cancelledState'));
      expect(row.text()).toContain(t('downloads.row.retry'));
      expect(row.find(`button[aria-label="${t('downloads.row.remove')}"]`).exists()).toBe(true);
    } finally {
      row.unmount();
      i18n.global.locale.value = previous;
    }
  });

  it('offers Download again only on an already-saved row, and choosing it calls the store', async () => {
    installTauriMock({});
    const groups = useMediaGroupStore();
    const url = 'https://example.com/again';
    for (const id of ['again-saved', 'again-fresh']) {
      groups.createGroup({ id, url, title: id, mediaKind: 'video', total: 1, processed: 1, errored: 0, isCombined: false,
        audioCodecs: [], formats: [], filesize: 0, items: { [`${id}-item`]: { id: `${id}-item`, isLeader: true, url, audioCodecs: [], formats: [], filesize: 0 } } });
      useMediaStateStore().setGroupState(id, MediaState.done);
      useMediaDestinationStore().addDestination(id, { path: `/${id}.mp4`, confidence: 100, alreadyDownloaded: id === 'again-saved' });
    }
    const again = vi.spyOn(useMediaStore(), 'downloadAgain').mockResolvedValue(undefined as never);
    const router = createRouter({ history: createMemoryHistory(), routes: [{ path: '/', component: { template: '<div />' } }] });
    await router.push('/');
    // reka menus do not open in jsdom: render the items inline.
    const passthrough = { template: '<div><slot /></div>' };
    const item = { template: '<button type="button" data-menu-item @click="$emit(\'select\')"><slot /></button>', emits: ['select'] };
    const stubs = { DropdownMenu: passthrough, DropdownMenuTrigger: passthrough, DropdownMenuContent: passthrough, DropdownMenuSeparator: passthrough, DropdownMenuItem: item,
      ContextMenu: passthrough, ContextMenuTrigger: passthrough, ContextMenuContent: passthrough, ContextMenuSeparator: passthrough, ContextMenuItem: item };
    const label = i18n.global.t('downloads.row.downloadAgain');
    const menuItems = (row: ReturnType<typeof mount>) => row.findAll('[data-menu-item]').filter(entry => entry.text().includes(label));
    const saved = mount(DownloadRow, { props: { group: groups.findGroupById('again-saved')! }, global: { plugins: [router, i18n], stubs } });
    const fresh = mount(DownloadRow, { props: { group: groups.findGroupById('again-fresh')! }, global: { plugins: [router, i18n], stubs } });
    try {
      expect(menuItems(fresh)).toHaveLength(0);
      expect(menuItems(saved).length).toBeGreaterThan(0);
      await menuItems(saved)[0].trigger('click');
      expect(again).toHaveBeenCalledWith('again-saved');
    } finally {
      saved.unmount();
      fresh.unmount();
    }
  });

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
      const retained = groups.orderedGroups[0];
      await row.setProps({ group: retained });
      expect(row.get('li').attributes('data-phase')).toBe('cancelled');
      expect(row.text()).toContain('Cancelled');
      expect(row.find('button[aria-label="Pause"]').exists()).toBe(false);
      expect(row.find('button[aria-label="Remove"]').exists()).toBe(true);
      const retry = vi.spyOn(useMediaStore(), 'retryGroup').mockResolvedValue();
      await row.findAll('button').find(button => button.text().includes('Try again'))!.trigger('click');
      expect(retry).toHaveBeenCalledWith(retained.id);
    } finally { row.unmount(); }
  });
});

describe('list view search lifetime', () => {
  it('reveals and briefly highlights a duplicate through filters, and still renders cancelled rows', async () => {
    vi.useFakeTimers();
    const groups = useMediaGroupStore();
    const states = useMediaStateStore();
    const id = 'focus-existing';
    const url = 'https://instagram.com/p/ABC?igsh=first';
    groups.createGroup({ id, url, title: 'Same post', mediaKind: 'image', total: 1, processed: 1, errored: 0, isCombined: false,
      audioCodecs: [], formats: [], filesize: 0, items: { item: { id: 'focus-item', isLeader: true, url, audioCodecs: [], formats: [], filesize: 0 } } });
    states.setGroupState(id, MediaState.configure);
    const router = createRouter({ history: createMemoryHistory(), routes: [{ name: 'home', path: '/', component: HomeView }] });
    await router.push('/?type=audio');
    const view = mount(HomeView, { attachTo: document.body, global: { plugins: [router, i18n] } });
    try {
      downloadSearchQuery.value = 'hidden by search';
      await useMediaStore().dispatchMediaInfoFetch('https://www.instagram.com/p/ABC/?igsh=second');
      await flushPromises();
      const row = view.get(`li[data-group-id="${id}"]`);
      expect(row.classes()).toContain('bg-primary-soft');
      expect(document.activeElement).toBe(row.element);
      expect(router.currentRoute.value.query).toEqual({});
      expect(downloadSearchQuery.value).toBe('');
      await vi.advanceTimersByTimeAsync(1200);
      expect(row.classes()).not.toContain('bg-primary-soft');
      states.setGroupState(id, MediaState.cancelled);
      await nextTick();
      expect(view.get('li[data-phase=cancelled]').text()).toContain('Cancelled');
    } finally {
      view.unmount();
      vi.useRealTimers();
    }
  });

  it('clears search when sidebar type/status or screen changes', async () => {
    const router = createRouter({ history: createMemoryHistory(), routes: [{ name: 'home', path: '/', component: HomeView }, { name: 'settings', path: '/settings', component: { template: '<div />' } }] });
    await router.push('/');
    const view = mount(HomeView, { global: { plugins: [router, i18n] } });
    try {
      for (const path of ['/?type=audio', '/?status=failed', '/settings']) {
        downloadSearchQuery.value = 'old search';
        await router.push(path);
        await nextTick();
        expect(downloadSearchQuery.value).toBe('');
        downloadSearchQuery.value = 'keep search';
        await router.push(`${path}${path.includes('?') ? '&' : '?'}unrelated=1`);
        await nextTick();
        expect(downloadSearchQuery.value).toBe('keep search');
      }
      downloadSearchQuery.value = 'leaving';
    } finally { view.unmount(); }
    expect(downloadSearchQuery.value).toBe('');
  });
});

it.each(['en', 'ar', 'fr'] as const)('shows retry waits instead of stale progress in %s', async (locale) => {
  const previous = i18n.global.locale.value;
  i18n.global.locale.value = locale;
  const item = { id: 'waiting-item', isLeader: true, url: 'https://example.com/v', audioCodecs: [], formats: [], filesize: 0 };
  const group = { ...item, id: 'waiting', total: 1, processed: 1, errored: 0, isCombined: false, items: { [item.id]: item } };
  useMediaGroupStore().createGroup(group);
  useMediaStateStore().setState(item.id, MediaState.downloading);
  let row!: ReturnType<typeof useDownloadRow>;
  const wrapper = mount(defineComponent({ setup() {
    row = useDownloadRow(ref(group));
    return () => null;
  } }), { global: { plugins: [i18n] } });
  try {
    const progress = useMediaProgressStore();
    progress.processMediaProgressPayload({ id: item.id, groupId: group.id, category: ProgressCategory.video, percentage: 63, etaSecs: 30 });
    progress.processMediaProgressStagePayload({ id: item.id, groupId: group.id, stage: ProgressStage.waitingNetwork });
    expect(row.metaLine.value).toContain(i18n.global.t('reliability.waitingNetwork'));
    progress.processMediaProgressStagePayload({ id: item.id, groupId: group.id, stage: ProgressStage.siteBusy });
    expect(row.metaLine.value.join(' ')).toContain('30');
    expect(row.metaLine.value.join(' ')).not.toContain('63%');
  } finally {
    wrapper.unmount();
    i18n.global.locale.value = previous;
  }
});
