import { flushPromises } from '@vue/test-utils';
import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';
import { clearMocks } from '@tauri-apps/api/mocks';
import { installTauriMock } from '../utils/tauriMock.ts';
import { useMediaStore } from '../../src/stores/media/media';
import { useMediaGroupStore } from '../../src/stores/media/group';
import { useMediaDestinationStore } from '../../src/stores/media/destination';
import { useMediaOptionsStore } from '../../src/stores/media/options';
import { useMediaProgressStore } from '../../src/stores/media/progress';
import { ProgressStage } from '../../src/tauri/types/progress';
import { useMediaStateStore, MediaState } from '../../src/stores/media/state';
import { useSettingsStore } from '../../src/stores/settings';
import { useWatchClipboardStore } from '../../src/stores/watchClipboard';
import { useUiPrefs } from '../../src/composables/useUiPrefs';
import { useLoginStore } from '../../src/stores/login';
import type { LoginUsing } from '../../src/tauri/types/login';
import { defaultSettings } from '../../src/tauri/types/config';
import type { EntryItem, MediaAddPayload } from '../../src/tauri/types/media';
import { TrackType } from '../../src/tauri/types/media';
import type { Group } from '../../src/tauri/types/group';

const { notify, notifyGroup } = vi.hoisted(() => ({
  notify: vi.fn(),
  notifyGroup: vi.fn(),
}));

vi.mock('../../src/tauri/notifications', () => ({
  notify,
  notifyGroup,
}));

function createEntries(total: number, prefix = 'https://example.com/playlist'): EntryItem[] {
  return Array.from({ length: total }, (_, index) => ({
    index,
    videoUrl: `${prefix}?v=${index + 1}`,
  }));
}

function createPlaylistGroup(groupId: string, total: number): Group {
  const entries = createEntries(total);
  return {
    id: groupId,
    url: 'https://example.com/playlist',
    title: 'Playlist',
    total,
    processed: 0,
    errored: 0,
    isCombined: false,
    audioCodecs: [],
    videoCodecs: [],
    audioTracks: [],
    videoTracks: [],
    formats: [],
    filesize: 0,
    entries,
    items: {
      leader: {
        id: 'leader',
        isLeader: true,
        url: 'https://example.com/playlist',
        title: 'Playlist',
        audioCodecs: [],
        videoCodecs: [],
        audioTracks: [],
        videoTracks: [],
        formats: [],
        filesize: 0,
        entries: [...entries],
        playlistCount: total,
      },
    },
  };
}

describe('media store', () => {
  it('cancels immediately through removal and Undo requeues the saved group with every option', async () => {
    const requests: { cmd: string; args: unknown }[] = [];
    installTauriMock({
      group_cancel: (cmd, args) => { requests.push({ cmd, args }); },
      media_download: (cmd, args) => { requests.push({ cmd, args }); },
    });
    const groups = useMediaGroupStore();
    const store = useMediaStore();
    const options = useMediaOptionsStore();
    const group = createPlaylistGroup('cancel-me', 1);
    delete group.items.leader.entries;
    group.items.leader.attachments = [{ id: 'photo', kind: 'image', url: 'https://example.com/photo.jpg' }];
    group.attachments = group.items.leader.attachments;
    groups.createGroup(group);
    useMediaStateStore().setGroupState(group.id, MediaState.downloading);
    const chosen = { trackType: TrackType.both, height: 720 };
    options.setEncodings(group.id, { audio: 'aac', video: 'h264' });
    options.setTracks(group.id, { audio: 'en', video: 'main' });
    options.setOverrides(group.id, { output: { downloadDir: '/chosen' } });
    options.setAttachmentSelection(group.id, ['photo']);
    await store.downloadGroup(group.id, chosen);
    const originalRequest = requests.at(-1)!.args as { items: unknown[] };
    requests.length = 0;

    expect(useMediaProgressStore().findDownloadProgress(group.id)?.stage).toBe(ProgressStage.queued);
    const undo = store.cancelDownload(group.id)!;
    expect(groups.findGroupById(group.id)).toBeUndefined();
    expect(options.getOptions(group.id)).toBeUndefined();
    expect(requests).toEqual([{ cmd: 'group_cancel', args: { groupId: group.id, discardFiles: true } }]);
    // Mutating the old reference cannot change the saved cancellation snapshot.
    group.title = 'Changed after cancel';
    expect(await undo()).toBe(true);
    const restored = groups.orderedGroups[0];
    expect(restored).toMatchObject({ title: 'Playlist', url: group.url });
    expect(restored.id).not.toBe(group.id);
    expect(Object.values(restored.items)[0].id).not.toBe('leader');
    expect(requests.at(-1)).toMatchObject({ cmd: 'media_download', args: { groupId: restored.id, items: originalRequest.items.map(item => ({ ...(item as object), id: expect.any(String) })) } });
    expect(options.getOptions(restored.id)).toEqual({ trackType: TrackType.both, height: 720 });
    expect(options.getAttachmentSelection(restored.id)).toEqual(['photo']);
    // Late events from the cancelled generation cannot complete the replacement.
    useMediaProgressStore().processMediaCompletePayload({ id: 'leader', groupId: group.id });
    expect(useMediaStateStore().getGroupState(restored.id)).toBe(MediaState.downloading);
    await undo();
    expect(requests.filter(r => r.cmd === 'media_download')).toHaveLength(1);
  });

  it('Undo keeps three finished playlist items and submits only the seven unfinished items', async () => {
    const download = vi.fn();
    installTauriMock({ media_download: download });
    const groups = useMediaGroupStore();
    const states = useMediaStateStore();
    const destinations = useMediaDestinationStore();
    const group = createPlaylistGroup('partial-playlist', 10);
    group.isCombined = true;
    group.processed = 10;
    for (let index = 0; index < 10; index++) {
      const id = `child-${index}`;
      group.items[id] = { id, url: group.entries![index].videoUrl, audioCodecs: [], formats: [], filesize: 0 };
    }
    groups.createGroup(group);
    states.setGroupState(group.id, MediaState.downloadingList);
    for (let index = 0; index < 10; index++) {
      states.setState(`child-${index}`, index < 3 ? MediaState.done : MediaState.downloadingList);
      if (index < 3) destinations.addDestination(group.id, { path: `/downloads/${index}.mp4`, confidence: 100 });
    }
    const savedDestinations = [...destinations.getDestinations(group.id)];
    const undo = useMediaStore().cancelDownload(group.id)!;
    await undo();
    const restored = groups.orderedGroups[0];
    expect(download).toHaveBeenCalledTimes(1);
    const submitted = download.mock.calls[0][1] as { items: { url: string }[] };
    expect(submitted.items.map(item => item.url)).toEqual(group.entries!.slice(3).map(entry => entry.videoUrl));
    const finished = Object.values(restored.items).filter(item => states.getState(item.id) === MediaState.done);
    expect(finished).toHaveLength(3);
    expect(destinations.getDestinations(restored.id)).toEqual(savedDestinations);
    expect(useMediaProgressStore().findGroupProgress(restored.id)).toMatchObject({ done: 3, total: 10 });
  });

  it('keeps successful links and puts rejected links in error', async () => {
    installTauriMock({ media_info: (_cmd, args) => {
      if ((args as { url: string }).url.endsWith('/bad')) throw new Error('queue closed');
      return 'ok';
    } });
    const ids = await useMediaStore().addUrlBatch(['https://example.com/bad', 'https://example.com/good']);
    const groups = useMediaGroupStore();
    expect(ids).toHaveLength(1);
    expect(groups.findGroupById(ids[0]).url).toBe('https://example.com/good');
    expect(groups.orderedGroups).toHaveLength(2);
    const bad = groups.orderedGroups.find(g => g.url?.endsWith('/bad'))!;
    expect(useMediaStateStore().getGroupState(bad.id)).toBe(MediaState.error);
  });

  it('returns the options-sheet ID while the notification is still pending', async () => {
    installTauriMock({ media_info: () => 'ok' });
    let finish!: () => void;
    notifyGroup.mockReturnValue(new Promise<void>((resolve) => {
      finish = resolve;
    }));
    let id: string | undefined;
    const request = useMediaStore().dispatchMediaInfoFetch('https://example.com/a').then((value) => {
      id = value;
    });
    await flushPromises();
    try {
      expect(id).toBeTypeOf('string');
    } finally {
      finish();
      await request;
    }
  });

  it('loads history as done groups below session rows and appends scan results without moving rows', async () => {
    let finishScan!: (rows: unknown[]) => void;
    const record = (id: string, path: string) => ({
      id, paths: [path], url: 'https://example.com/video', title: 'فيديو',
      site: 'Example', kind: 'video', size: 128, completedAt: 100, thumbnail: null,
    });
    const first = record('saved', '/downloads/فيديو.mp4');
    installTauriMock({
      library_list: (_cmd, args) => (args as { scan: boolean }).scan
        ? new Promise((resolve) => { finishScan = resolve; })
        : [first],
    });
    const store = useMediaStore();
    const session = await store.dispatchMediaInfoFetch('https://example.com/current');
    const loading = store.loadLibrary();
    await flushPromises();
    const groups = useMediaGroupStore();
    expect(groups.groupOrder).toEqual([session, 'library:saved']);
    expect(useMediaStateStore().getGroupState('library:saved')).toBe(MediaState.done);
    expect(groups.findGroupById('library:saved')).toMatchObject({ title: 'فيديو', mediaKind: 'video', filesize: 128 });
    expect(useMediaDestinationStore().getPrimaryDestination('library:saved')?.path).toBe('/downloads/فيديو.mp4');
    const newer = await store.dispatchMediaInfoFetch('https://example.com/new');
    finishScan([first, record('scan', '/downloads/scan.mp4')]);
    await loading;
    expect(groups.groupOrder).toEqual([newer, session, 'library:saved', 'library:scan']);
  });

  it('dedupes Windows paths against session rows and suppresses a removed row from an in-flight scan', async () => {
    let finishScan!: (rows: unknown[]) => void;
    const first = { id: 'saved', paths: ['C:/Downloads/clip.mp4'], url: '', title: 'clip', kind: 'video', size: 1, completedAt: 1 };
    const second = { ...first, id: 'other', paths: ['C:/Downloads/other.mp4'] };
    const removed: unknown[] = [];
    installTauriMock({
      library_list: (_cmd, args) => (args as { scan: boolean }).scan
        ? new Promise((resolve) => { finishScan = resolve; })
        : [first, second],
      library_remove: (_cmd, args) => { removed.push(args); },
    });
    const store = useMediaStore();
    const session = await store.dispatchMediaInfoFetch('https://example.com/clip');
    useMediaDestinationStore().addDestination(session, { path: 'C:\\Downloads\\clip.mp4', confidence: 100 });
    const loading = store.loadLibrary();
    await flushPromises();
    expect(useMediaGroupStore().groupOrder).toEqual([session, 'library:other']);
    vi.useFakeTimers();
    store.deleteGroup('library:other');
    await vi.runOnlyPendingTimersAsync();
    await flushPromises();
    expect(removed).toEqual([{ paths: ['C:/Downloads/other.mp4'] }]);
    finishScan([first, second]);
    await loading;
    expect(useMediaGroupStore().groupOrder).toEqual([session]);
  });

  it('replaces a restored duplicate when a session final path arrives later', async () => {
    const row = { id: 'saved', paths: ['\\\\?\\C:\\Downloads\\clip.mp4'], url: '', title: 'clip', kind: 'video', size: 1, completedAt: 1 };
    installTauriMock({ library_list: () => [row] });
    const store = useMediaStore();
    await store.loadLibrary();
    const session = await store.dispatchMediaInfoFetch('https://example.com/clip');
    store.processMediaDestinationPayload({ id: 'session-item', groupId: session, destination: { path: 'C:/Downloads/clip.mp4', confidence: 100 } });
    await flushPromises();
    expect(useMediaGroupStore().groupOrder).toEqual([session]);
    expect(useMediaStateStore().getState('library:saved')).toBeUndefined();
  });

  it('ignores a destination event for a removed download', () => {
    installTauriMock({});
    const store = useMediaStore();
    store.processMediaDestinationPayload({ id: 'gone-item', groupId: 'gone', destination: { path: '/downloads/gone.mp4', confidence: 100 } });
    expect(useMediaDestinationStore().getDestinations('gone')).toEqual([]);
  });

  it('keeps durable history when disposing stores for hot reload', async () => {
    const removed: unknown[] = [];
    installTauriMock({
      library_list: () => [{ id: 'saved', paths: ['/downloads/clip.mp4'], url: '', title: 'clip', kind: 'video', size: 1, completedAt: 1 }],
      library_remove: (_cmd, args) => { removed.push(args); },
    });
    const store = useMediaStore();
    await store.loadLibrary();
    store.deleteAllGroups(false);
    await flushPromises();
    expect(useMediaGroupStore().groupOrder).toEqual([]);
    expect(removed).toEqual([]);
  });

  it('keeps distinct POSIX files whose names contain literal backslashes', async () => {
    installTauriMock({ library_list: () => [
      { id: 'literal', paths: ['/downloads/a\\b.mp4'], url: '', title: 'a', kind: 'video', size: 1, completedAt: 2 },
      { id: 'nested', paths: ['/downloads/a/b.mp4'], url: '', title: 'b', kind: 'video', size: 1, completedAt: 1 },
    ] });
    await useMediaStore().loadLibrary();
    expect(useMediaGroupStore().groupOrder).toEqual(['library:literal', 'library:nested']);
  });

  it('batches finished-row removals into one library rewrite request', async () => {
    vi.useFakeTimers();
    try {
      const removed: unknown[] = [];
      installTauriMock({
        library_list: () => ['a', 'b', 'c'].map(id => ({ id, paths: [`/downloads/${id}.mp4`], url: '', title: id, kind: 'video', size: 1, completedAt: 1 })),
        library_remove: (_cmd, args) => { removed.push(args); },
      });
      const store = useMediaStore();
      await store.loadLibrary();
      store.deleteGroupsByState([MediaState.done]);
      await vi.runAllTimersAsync();
      expect(removed).toEqual([{ paths: ['/downloads/a.mp4', '/downloads/b.mp4', '/downloads/c.mp4'] }]);
      expect(useMediaGroupStore().groupOrder).toEqual([]);
    } finally {
      vi.useRealTimers();
    }
  });

  it('keeps partial files on pause and discards them when removing a File row', async () => {
    const calls: Record<string, unknown>[] = [];
    installTauriMock({
      group_cancel: (_cmd, args) => {
        calls.push(args as Record<string, unknown>);
      },
    });
    const store = useMediaStore();
    const id = await store.dispatchMediaInfoFetch('https://example.com/report.pdf', false, false, {
      id: 'file-row', url: 'https://example.com/report.pdf', mediaKind: 'file', audioCodecs: [], formats: [], filesize: 128,
    });
    useMediaStateStore().setGroupState(id, MediaState.downloading);
    await store.pauseGroup(id);
    store.deleteGroup(id);
    expect(calls).toEqual([{ groupId: id, discardFiles: false }, { groupId: id, discardFiles: true }]);
  });

  it('drops a waiting login card even when its row is already gone', () => {
    const login = useLoginStore();
    login.using['gone-row'] = { groupId: 'gone-row' } as LoginUsing;
    login.failed['gone-row'] = { groupId: 'gone-row', host: 'x.com', reason: 'exhausted' };
    useMediaStore().deleteGroup('gone-row');
    expect(login.using['gone-row']).toBeUndefined();
    expect(login.failed['gone-row']).toBeUndefined();
  });

  it.each(['image', 'file'] as const)('keeps %s routing when prepared metadata has no attachment manifest', async (mediaKind) => {
    const calls: Record<string, unknown>[] = [];
    installTauriMock({
      media_download: (_cmd, args) => {
        calls.push(args as Record<string, unknown>);
        return 'images';
      },
    });
    const store = useMediaStore();
    const id = await store.dispatchMediaInfoFetch('https://www.linkedin.com/posts/example-share-456/', false, false, {
      id: 'photo-post', url: 'https://www.linkedin.com/posts/example-share-456/', mediaKind, audioCodecs: [], formats: [], filesize: 0,
    });
    await store.downloadGroup(id, { trackType: TrackType.both });
    expect(calls).toHaveLength(1);
    expect((calls[0].items as Record<string, unknown>[])[0]).toMatchObject({ mediaKind });
  });

  it('carries the gallery winner for a TikTok video-path photo row without a manifest', async () => {
    const calls: Record<string, unknown>[] = [];
    installTauriMock({
      media_download: (_cmd, args) => {
        calls.push(args as Record<string, unknown>);
      },
    });
    const store = useMediaStore();
    const item = {
      id: 'tiktok-photo', url: 'https://www.tiktok.com/@x/video/7703997545197915412',
      mediaKind: 'carousel' as const, fetchEngine: 'gallery' as const,
      audioCodecs: [], formats: [], filesize: 0,
    };
    const id = await store.dispatchMediaInfoFetch(item.url, false, false, item);
    await store.downloadGroup(id, { trackType: TrackType.both });
    expect((calls[0].items as Record<string, unknown>[])[0]).toMatchObject({
      fetchEngine: 'gallery', mediaKind: 'carousel',
    });
  });

  it('keeps a reader-resolved video target when downloading a verified fetch', async () => {
    const calls: Record<string, unknown>[] = [];
    installTauriMock({
      media_download: (_cmd, args) => {
        calls.push(args as Record<string, unknown>);
      },
    });
    const store = useMediaStore();
    const item = {
      id: 'reddit-video', url: 'https://www.reddit.com/r/nasa/comments/1wtjy4r/title/',
      resolvedUrl: 'https://v.redd.it/clip/DASHPlaylist.mpd?token=signed',
      title: 'NASA VIEW program', uploader: 'Author',
      fetchEngine: 'ytdlp' as const, mediaKind: 'video' as const,
      audioCodecs: [], formats: [], filesize: 0,
    };
    const id = await store.dispatchMediaInfoFetch(item.url, false, false, item);
    await store.downloadGroup(id, { trackType: TrackType.both });
    expect((calls[0].items as Record<string, unknown>[])[0]).toMatchObject({
      url: item.url, resolvedUrl: item.resolvedUrl, fetchEngine: 'ytdlp',
      templateContext: { values: { title: 'NASA VIEW program', fulltitle: 'NASA VIEW program', uploader: 'Author' } },
    });
  });

  it('does not download an empty image selection and sends only the chosen attachment IDs', async () => {
    const calls: Record<string, unknown>[] = [];
    installTauriMock({
      media_download: (_cmd, args) => {
        calls.push(args as Record<string, unknown>);
        return 'images';
      },
    });
    const store = useMediaStore();
    const id = await store.dispatchMediaInfoFetch('https://example.com/post', false, false, {
      id: 'photo-post', url: 'https://example.com/post', mediaKind: 'carousel', audioCodecs: [], formats: [], filesize: 0,
      title: 'Photo post', uploader: 'Author',
      attachments: [
        { id: 'a', kind: 'image', url: 'https://example.com/a.jpg' },
        { id: 'b', kind: 'image', url: 'https://example.com/b.jpg', fileIndex: 2 },
      ],
    });
    const options = useMediaOptionsStore();
    options.setAttachmentSelection(id, []);
    await store.downloadGroup(id, { trackType: TrackType.both });
    expect(calls).toHaveLength(0);
    options.setAttachmentSelection(id, ['b']);
    await store.downloadGroup(id, { trackType: TrackType.both });
    expect(calls).toHaveLength(1);
    expect((calls[0].items as Record<string, unknown>[])[0]).toMatchObject({
      mediaKind: 'carousel', attachmentIds: ['b'],
      templateContext: { values: { title: 'Photo post', uploader: 'Author' } },
      attachments: [
        { id: 'a', kind: 'image', url: 'https://example.com/a.jpg' },
        { id: 'b', kind: 'image', url: 'https://example.com/b.jpg', fileIndex: 2 },
      ],
    });
  });

  beforeEach(() => {
    notify.mockReset();
    notifyGroup.mockReset();
    clearMocks();
  });

  afterEach(() => {
    vi.useRealTimers();
    clearMocks();
  });

  it('applies input filter overrides when dispatching media info fetches', async () => {
    const settingsStore = useSettingsStore();
    Object.assign(settingsStore.settings, structuredClone(defaultSettings));
    settingsStore.settings.inputFilters = {
      ...settingsStore.settings.inputFilters,
      minSize: { value: 50, unit: 'MB' },
      dateFilter: { mode: 'after', value: '2026-06-01' },
    };

    const calls: Array<Record<string, unknown>> = [];
    installTauriMock({
      media_info: (_cmd, args) => {
        const payload = args as Record<string, unknown>;
        calls.push(payload);
        return payload.groupId as string;
      },
    });

    const mediaStore = useMediaStore();
    const groupId = await mediaStore.dispatchMediaInfoFetch('https://example.com/video');

    expect(groupId).toBeTypeOf('string');
    expect(calls).toHaveLength(1);
    expect(calls[0].overrides).toEqual({
      inputFilters: {
        minFilesize: '50M',
        dateafter: '20260601',
      },
    });
  });

  it.each([
    { total: 2, threshold: 3, groups: 2 },
    { total: 3, threshold: 3, groups: 1 },
    { total: 4, threshold: 3, groups: 1 },
  ])('finalizes $total items with split threshold $threshold into $groups groups', ({ total, threshold, groups }) => {
    useSettingsStore().settings.performance.splitPlaylistThreshold = threshold;
    const groupStore = useMediaGroupStore();
    const group = createPlaylistGroup('group-1', total);
    for (const entry of group.entries!) {
      const id = `item-${entry.index}`;
      group.items[id] = {
        id, groupId: group.id, url: entry.videoUrl, title: `Item ${entry.index + 1}`,
        audioCodecs: [], formats: [], filesize: 0,
      };
    }
    groupStore.createGroup(group);

    useMediaStore().finalizePlaylistGroup(group);

    const result = groupStore.orderedGroups;
    expect(result).toHaveLength(groups);
    expect(result.flatMap(g => Object.values(g.items).filter(item => !item.entries).map(item => item.url))).toEqual(
      group.entries!.map(entry => entry.videoUrl),
    );
    expect(result.map(g => g.total)).toEqual(groups === 1 ? [total] : Array(total).fill(1));
    for (const g of result) expect(useMediaStateStore().getGroupState(g.id)).toBe(MediaState.configure);
  });

  it('expands playlists with the selected subset and updates overrides', async () => {
    let expandArgs: Record<string, unknown> | undefined;
    installTauriMock({
      media_playlist_expand: (_cmd, args) => {
        expandArgs = args as Record<string, unknown>;
        return 'group-1';
      },
    });

    const groupStore = useMediaGroupStore();
    const optionsStore = useMediaOptionsStore();
    const stateStore = useMediaStateStore();
    const mediaStore = useMediaStore();

    const group = createPlaylistGroup('group-1', 3);
    groupStore.createGroup(group);
    optionsStore.setOverrides('group-1', {
      inputFilters: {
        matchFilters: '!is_live',
      },
    });

    await mediaStore.expandPlaylistGroup('group-1', {
      rows: [{ id: 'row-1', type: 'range', start: 1, end: 2 }],
    });

    expect(group.entries?.map(entry => entry.videoUrl)).toEqual([
      'https://example.com/playlist?v=1',
      'https://example.com/playlist?v=2',
    ]);
    expect(stateStore.getGroupState('group-1')).toBe(MediaState.fetchingList);
    expect(optionsStore.getOverrides('group-1')).toEqual({
      inputFilters: {
        matchFilters: '!is_live',
        playlistItems: '1:2',
      },
    });
    expect(expandArgs).toMatchObject({
      groupId: 'group-1',
      entries: group.entries,
      overrides: {
        inputFilters: {
          matchFilters: '!is_live',
          playlistItems: '1:2',
        },
      },
    });
  });

  it('removes playlist item overrides when selecting the full playlist', async () => {
    installTauriMock({
      media_playlist_expand: () => 'group-1',
    });

    const groupStore = useMediaGroupStore();
    const optionsStore = useMediaOptionsStore();
    const mediaStore = useMediaStore();

    const group = createPlaylistGroup('group-1', 2);
    groupStore.createGroup(group);
    optionsStore.setOverrides('group-1', {
      inputFilters: {
        playlistItems: '1',
      },
    });

    await mediaStore.expandPlaylistGroup('group-1', { rows: [] });

    expect(optionsStore.getOverrides('group-1')).toBeUndefined();
    expect(group.entries).toHaveLength(2);
  });

  it('falls back to playlist selection when auto-expanding a skipped playlist fails', async () => {
    installTauriMock({
      media_playlist_expand: () => {
        throw new Error('expand failed');
      },
    });

    const groupStore = useMediaGroupStore();
    const stateStore = useMediaStateStore();
    const mediaStore = useMediaStore();

    groupStore.createGroup({
      id: 'group-1',
      url: 'https://example.com/playlist',
      total: 1,
      processed: 0,
      errored: 0,
      isCombined: false,
      audioCodecs: [],
      videoCodecs: [],
      audioTracks: [],
      videoTracks: [],
      formats: [],
      filesize: 0,
      skipPlaylistSelection: true,
      items: {
        placeholder: {
          id: 'placeholder',
          isLeader: true,
          url: 'https://example.com/playlist',
          audioCodecs: [],
          formats: [],
          filesize: 0,
        },
      },
    });

    const payload: MediaAddPayload = {
      groupId: 'group-1',
      total: 2,
      item: {
        id: 'leader',
        url: 'https://example.com/playlist',
        title: 'Playlist',
        audioCodecs: [],
        formats: [],
        filesize: 0,
        entries: createEntries(2),
      },
    };

    const consoleError = vi.spyOn(console, 'error').mockImplementation(() => {});

    mediaStore.processMediaAddPayload(payload);
    await flushPromises();

    expect(stateStore.getState('leader')).toBe(MediaState.playlistSelection);

    consoleError.mockRestore();
  });
  it('downloads all photos from cached capture metadata without refetching or stopping capture', async () => {
    const writes: unknown[] = [];
    installTauriMock({
      media_info: () => { throw new Error('must reuse prepared metadata'); },
      media_download: (_cmd, payload) => {
        writes.push(payload);
        return 'ok';
      },
    });
    const media = useMediaStore();
    const item = {
      id: 'capture-1', url: 'https://x.com/user/status/123', title: 'Captured photos',
      audioCodecs: [], videoCodecs: [], formats: [], filesize: 0,
      mediaKind: 'carousel' as const,
      attachments: [
        { id: 'photo-1', kind: 'image' as const, url: 'https://cdn.example/first.jpg' },
        { id: 'photo-2', kind: 'image' as const, url: 'https://cdn.example/second.jpg' },
      ],
    };
    const id = await media.dispatchMediaInfoFetch(item.url, true, true, item);
    expect(useMediaGroupStore().findGroupById(id)).toMatchObject({ mediaKind: 'carousel', attachments: item.attachments });
    expect(useMediaStateStore().getState('capture-1')).toBe(MediaState.configure);
    await media.downloadPreparedGroup(id);
    expect(writes).toHaveLength(1);
    expect(writes[0]).toMatchObject({
      groupId: id,
      items: [{ id: 'capture-1', url: item.url, mediaKind: 'carousel', attachmentIds: ['photo-1', 'photo-2'] }],
    });
    expect(useMediaStateStore().getState('capture-1')).toBe(MediaState.downloading);
    expect(useWatchClipboardStore().enabled).toBe(true);
  });

  it.each([
    ['small', 720, 'both'],
    ['audio', undefined, 'audio'],
  ] as const)('starts a captured video with the quick pick %s once it is read', async (kind, height, trackType) => {
    const writes: unknown[] = [];
    installTauriMock({
      media_info: () => { throw new Error('must reuse prepared metadata'); },
      media_download: (_cmd, payload) => {
        writes.push(payload);
        return 'ok';
      },
    });
    const media = useMediaStore();
    const fmt = (id: string, h: number) => ({ id, height: h, fps: 30, audioCodecs: [], videoCodecs: [] });
    const item = {
      id: 'capture-2', url: 'https://www.youtube.com/watch?v=abc', title: 'Captured video',
      audioCodecs: [], videoCodecs: [], filesize: 0, mediaKind: 'video' as const,
      formats: [fmt('v1080', 1080), fmt('v720', 720), fmt('v480', 480), { id: 'a', abr: 128, audioCodecs: [], videoCodecs: [] }],
    };
    const id = await media.dispatchMediaInfoFetch(item.url, true, true, item);
    await media.downloadPreparedGroup(id, { kind });
    expect(writes).toHaveLength(1);
    const sent = JSON.stringify(writes[0]);
    expect(sent).toContain(`"trackType":"${trackType}"`);
    if (height) expect(sent).toContain(`"height":${height}`);
    else expect(sent).not.toContain('"height":1080');
  });

  it.each([false, true])('applies the audio default to quick Best, preserving stored options (%s)', async (stored) => {
    const writes: unknown[] = [];
    installTauriMock({
      media_download: (_cmd, payload) => {
        writes.push(payload);
        return 'ok';
      },
    });
    const { defaultChoice } = useUiPrefs();
    const previous = defaultChoice.value;
    defaultChoice.value = 'audio';
    try {
      const media = useMediaStore();
      const item = {
        id: 'capture-default', url: 'https://example.com/video', title: 'Video',
        audioCodecs: [], videoCodecs: [], filesize: 0, mediaKind: 'video' as const,
        formats: [{ id: 'v1080', height: 1080, audioCodecs: [], videoCodecs: [] }],
      };
      const id = await media.dispatchMediaInfoFetch(item.url, true, true, item);
      if (stored) useMediaOptionsStore().setOptions(id, { trackType: TrackType.both, height: 720 });
      await media.downloadPreparedGroup(id);
      expect(writes).toEqual([expect.objectContaining({
        items: [expect.objectContaining({
          format: expect.objectContaining(stored ? { trackType: TrackType.both, height: 720 } : { trackType: TrackType.audio }),
        })],
      })]);
    } finally {
      defaultChoice.value = previous;
    }
  });
});
