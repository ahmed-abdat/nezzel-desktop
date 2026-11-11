import { describe, it, vi, expect, afterEach } from 'vitest';
import { emit } from '@tauri-apps/api/event';
import { mockWindows } from '@tauri-apps/api/mocks';
import { effectScope } from 'vue';
import { flushPromises } from '@vue/test-utils';
import { installTauriMock } from '../utils/tauriMock';
import { capturePrefill, startCaptureDelivery, type CaptureMessage } from '../../src/tauri/listeners/capture';
import { registerMediaListeners } from '../../src/tauri/listeners/media';
import { useMediaStore } from '../../src/stores/media/media';
import { useMediaOptionsStore } from '../../src/stores/media/options';
import { useMediaGroupStore } from '../../src/stores/media/group';
import { MediaState, useMediaStateStore } from '../../src/stores/media/state';
import { useDownloadSheetStore } from '../../src/stores/downloadSheet';
import { useReportStore } from '../../src/stores/report';
import tauriListeners from '../../src/plugins/tauriListeners';
import type { CaptureChoice } from '../../src/generated/contract';
import { TrackType, type MediaItem } from '../../src/tauri/types/media';

// The login listeners attach after an async store import; they have their own spec (loginStore.spec.ts).
vi.mock('../../src/tauri/listeners/login', () => ({ registerLoginListeners: vi.fn() }));
vi.mock('../../src/tauri/notifications', () => ({ notify: vi.fn(), notifyGroup: vi.fn() }));

describe('tauriListeners', () => {
  describe('install', () => {
    it('registers listeners', () => {
      const spy = vi.spyOn(window.__TAURI_INTERNALS__, 'invoke');

      tauriListeners.install();

      expect(spy).toHaveBeenCalledWith('plugin:event|listen', {
        event: 'media_add',
        target: {
          kind: 'Any',
        },
        handler: expect.any(Number),
      }, undefined);
    });
  });
});

describe('capture delivery to the main window', () => {
  const url = 'https://www.youtube.com/watch?v=capture';
  const item = { id: 'capture-1', url, title: 'Captured title', audioCodecs: [], formats: [], filesize: 0 };
  type Body = CaptureMessage extends infer M ? M extends unknown ? Omit<M, 'seq'> : never : never;
  let activeScope: ReturnType<typeof effectScope>;
  afterEach(() => {
    activeScope?.stop();
    vi.useRealTimers();
  });

  /** Rust's outbox as the main window sees it: numbered messages, `capture_pull` returns those after `after`. */
  async function setup(queued: Body[] = []) {
    const queue: CaptureMessage[] = [];
    const afters: number[] = [];
    const queue_ = (body: Body) => queue.push({ ...body, seq: queue.length + 1 } as CaptureMessage);
    queued.forEach(queue_);
    const info = vi.fn(() => new Promise(() => {}));
    const download = vi.fn().mockResolvedValue('started');
    const ready = vi.fn();
    installTauriMock({
      media_info: info,
      media_download: download,
      capture_ready: ready,
      capture_pull: (_cmd, args) => {
        const after = (args as { after: number }).after;
        afters.push(after);
        return queue.filter(message => message.seq > after);
      },
    });
    mockWindows('main');
    activeScope = effectScope();
    registerMediaListeners();
    await activeScope.run(() => startCaptureDelivery());
    /** Queue in Rust and wake the main window, as `outbox::send_to_main` does. */
    async function send(...bodies: Body[]) {
      bodies.forEach(queue_);
      await emit('capture_wake');
      await flushPromises();
    }
    return { info, download, ready, afters, send, groups: useMediaGroupStore(), states: useMediaStateStore(), sheet: useDownloadSheetStore() };
  }

  it('pulls what waited before it started, then turns native capture on', async () => {
    const { groups, ready, afters } = await setup([{ kind: 'action', id: 1, url, action: 'adopt', item: null, choice: null }]);
    expect(groups.orderedGroups).toHaveLength(1);
    expect(afters[0]).toBe(0);
    expect(ready).toHaveBeenCalledOnce();
  });

  it('creates an options row and opens its sheet before the item arrives', async () => {
    const { info, groups, states, sheet, send } = await setup();
    const prepare = vi.spyOn(useMediaStore(), 'prepareCaptureGroup');
    await send({ kind: 'action', id: 1, url, action: 'options', item: null, choice: null });
    expect(groups.orderedGroups).toHaveLength(1);
    const group = groups.orderedGroups[0];
    expect(states.getGroupState(group.id)).toBe(MediaState.fetching);
    expect(sheet.groupId).toBe(group.id);
    expect(sheet.open).toBe(true);
    await send({ kind: 'item', id: 1, item });
    expect(groups.orderedGroups).toHaveLength(1);
    expect(groups.findGroupById(group.id).title).toBe('Captured title');
    expect(states.getGroupState(group.id)).toBe(MediaState.configure);
    expect(info).not.toHaveBeenCalled();
    expect(prepare).not.toHaveBeenCalled();
  });

  it('reads the link itself when the capture read failed', async () => {
    const { info, groups, send } = await setup();
    await send({ kind: 'action', id: 2, url, action: 'adopt', item: null, choice: null });
    const group = groups.orderedGroups[0];
    await send({ kind: 'item', id: 2, item: null });
    expect(info).toHaveBeenCalledExactlyOnceWith('media_info', expect.objectContaining({ url, groupId: group.id, id: Object.keys(group.items)[0] }));
  });

  it('adopts at the top without a sheet or download', async () => {
    const { info, download, groups, sheet, send } = await setup();
    await send({ kind: 'action', id: 3, url, action: 'adopt', item: null, choice: null });
    await send({ kind: 'item', id: 3, item });
    expect(groups.orderedGroups).toHaveLength(1);
    expect(groups.orderedGroups[0].url).toBe(url);
    expect(info).not.toHaveBeenCalled();
    expect(download).not.toHaveBeenCalled();
    expect(sheet.open).toBe(false);
  });

  it.each(['awaited', 'inline', 'fallback'])('downloads once the item is there (%s)', async (preview) => {
    const { info, download, groups, states, send } = await setup();
    await send({ kind: 'action', id: 4, url, action: 'download', item: preview === 'inline' ? item : null, choice: null });
    const group = groups.orderedGroups[0];
    if (preview !== 'inline') {
      expect(states.getGroupState(group.id)).toBe(MediaState.fetching);
      expect(download).not.toHaveBeenCalled();
      if (preview === 'fallback') info.mockImplementation(() => emit('media_add', { groupId: group.id, total: 1, item }));
      await send({ kind: 'item', id: 4, item: preview === 'fallback' ? null : item });
    }
    await flushPromises();
    expect(download).toHaveBeenCalledOnce();
    expect(states.getGroupState(group.id)).toBe(MediaState.downloading);
    expect(info).toHaveBeenCalledTimes(preview === 'fallback' ? 1 : 0);
  });

  it.each(['inline', 'late', 'adopted', 'fallback'])('opens playlist selection without expanding or downloading (%s)', async (mode) => {
    const { info, download, groups, states, sheet, send } = await setup();
    const playlist = { ...item, entries: [{ index: 1, videoUrl: 'https://example.com/one' }, { index: 2, videoUrl: 'https://example.com/two' }] };
    const expand = vi.spyOn(window.__TAURI_INTERNALS__, 'invoke');
    if (mode === 'adopted') await send({ kind: 'action', id: 4, url, action: 'adopt', item: playlist, choice: null });
    await send({ kind: 'action', id: 4, url, action: 'download', item: mode === 'inline' ? playlist : null, choice: null });
    if (mode === 'fallback') info.mockImplementation(() => emit('media_add', { groupId: 'capture-4', total: 2, item: playlist }));
    if (mode === 'late' || mode === 'fallback') await send({ kind: 'item', id: 4, item: mode === 'late' ? playlist : null });
    await flushPromises();
    expect(groups.orderedGroups).toHaveLength(1);
    expect(states.getGroupState('capture-4')).toBe(MediaState.playlistSelection);
    expect(sheet.groupId).toBe('capture-4');
    expect(sheet.open).toBe(true);
    expect(download).not.toHaveBeenCalled();
    expect(expand.mock.calls.some(([cmd]) => cmd === 'media_playlist_expand')).toBe(false);
    expect(expand.mock.calls.map(([cmd]) => cmd).filter(cmd => ['plugin:window|unminimize', 'plugin:window|show', 'plugin:window|set_focus'].includes(cmd))).toEqual(['plugin:window|unminimize', 'plugin:window|show', 'plugin:window|set_focus']);
  });

  it.each([
    ['download', null, {}],
    ['download', { kind: 'best' }, { trackType: 'both' }],
    ['download', { kind: 'small' }, { trackType: 'both', height: 720 }],
    ['download', { kind: 'audio' }, { trackType: 'audio' }],
    ['download', { kind: 'audio', kbps: 128 }, { trackType: 'audio', abr: 128 }],
    ['download', { kind: 'video', height: 1080, fps: 60 }, { trackType: 'both', height: 1080, fps: 60 }],
    ['options', { kind: 'video', height: 720 }, { trackType: 'both', height: 720 }],
    ['options', null, {}],
  ] as const)('routes %s %j to the picker with quality preserved', async (action, choice, expected) => {
    const { download, states, sheet, send } = await setup();
    const expand = vi.spyOn(window.__TAURI_INTERNALS__, 'invoke');
    await send({ kind: 'action', id: 57, url, action, choice, item: { ...item, entries: [{ index: 0, videoUrl: url }] } });
    expect(states.getGroupState('capture-57')).toBe(MediaState.playlistSelection);
    expect(sheet.groupId).toBe('capture-57');
    expect(sheet.open).toBe(true);
    expect(download).not.toHaveBeenCalled();
    expect(expand.mock.calls.some(([cmd]) => cmd === 'media_playlist_expand')).toBe(false);
    expect(useMediaOptionsStore().getOptions('capture-57')).toMatchObject(expected);
  });

  it('keeps a late playlist Options choice without creating a read waiter for single-video Options', async () => {
    const { send, download } = await setup();
    const prepare = vi.spyOn(useMediaStore(), 'prepareCaptureGroup');
    await send({ kind: 'action', id: 58, url, action: 'options', choice: { kind: 'small' }, item: null });
    expect(prepare).not.toHaveBeenCalled();
    await send({ kind: 'item', id: 58, item: { ...item, entries: [{ index: 0, videoUrl: url }] } });
    expect(prepare).toHaveBeenCalledTimes(1);
    expect(useMediaOptionsStore().getOptions('capture-58')).toMatchObject({ height: 720 });
    expect(download).not.toHaveBeenCalled();
  });

  it.each([
    {
      kind: 'video', choice: { kind: 'video', height: 720 },
      read: { mediaKind: 'video', formats: [{ id: 'v720', height: 720, audioCodecs: [], videoCodecs: ['h264'] }] },
      expected: { id: 'v720', height: 720, trackType: TrackType.both },
    },
    {
      kind: 'audio', choice: { kind: 'small' },
      read: { mediaKind: 'audio', formats: [
        { id: 'a128', abr: 128, audioCodecs: ['aac'], videoCodecs: [] },
        { id: 'a64', abr: 64, audioCodecs: ['aac'], videoCodecs: [] },
      ] },
      expected: { trackType: TrackType.audio },
    },
  ] satisfies { kind: string; choice: CaptureChoice; read: Partial<MediaItem>; expected: object }[])('applies a single $kind Options choice only after reading its formats and kind', async ({ choice, read, expected }) => {
    const { send, download } = await setup();
    const prepare = vi.spyOn(useMediaStore(), 'prepareCaptureGroup');
    const options = useMediaOptionsStore();
    await send({ kind: 'action', id: 59, url, action: 'options', choice, item: null });
    expect(options.getOptions('capture-59')).toBeUndefined();
    expect(prepare).not.toHaveBeenCalled();
    await send({ kind: 'item', id: 59, item: { ...item, ...read } });
    expect(options.getOptions('capture-59')).toMatchObject(expected);
    expect(download).not.toHaveBeenCalled();
  });

  it('uses the read formats when Options upgrades an already-adopted single video', async () => {
    const { send, download } = await setup();
    const formats = [{ id: 'v720', height: 720, audioCodecs: [], videoCodecs: ['h264'] }];
    await send({ kind: 'action', id: 60, url, action: 'adopt', choice: null, item: { ...item, mediaKind: 'video', formats } });
    await send({ kind: 'action', id: 60, url, action: 'options', choice: { kind: 'video', height: 720 }, item: null });
    expect(useMediaOptionsStore().getOptions('capture-60')).toMatchObject({ id: 'v720', height: 720, trackType: TrackType.both });
    expect(download).not.toHaveBeenCalled();
  });

  it('a click after focus adopted the row starts that row, not a second one', async () => {
    const { download, groups, states, send } = await setup();
    await send({ kind: 'action', id: 5, url, action: 'adopt', item, choice: null });
    await send({ kind: 'action', id: 5, url, action: 'download', item: null, choice: { kind: 'audio' } });
    expect(groups.orderedGroups).toHaveLength(1);
    expect(download).toHaveBeenCalledOnce();
    expect(states.getGroupState(groups.orderedGroups[0].id)).toBe(MediaState.downloading);
  });

  it('handles each message once, however many pulls return it', async () => {
    const { download, groups, afters, send } = await setup();
    await send({ kind: 'action', id: 6, url, action: 'download', item, choice: null });
    // Focus, the timer and a late wake all pull; an unacknowledged message comes back until the next pull.
    window.dispatchEvent(new Event('focus'));
    await emit('capture_wake');
    await flushPromises();
    expect(groups.orderedGroups).toHaveLength(1);
    expect(download).toHaveBeenCalledOnce();
    expect(afters.at(-1)).toBe(1);
  });

  it('pulls on a timer when no event arrives (Linux, tauri#3654)', async () => {
    vi.useFakeTimers();
    const { groups, afters } = await setup();
    const before = afters.length;
    await vi.advanceTimersByTimeAsync(1600);
    expect(afters.length).toBeGreaterThan(before);
    expect(groups.orderedGroups).toHaveLength(0);
  });

  it('passes the clipboard prefill on and opens Report for a failed read', async () => {
    const { send } = await setup();
    await send({ kind: 'prefill', url });
    expect(capturePrefill.value).toBe(url);
    await send({ kind: 'prefill', url: '' });
    expect(capturePrefill.value).toBe('');
    const open = vi.spyOn(useReportStore(), 'openReport').mockImplementation(() => {});
    await send({ kind: 'report', url, reason: 'login' });
    expect(open).toHaveBeenCalledExactlyOnceWith(expect.objectContaining({ url, context: expect.objectContaining({ errorCode: 'capture:login' }) }));
  });
});
