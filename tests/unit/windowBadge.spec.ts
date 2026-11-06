import { beforeEach, describe, expect, it, vi } from 'vitest';
import { nextTick } from 'vue';
import { useMediaGroupStore } from '../../src/stores/media/group';
import { MediaState, useMediaStateStore } from '../../src/stores/media/state';
import { useMediaProgressStore } from '../../src/stores/media/progress';
import { ProgressCategory } from '../../src/tauri/types/progress';

const win = vi.hoisted(() => ({
  setBadgeCount: vi.fn(),
  setProgressBar: vi.fn(),
  requestUserAttention: vi.fn(),
}));

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => win,
  ProgressBarStatus: { None: 'none', Normal: 'normal', Indeterminate: 'indeterminate' },
  UserAttentionType: { Informational: 'informational' },
}));
vi.mock('../../src/tauri/notifications', () => ({
  notify: vi.fn(),
  notifyGroup: vi.fn(),
}));

import { startWindowWatcher } from '../../src/tauri/window';

function addGroup(id: string) {
  useMediaGroupStore().createGroup({
    id,
    url: id,
    total: 1,
    processed: 1,
    errored: 0,
    isCombined: false,
    audioCodecs: [],
    formats: [],
    filesize: 0,
    items: { [id]: { id, url: id, audioCodecs: [], formats: [], filesize: 0 } },
  });
}

function send(id: string, percentage: number, speedBps: number) {
  useMediaProgressStore().processMediaProgressPayload({
    id,
    groupId: id,
    category: ProgressCategory.other,
    percentage,
    speedBps,
  } as never);
}

async function flush() {
  await nextTick();
  await Promise.resolve();
}

describe('window badge watcher', () => {
  beforeEach(() => {
    win.setBadgeCount.mockReset();
    win.setProgressBar.mockReset();
    win.requestUserAttention.mockReset();
  });

  it('does not call the OS again when only speed changes', async () => {
    addGroup('a');
    useMediaStateStore().setState('a', MediaState.downloading);
    send('a', 41.2, 1000);
    startWindowWatcher();
    await flush();
    expect(win.setBadgeCount).toHaveBeenCalledTimes(1);
    expect(win.setProgressBar).toHaveBeenCalledTimes(1);

    for (let i = 0; i < 20; i++) {
      send('a', 41.2, 2000 + i);
      await flush();
    }
    expect(win.setBadgeCount).toHaveBeenCalledTimes(1);
    expect(win.setProgressBar).toHaveBeenCalledTimes(1);

    send('a', 42.1, 3000);
    await flush();
    expect(win.setProgressBar).toHaveBeenCalledTimes(2);
    expect(win.setProgressBar).toHaveBeenLastCalledWith({ status: 'normal', progress: 42 });
    expect(win.setBadgeCount).toHaveBeenCalledTimes(1);

    useMediaStateStore().setState('a', MediaState.done);
    await flush();
    expect(win.setProgressBar).toHaveBeenLastCalledWith({ status: 'none' });
    expect(win.setBadgeCount).toHaveBeenLastCalledWith(undefined);
  });

  it('uses done/total percent with two downloading items', async () => {
    addGroup('b');
    addGroup('c');
    addGroup('d');
    const state = useMediaStateStore();
    state.setState('b', MediaState.downloading);
    state.setState('c', MediaState.downloading);
    state.setState('d', MediaState.done);
    send('b', 90, 1);
    send('c', 10, 1);
    startWindowWatcher();
    await flush();
    // done 1 of total (downloading 2 + done 1)
    expect(win.setProgressBar).toHaveBeenLastCalledWith({ status: 'normal', progress: 33 });
    expect(win.setBadgeCount).toHaveBeenLastCalledWith(2);
  });
});
