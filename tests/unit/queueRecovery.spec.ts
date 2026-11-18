import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { nextTick } from 'vue';
import { createPinia, disposePinia, getActivePinia, setActivePinia } from 'pinia';
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import { encodeQueue, decodeQueue, writeQueue, readQueue, type SavedDownload } from '../../src/helpers/queueRecovery';
import { useMediaStore } from '../../src/stores/media/media';
import { useMediaStateStore, MediaState } from '../../src/stores/media/state';
import { useMediaGroupStore } from '../../src/stores/media/group';
import { useMediaOptionsStore } from '../../src/stores/media/options';
import { TrackType } from '../../src/tauri/types/media';

function saved(): SavedDownload {
  return {
    group: { id: 'saved', url: 'https://example.com/video', title: 'فيديو', total: 1, processed: 1, errored: 0,
      isCombined: false, audioCodecs: [], formats: [], filesize: 100,
      items: { item: { id: 'item', groupId: 'saved', url: 'https://example.com/video', isLeader: true, playlistChild: true, title: 'فيديو', formats: [], audioCodecs: [], filesize: 100 } } },
    options: { trackType: TrackType.both, height: 720 },
    overrides: { output: { downloadDir: '/downloads' }, auth: { password: 'secret', cookieFile: '/private/cookies' }, network: { proxy: 'https://user:pass@proxy' } },
  };
}

describe('saved download queue', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    localStorage.clear();
    setActivePinia(createPinia());
  });
  afterEach(() => {
    disposePinia(getActivePinia()!);
    vi.useRealTimers();
    clearMocks();
    vi.restoreAllMocks();
    localStorage.clear();
  });
  it('round-trips choices, IDs and destination without credentials', () => {
    const text = encodeQueue([saved()]);
    const [row] = decodeQueue(text);
    expect(row.group).toEqual(saved().group);
    expect(row.options).toEqual(saved().options);
    expect(row.overrides).toEqual({ output: { downloadDir: '/downloads' } });
    expect(text).not.toContain('secret');
    expect(text).not.toContain('proxy');
  });
  it('drops nested credentials, signed media and bulky probe metadata', () => {
    const row = saved();
    row.group.items.item.overrides = { auth: { password: 'nested-secret', bearerToken: 'bearer-secret', headers: ['Authorization: header-secret'] } };
    row.group.items.item.resolvedUrl = 'https://cdn.example/video?signature=secret';
    row.group.items.item.attachments = [{ id: 'a', kind: 'image', url: 'https://cdn.example/photo?signature=secret' }];
    row.group.attachments = row.group.items.item.attachments;
    row.group.items.item.description = 'bulky description';
    const text = encodeQueue([row]);
    for (const secret of ['nested-secret', 'bearer-secret', 'header-secret', 'signature=secret', 'bulky description']) expect(text).not.toContain(secret);
    expect(decodeQueue(text)[0].group.items.item.url).toBe(row.group.items.item.url);
  });
  it('coalesces a burst of queue changes into one delayed persistence write', async () => {
    vi.useFakeTimers();
    mockIPC(() => undefined);
    useMediaStore();
    const writes = vi.spyOn(Storage.prototype, 'setItem');
    const row = saved();
    useMediaGroupStore().createGroup(row.group);
    useMediaOptionsStore().setOptions('saved', row.options);
    useMediaStateStore().setState('item', MediaState.downloading);
    for (let n = 0; n < 1000; n++) useMediaGroupStore().groups.saved.title = String(n);
    await nextTick();
    expect(writes).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(500);
    expect(writes).toHaveBeenCalledTimes(1);
    vi.useRealTimers();
  });
  it.each(['bad json', '{"version":2,"rows":[]}', '{"version":1,"rows":[null,{}]}'])('ignores corrupt/unknown data: %s', (raw) => {
    expect(decodeQueue(raw)).toEqual([]);
  });
  it('does not start on launch; click resumes with the saved IDs and selection', async () => {
    writeQueue([saved()]);
    const calls: { cmd: string; payload: unknown }[] = [];
    mockIPC((cmd, payload) => {
      calls.push({ cmd, payload });
      return undefined;
    });
    const media = useMediaStore();
    expect(media.restoreCount).toBe(1);
    expect(calls.find(call => call.cmd === 'group_sweep_partials')?.payload).toEqual({ saved: [['saved', 'item']], roots: ['/downloads'] });
    expect(calls.filter(call => call.cmd === 'media_download')).toHaveLength(0);
    await media.resumeSavedDownloads();
    expect(media.restoreCount).toBe(0);
    expect(calls.find(call => call.cmd === 'media_download')?.payload).toMatchObject({
      groupId: 'saved', origin: 'resume', items: [{ id: 'item', format: { height: 720 }, overrides: { output: { downloadDir: '/downloads' } } }],
    });
    expect(readQueue()).toHaveLength(1);
    useMediaStateStore().setState('item', MediaState.done);
    await nextTick();
    await vi.advanceTimersByTimeAsync(500);
    expect(readQueue()).toEqual([]);
  });
  it('re-resolves signed attachments while preserving the selected IDs', async () => {
    const row = saved();
    row.group.mediaKind = 'image';
    row.group.items.item.mediaKind = 'image';
    row.group.items.item.fetchEngine = 'gallery';
    row.group.attachments = [{ id: 'chosen', kind: 'image', url: 'https://cdn.example/signed' }];
    row.group.items.item.attachments = row.group.attachments;
    row.attachmentIds = ['chosen'];
    writeQueue([row]);
    const calls: { cmd: string; payload: unknown }[] = [];
    mockIPC((cmd, payload) => {
      calls.push({ cmd, payload });
    });
    await useMediaStore().resumeSavedDownloads();
    expect(calls.find(call => call.cmd === 'media_download')?.payload).toMatchObject({
      items: [{ url: row.group.url, attachmentIds: ['chosen'], fetchEngine: 'gallery' }],
    });
    expect(JSON.stringify(calls)).not.toContain('https://cdn.example/signed');
  });
  it('dismiss clears persisted queue and requests partial cleanup without a download', async () => {
    writeQueue([saved()]);
    const calls: string[] = [];
    mockIPC((cmd) => {
      calls.push(cmd);
    });
    const media = useMediaStore();
    await media.dismissSavedDownloads();
    expect(readQueue()).toEqual([]);
    expect(calls).toContain('group_cancel');
    expect(calls).not.toContain('media_download');
  });
  it('persists pending rows, retains an unanswered restore offer, and removes cancelled rows', async () => {
    writeQueue([saved()]);
    useMediaStore();
    const row = saved();
    row.group.id = 'new';
    row.group.items.item.id = 'new-item';
    useMediaGroupStore().createGroup(row.group);
    useMediaOptionsStore().setOptions('new', row.options);
    useMediaStateStore().setState('new-item', MediaState.downloading);
    await nextTick();
    await vi.advanceTimersByTimeAsync(500);
    expect(readQueue().map(row => row.group.id)).toEqual(['saved', 'new']);
    useMediaStateStore().setState('new-item', MediaState.cancelled);
    await nextTick();
    await vi.advanceTimersByTimeAsync(500);
    expect(readQueue().map(row => row.group.id)).toEqual(['saved']);
  });
});
