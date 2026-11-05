import { describe, it, vi, expect, afterEach } from 'vitest';
import { emit } from '@tauri-apps/api/event';
import { mockWindows } from '@tauri-apps/api/mocks';
import { effectScope } from 'vue';
import { flushPromises } from '@vue/test-utils';
import { installTauriMock } from '../utils/tauriMock';
import { CAPTURE_MEMORY_CAP, CAPTURE_MEMORY_MS, pruneCaptures, registerCaptureListeners } from '../../src/tauri/listeners/shortcuts';
import { registerMediaListeners } from '../../src/tauri/listeners/media';
import { useMediaStore } from '../../src/stores/media/media';
import { useMediaGroupStore } from '../../src/stores/media/group';
import { MediaState, useMediaStateStore } from '../../src/stores/media/state';
import { useDownloadSheetStore } from '../../src/stores/downloadSheet';
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

describe('capture listeners', () => {
  const url = 'https://www.youtube.com/watch?v=capture';
  const item = { id: 'capture-1', url, title: 'Captured title', audioCodecs: [], formats: [], filesize: 0 };
  let activeScope: ReturnType<typeof effectScope>;
  afterEach(() => {
    activeScope?.stop();
    vi.useRealTimers();
  });

  async function setup(pending: { actions: unknown[]; items: unknown[] }[] = []) {
    const info = vi.fn(() => new Promise(() => {}));
    const download = vi.fn().mockResolvedValue('started');
    const takePending = vi.fn(() => pending.shift() ?? { actions: [], items: [] });
    installTauriMock({ media_info: info, media_download: download, capture_take_pending: takePending });
    mockWindows('main');
    activeScope = effectScope();
    registerMediaListeners();
    await activeScope.run(() => registerCaptureListeners());
    return { info, download, groups: useMediaGroupStore(), states: useMediaStateStore(), sheet: useDownloadSheetStore() };
  }

  it('creates an options row and opens its sheet before preview metadata arrives', async () => {
    const { info, groups, states, sheet } = await setup();
    await emit('capture_action', { id: 1, url, action: 'options', item: null });
    expect(groups.orderedGroups).toHaveLength(1);
    const group = groups.orderedGroups[0];
    expect(states.getGroupState(group.id)).toBe(MediaState.fetching);
    expect(sheet.groupId).toBe(group.id);
    expect(sheet.open).toBe(true);
    expect(info).not.toHaveBeenCalled();
    await emit('capture_item', { id: 1, item });
    await flushPromises();
    expect(groups.orderedGroups).toHaveLength(1);
    expect(groups.findGroupById(group.id).title).toBe('Captured title');
    expect(states.getGroupState(group.id)).toBe(MediaState.configure);
    expect(info).not.toHaveBeenCalled();
  });

  it('falls back once for the same row when preview fails, ignoring duplicate and late items', async () => {
    const { info, groups } = await setup();
    await emit('capture_action', { id: 2, url, action: 'adopt' });
    const group = groups.orderedGroups[0];
    const leaderId = Object.keys(group.items)[0];
    expect(info).not.toHaveBeenCalled();
    await emit('capture_item', { id: 2, item: null });
    await emit('capture_item', { id: 2, item: null });
    await emit('capture_item', { id: 2, item });
    expect(info).toHaveBeenCalledExactlyOnceWith('media_info', expect.objectContaining({ url, groupId: group.id, id: leaderId }));
    expect(groups.orderedGroups).toHaveLength(1);
    expect(groups.findGroupById(group.id).title).toBeUndefined();
  });

  it('adopts at the top without a sheet or download and suppresses recent clipboard duplicates', async () => {
    vi.useFakeTimers();
    const { info, download, groups, sheet } = await setup();
    await useMediaStore().dispatchMediaInfoFetch('https://example.com/old', false, false, { ...item, id: 'old' });
    await emit('capture_action', { id: 3, url, action: 'adopt' });
    await emit('capture_action', { id: 3, url, action: 'adopt' });
    expect(groups.orderedGroups).toHaveLength(2);
    expect(groups.orderedGroups[0].url).toBe(url);
    expect(sheet.open).toBe(false);
    expect(useMediaStore().hasRecentCapture(url)).toBe(true);
    await emit('capture_item', { id: 3, item });
    await flushPromises();
    expect(info).not.toHaveBeenCalled();
    expect(download).not.toHaveBeenCalled();
    expect(sheet.open).toBe(false);
    vi.advanceTimersByTime(5001);
    expect(useMediaStore().hasRecentCapture(url)).toBe(false);
  });

  it.each(['pending', 'cached', 'fallback'])('downloads once after preview readiness (%s)', async (preview) => {
    const { info, download, groups, states, sheet } = await setup();
    await emit('capture_action', { id: 4, url, action: 'download', item: preview === 'cached' ? item : null });
    const group = groups.orderedGroups[0];
    if (preview !== 'cached') {
      expect(states.getGroupState(group.id)).toBe(MediaState.fetching);
      expect(download).not.toHaveBeenCalled();
      if (preview === 'fallback') {
        info.mockImplementation(() => emit('media_add', { groupId: group.id, total: 1, item }));
      }
      await emit('capture_item', { id: 4, item: preview === 'fallback' ? null : item });
      await emit('capture_item', { id: 4, item });
    }
    await flushPromises();
    expect(download).toHaveBeenCalledOnce();
    expect(states.getGroupState(group.id)).toBe(MediaState.downloading);
    expect(info).toHaveBeenCalledTimes(preview === 'fallback' ? 1 : 0);
    expect(sheet.open).toBe(false);
  });

  it('a Download click that races focus adoption starts the adopted row once', async () => {
    const { download, groups, states } = await setup();
    await emit('capture_action', { id: 5, url, action: 'adopt' });
    await emit('capture_action', { id: 5, url, action: 'download', item });
    await emit('capture_action', { id: 5, url, action: 'download', item });
    await flushPromises();
    expect(groups.orderedGroups).toHaveLength(1);
    expect(download).toHaveBeenCalledOnce();
    expect(states.getGroupState(groups.orderedGroups[0].id)).toBe(MediaState.downloading);
  });

  it('a card Download whose event never arrives still starts once from the pull (Linux, tauri#3654)', async () => {
    const { download, groups, states } = await setup([{ actions: [{ id: 6, url, action: 'download', item }], items: [] }]);
    window.dispatchEvent(new Event('focus'));
    await flushPromises();
    expect(groups.orderedGroups).toHaveLength(1);
    // The held event turns up later: same capture id, no second row or download.
    await emit('capture_action', { id: 6, url, action: 'download', item });
    await flushPromises();
    expect(groups.orderedGroups).toHaveLength(1);
    expect(download).toHaveBeenCalledOnce();
    expect(states.getGroupState(groups.orderedGroups[0].id)).toBe(MediaState.downloading);
  });

  it('focus does not adopt a card offer for a link the bar just downloaded', async () => {
    const { download, groups } = await setup();
    await emit('capture_action', { id: 7, url, action: 'download', item });
    await emit('capture_action', { id: 8, url, action: 'adopt' });
    await flushPromises();
    expect(groups.orderedGroups).toHaveLength(1);
    expect(download).toHaveBeenCalledOnce();
  });

  it('remembers capture ids for two minutes and at most the cap', () => {
    const captures = new Map<number, { at: number }>([[1, { at: 0 }], [2, { at: CAPTURE_MEMORY_MS }]]);
    pruneCaptures(captures, CAPTURE_MEMORY_MS);
    expect([...captures.keys()]).toEqual([2]);
    for (let id = 3; id < CAPTURE_MEMORY_CAP + 10; id++) captures.set(id, { at: CAPTURE_MEMORY_MS });
    pruneCaptures(captures, CAPTURE_MEMORY_MS);
    expect(captures.size).toBe(CAPTURE_MEMORY_CAP);
    expect(captures.has(2)).toBe(false);
  });
});
