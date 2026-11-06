import { describe, it, vi, expect, afterEach } from 'vitest';
import { emit } from '@tauri-apps/api/event';
import { mockWindows } from '@tauri-apps/api/mocks';
import { effectScope } from 'vue';
import { flushPromises } from '@vue/test-utils';
import { installTauriMock } from '../utils/tauriMock';
import { capturePrefill, startCaptureDelivery, type CaptureMessage } from '../../src/tauri/listeners/capture';
import { registerMediaListeners } from '../../src/tauri/listeners/media';
import { useMediaGroupStore } from '../../src/stores/media/group';
import { MediaState, useMediaStateStore } from '../../src/stores/media/state';
import { useDownloadSheetStore } from '../../src/stores/downloadSheet';
import { useReportStore } from '../../src/stores/report';
import tauriListeners from '../../src/plugins/tauriListeners';

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
