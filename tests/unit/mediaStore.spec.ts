import { createPinia, setActivePinia } from 'pinia';
import type { LibraryRecord } from '../../src/tauri/types/library';
import { flushPromises } from '@vue/test-utils';
import { reactive } from 'vue';
import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';
import { clearMocks } from '@tauri-apps/api/mocks';
import { installTauriMock } from '../utils/tauriMock.ts';
import { useMediaStore } from '../../src/stores/media/media';
import { useMediaGroupStore } from '../../src/stores/media/group';
import { useMediaDiagnosticsStore } from '../../src/stores/media/diagnostics';
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
  it('normalizes links at the queue boundary for shortcuts and direct callers', async () => {
    const info = vi.fn().mockResolvedValue('ok');
    installTauriMock({ media_info: info });
    await useMediaStore().dispatchMediaInfoFetch('\u200fyoutube.com/watch?v=YsKxkP_kGbE&t=10\u200f', true);
    expect(info.mock.calls[0][1]).toMatchObject({ url: 'https://youtube.com/watch?v=YsKxkP_kGbE&t=10' });
  });

  it('fills a missing photo title on the final destination event without overwriting metadata', () => {
    const store = useMediaStore();
    const groups = useMediaGroupStore();
    const groupId = store.createCaptureGroup('capture-photo', 'https://www.facebook.com/photo/?fbid=123');
    const group = groups.findGroupById(groupId)!;
    const item = Object.values(group.items)[0];
    store.processMediaAddPayload({ groupId, total: 1, item: { ...item, mediaKind: 'image', title: undefined } });
    store.processMediaDestinationPayload({ groupId, id: item.id, destination: { path: '/downloads/123.jpg', confidence: 100 } });
    expect(group.title).toBe('123.jpg');
    expect(group.items[item.id].title).toBe('123.jpg');
    store.processMediaAddPayload({ groupId, total: 1, item: { ...item, mediaKind: 'image', title: undefined } });
    expect(group.title).toBe('123.jpg');
    store.processMediaAddPayload({ groupId, total: 1, item: { ...item, mediaKind: 'image', title: 'Photo caption' } });
    store.processMediaDestinationPayload({ groupId, id: item.id, destination: { path: '/downloads/123.jpg', confidence: 100 } });
    expect(group.title).toBe('Photo caption');
  });

  it('cancels immediately through removal and Undo requeues the saved group with every option', async () => {
    const requests: { cmd: string; args: unknown }[] = [];
    installTauriMock({
      group_cancel: (cmd, args) => { requests.push({ cmd, args: structuredClone(args) }); },
      media_download: (cmd, args) => { requests.push({ cmd, args: structuredClone(args) }); },
    });
    const groups = useMediaGroupStore();
    const store = useMediaStore();
    const options = useMediaOptionsStore();
    const group = createPlaylistGroup('cancel-me', 1);
    delete group.items.leader.entries;
    // Media events and option selections can insert already-reactive children into the group.
    group.items.leader.attachments = reactive([{ id: 'photo', kind: 'image' as const, url: 'https://example.com/photo.jpg' }]);
    group.attachments = group.items.leader.attachments;
    groups.createGroup(group);
    useMediaStateStore().setGroupState(group.id, MediaState.downloading);
    const chosen = { trackType: TrackType.both, height: 720 };
    options.setEncodings(group.id, { audio: 'aac', video: 'h264' });
    options.setTracks(group.id, { audio: 'en', video: 'main' });
    options.setOverrides(group.id, { output: { downloadDir: '/chosen' } });
    options.setAttachmentSelection(group.id, ['photo']);
    expect(await store.downloadGroup(group.id, chosen)).toBe(true);
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

  it.each(['video', 'audio', 'photos', 'mixed', 'failed'] as const)('persists then reloads %s display facts', async (kind) => {
    let saved: LibraryRecord | undefined;
    installTauriMock({
      media_download: (_cmd, args) => {
        const request = args as { metadata: Record<string, { historyId: string; display: LibraryRecord['display'] }> };
        saved = { id: Object.values(request.metadata)[0].historyId, paths: ['/downloads/saved.mp4'], url: 'https://example.com/post', title: 'Caption', kind: kind === 'audio' ? 'audio' : 'video', size: 5, completedAt: 1,
          ...Object.values(request.metadata)[0] };
      },
      library_record_failure: (_cmd, args) => { saved = { ...(args as { row: LibraryRecord }).row, paths: [], completedAt: 1, size: 0 }; },
      library_list: () => saved ? [JSON.parse(JSON.stringify(saved))] : [],
    });
    const store = useMediaStore();
    const groupId = await store.dispatchMediaInfoFetch('https://example.com/post');
    const group = useMediaGroupStore().findGroupById(groupId)!;
    const first = Object.values(group.items)[0];
    const attachments = kind === 'photos' || kind === 'mixed'
      ? [1, 2, 3].map(n => ({ id: `${n}`, kind: kind === 'mixed' && n === 3 ? 'video' as const : 'image' as const, url: `https://cdn.example/${n}` }))
      : undefined;
    const sourceKind = kind === 'photos' ? 'carousel' : kind === 'mixed' ? 'mixed' : 'video';
    store.processMediaAddPayload({ groupId: group.id, total: 1, item: {
      ...first, title: 'Caption', duration: 42, mediaKind: sourceKind, attachments,
      formats: kind === 'video' ? [{ id: 'v', height: 1080, fps: 60, videoCodecs: ['h264'], audioCodecs: ['aac'] }] : [],
    } });
    const options = { trackType: kind === 'audio' ? TrackType.audio : TrackType.both, ...(kind === 'video' ? { height: 1080, fps: 60 } : {}) };
    if (kind === 'failed') {
      useMediaDiagnosticsStore().processMediaDiagnosticPayload({ id: first.id, groupId: group.id, code: 'networkTimeout', level: 'error', component: null, message: '', raw: '', timestamp: 1 });
      useMediaDiagnosticsStore().processMediaFatalPayload({ id: first.id, groupId: group.id, message: 'failure', details: null, exitCode: 1, internal: false, timestamp: 1 });
    } else {
      await store.downloadGroup(group.id, options);
    }
    await flushPromises();
    expect(saved).toBeDefined();
    if (kind === 'failed') expect(saved!.id).not.toBe(first.id);
    // A new app/store instance consumes only the serialized library contract.
    setActivePinia(createPinia());
    await useMediaStore().loadLibrary();
    const id = `library:${saved!.id}`;
    const loaded = useMediaGroupStore().findGroupById(id)!;
    expect(loaded.duration).toBe(42);
    expect(useMediaStateStore().getGroupState(id)).toBe(kind === 'failed' ? MediaState.error : MediaState.done);
    if (kind !== 'failed') expect(useMediaOptionsStore().getOptions(id)).toMatchObject(options);
    if (attachments) expect(loaded.savedAttachmentSummary).toEqual(kind === 'mixed' ? { photos: 2, videos: 1 } : { photos: 3, videos: 0 });
    if (kind === 'failed') {
      expect(useMediaDiagnosticsStore().findLastDiagnosticByGroupId(id)?.code).toBe('networkTimeout');
      expect(useMediaDestinationStore().getDestinations(id)).toEqual([]);
    }
  });

  it('removes failed history before retry and does not resurrect it from an in-flight scan', async () => {
    const row = { id: 'failure', paths: [], url: 'https://example.com/post', title: 'Failed', kind: 'video', size: 0, completedAt: 1, errorCode: 'networkTimeout' };
    let finishScan!: (rows: unknown[]) => void;
    const removes: unknown[] = [];
    installTauriMock({
      library_list: (_cmd, args) => (args as { scan: boolean }).scan ? new Promise((resolve) => { finishScan = resolve; }) : [row],
      library_remove: (_cmd, args) => { removes.push(args); },
    });
    const store = useMediaStore();
    const loading = store.loadLibrary();
    await flushPromises();
    await store.retryGroup('library:failure');
    await flushPromises();
    expect(removes).toContainEqual({ paths: [], ids: ['failure'] });
    finishScan([row]);
    await loading;
    expect(useMediaGroupStore().groupOrder).toEqual(['library:failure']);
    expect(useMediaStateStore().getGroupState('library:failure')).toBe(MediaState.fetching);
  });

  it('recovers legacy audio labels and photo counts from saved files', async () => {
    installTauriMock({ library_list: () => [
      { id: 'audio', paths: ['/downloads/a.mp3'], url: '', title: 'Audio', kind: 'audio', size: 1, completedAt: 1 },
      { id: 'photos', paths: ['/downloads/a.jpg', '/downloads/b.jpg'], url: '', title: 'Photos', kind: 'image', size: 2, completedAt: 1 },
    ] });
    await useMediaStore().loadLibrary();
    expect(useMediaOptionsStore().getOptions('library:audio')?.trackType).toBe(TrackType.audio);
    expect(useMediaGroupStore().findGroupById('library:photos')?.savedAttachmentSummary).toEqual({ photos: 2, videos: 0 });
  });

  it('does not duplicate a live failed row when startup history finishes loading', async () => {
    let failure: object | undefined;
    installTauriMock({
      library_record_failure: (_cmd, args) => { failure = (args as { row: object }).row; },
      library_list: () => failure ? [{ ...failure, paths: [], completedAt: 1, size: 0 }] : [],
    });
    const store = useMediaStore();
    const id = await store.dispatchMediaInfoFetch('https://example.com/post');
    useMediaStateStore().setGroupState(id, MediaState.error);
    store.rememberFailedGroup(id);
    await flushPromises();
    await store.loadLibrary();
    expect(useMediaGroupStore().groupOrder).toEqual([id]);
  });

  it('uses distinct durable identities when capture counters restart', async () => {
    const ids: string[] = [];
    installTauriMock({ library_record_failure: (_cmd, args) => {
      ids.push((args as { row: { id: string } }).row.id);
    } });
    for (const url of ['https://example.com/one', 'https://example.com/two']) {
      setActivePinia(createPinia());
      const store = useMediaStore();
      store.createCaptureGroup('capture-1', url);
      store.processMediaAddPayload({ groupId: 'capture-1', total: 1, item: { id: 'capture-1', isLeader: true, url, formats: [], audioCodecs: [], filesize: 0 } });
      useMediaStateStore().setGroupState('capture-1', MediaState.error);
      store.rememberFailedGroup('capture-1');
      await flushPromises();
    }
    expect(ids).toHaveLength(2);
    expect(ids[0]).not.toBe(ids[1]);
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

    await mediaStore.expandPlaylistGroup('group-1', { rows: [], positions: [0, 1] });

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
  it('downloads exactly the checked entries with their selected quality after expansion', async () => {
    const expand = vi.fn();
    const download = vi.fn();
    installTauriMock({ media_playlist_expand: expand, media_download: download });
    const media = useMediaStore();
    const group = createPlaylistGroup('checklist', 3);
    group.entries![2].index = 8;
    useMediaGroupStore().createGroup(group);
    useMediaStateStore().setGroupState(group.id, MediaState.playlistSelection);
    const options = { trackType: TrackType.both, height: 720, id: 'best', audioCodecs: [], videoCodecs: [] };
    useMediaOptionsStore().setOptions(group.id, options);
    const entries = [...group.entries!];
    const queued = media.downloadPlaylistSelection(group.id, { rows: [], positions: [0, 2] });
    expect(expand).toHaveBeenCalledExactlyOnceWith('media_playlist_expand', expect.objectContaining({ entries: [entries[0], entries[2]], overrides: { inputFilters: { playlistItems: '1,9' } } }));
    expect(download).not.toHaveBeenCalled();
    for (const [position, entry] of [entries[0], entries[2]].entries()) {
      media.processMediaAddPayload({ groupId: group.id, total: 2, item: {
        id: `checked-${position}`, url: entry.videoUrl, formats: [], audioCodecs: [], filesize: 0,
      } });
    }
    const children = useMediaGroupStore().orderedGroups;
    expect(children).toHaveLength(2);
    for (const child of children) expect(useMediaOptionsStore().getOptions(child.id)).toEqual(options);
    await queued;
    expect(download).toHaveBeenCalledTimes(2);
    for (const [, args] of download.mock.calls) expect(args).toMatchObject({ items: [{ format: options }] });
  });

  it('keeps the complete checklist and selection available after expansion is rejected', async () => {
    const expand = vi.fn().mockRejectedValueOnce(new Error('unavailable')).mockResolvedValue('retry');
    installTauriMock({ media_playlist_expand: expand });
    const group = createPlaylistGroup('retry', 3);
    useMediaGroupStore().createGroup(group);
    useMediaStateStore().setGroupState(group.id, MediaState.playlistSelection);
    useMediaOptionsStore().setPlaylistSelection(group, [0, 2]);
    const entries = [...group.entries!];
    await expect(useMediaStore().downloadPlaylistSelection(group.id, { rows: [], positions: [0, 2] })).rejects.toThrow('unavailable');
    expect(useMediaGroupStore().findGroupById(group.id)?.entries).toEqual(entries);
    expect(useMediaStateStore().getGroupState(group.id)).toBe(MediaState.playlistSelection);
    expect(useMediaOptionsStore().getPlaylistSelection(group)).toEqual([0, 2]);
    await useMediaStore().expandPlaylistGroup(group.id, { rows: [], positions: [0, 2] });
    expect(expand).toHaveBeenLastCalledWith('media_playlist_expand', expect.objectContaining({ entries: [entries[0], entries[2]] }));
  });

  it('settles every capture waiter when playlist metadata arrives', async () => {
    const media = useMediaStore();
    media.createCaptureGroup('waiting', 'https://example.com/list');
    const settled = vi.fn();
    const first = media.prepareCaptureGroup('waiting').then(settled);
    const second = media.prepareCaptureGroup('waiting').then(settled);
    const group = createPlaylistGroup('waiting', 2);
    media.processMediaAddPayload({ groupId: group.id, total: 2, item: group.items.leader });
    await flushPromises();
    expect(settled).toHaveBeenCalledTimes(2);
    await Promise.all([first, second]);
  });
  it.each(['read failure', 'fatal read', 'delete'] as const)('rejects and cleans all capture waiters on %s', async (failure) => {
    installTauriMock({ media_info: () => {
      throw new Error('Read failed');
    } });
    const media = useMediaStore();
    media.createCaptureGroup('waiting', 'https://example.com/list');
    const rejected = vi.fn();
    void media.prepareCaptureGroup('waiting').catch(rejected);
    void media.prepareCaptureGroup('waiting').catch(rejected);
    if (failure === 'read failure') await media.fetchMediaInfoForGroup('waiting').catch(() => {});
    else if (failure === 'fatal read') {
      const group = useMediaGroupStore().findGroupById('waiting')!;
      useMediaDiagnosticsStore().processMediaFatalPayload({ id: Object.keys(group.items)[0], groupId: 'waiting', message: 'Read failed', exitCode: 1, internal: true, details: null, timestamp: 0 });
    } else await media.deleteGroup('waiting');
    await flushPromises();
    expect(rejected).toHaveBeenCalledTimes(2);
    // A later wait must reject immediately, rather than leaking behind a completed failure/delete.
    void media.prepareCaptureGroup('waiting').catch(rejected);
    await flushPromises();
    expect(rejected).toHaveBeenCalledTimes(3);
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
