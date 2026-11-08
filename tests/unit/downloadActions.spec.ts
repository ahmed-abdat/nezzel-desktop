import { flushPromises, mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { defineComponent, ref } from 'vue';
import { i18n } from '../../src/i18n';
import { useDownloadActions, type TrashResult } from '../../src/composables/useDownloadActions';
import { useMediaDiagnosticsStore } from '../../src/stores/media/diagnostics';
import { useMediaOptionsStore } from '../../src/stores/media/options';
import { useMediaStore } from '../../src/stores/media/media';
import { useMediaDestinationStore } from '../../src/stores/media/destination';
import { useMediaGroupStore } from '../../src/stores/media/group';
import { MediaState, useMediaStateStore } from '../../src/stores/media/state';
import { useToastStore } from '../../src/stores/toast';
import { useListVisibilityStore } from '../../src/stores/listVisibility';
import { resetPlatformForTests } from '../../src/composables/usePlatform';
import { Platform } from '../../src/tauri/types/app';
import { MAX_CLEARED } from '../../src/stores/listVisibility';
import { installTauriMock } from '../utils/tauriMock';
import type { Group } from '../../src/tauri/types/group';

/** Runs trashFile for one finished row whose engine answers `reply` (undefined = success, string = refusal code). */
async function trash(reply: string | undefined) {
  mockIPC((cmd) => {
    if (cmd === 'media_trash_file') return reply === undefined ? null : Promise.reject(reply);
  });
  vi.spyOn(useMediaDestinationStore(), 'getPrimaryDestination').mockReturnValue({ path: '/d/clip.mp4' } as never);
  const del = vi.spyOn(useMediaStore(), 'deleteGroup').mockImplementation(() => undefined as never);
  let result: TrashResult | undefined;
  mount(defineComponent({
    setup() {
      const { trashFile } = useDownloadActions(ref({ id: 'g' } as Group));
      void trashFile().then((r) => {
        result = r;
      });
      return () => null;
    },
  }), { global: { plugins: [i18n] } });
  await vi.waitFor(() => expect(result).toBeDefined());
  return { result, removed: del.mock.calls.length > 0 };
}

describe('trashFile', () => {
  beforeEach(() => setActivePinia(createPinia()));
  afterEach(() => {
    vi.useRealTimers();
    clearMocks();
    vi.restoreAllMocks();
  });

  it.each([
    [undefined, 'ok', true],
    // Already gone from disk: nothing left to delete, so the row goes too.
    ['missing', 'missing', true],
    ['notOurs', 'notOurs', false],
    ['changed', 'changed', false],
    ['something else', 'trashFailed', false],
  ] as const)('engine %s → %s, row removed: %s', async (reply, expected, removed) => {
    expect(await trash(reply)).toEqual({ result: expected, removed });
  });

  it('a file saved before receipts (notOurs) offers Remove from list and stops offering Delete', async () => {
    mockIPC((cmd) => {
      // The confirm dialog answers with the label of the button pressed.
      if (cmd === 'plugin:dialog|message') return i18n.global.t('downloads.row.deleteConfirm.ok.trash');
      if (cmd === 'media_trash_file') return Promise.reject('notOurs');
    });
    resetPlatformForTests(Platform.MacOS);
    vi.spyOn(useMediaDestinationStore(), 'getPrimaryDestination').mockReturnValue({ path: '/d/old.mp4' } as never);
    const showToast = vi.spyOn(useToastStore(), 'showToast');
    const { actions } = actionsFor({ id: 'legacy' } as Group);
    expect(actions.canTrash.value).toBe(true);

    await actions.deleteFile();

    expect(actions.canTrash.value).toBe(false);
    const [, options] = showToast.mock.calls.at(-1)!;
    expect(options?.action?.label).toBe('Remove from list (keeps the file)');
    options!.action!.run();
    expect(useListVisibilityStore().isHidden('legacy')).toBe(true);
  });
});

function actionsFor(group: Group) {
  let actions!: ReturnType<typeof useDownloadActions>;
  const wrapper = mount(defineComponent({
    setup() {
      actions = useDownloadActions(ref(group));
      return () => null;
    },
  }), { global: { plugins: [i18n] } });
  return { actions, wrapper };
}

// Same row actions are used by the menu and Delete/Backspace handler.
describe('download row recovery', () => {
  afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
  });

  it.each([true, false])('recovers a no-audio failure with Best (video available: %s)', async (video) => {
    const groups = useMediaGroupStore();
    const format = { id: 'v', height: 720, audioCodecs: [], videoCodecs: ['h264'] };
    groups.createGroup({ id: 'silent', url: 'https://t.me/durov/531', title: 'Silent', mediaKind: video ? 'video' : undefined, total: 1, processed: 0, errored: 0, isCombined: false, audioCodecs: [], formats: video ? [format] : [], filesize: 0, items: {} });
    useMediaDiagnosticsStore().processMediaDiagnosticPayload({ id: 'i', groupId: 'silent', code: 'noAudioTrack', level: 'error', component: null, message: '', raw: '', timestamp: 1 });
    const download = vi.spyOn(useMediaStore(), 'downloadGroup').mockResolvedValue(true);
    const retry = vi.spyOn(useMediaStore(), 'retryGroup').mockResolvedValue(undefined);
    const { actions, wrapper } = actionsFor(groups.findGroupById('silent'));
    try {
      expect(await actions.recover()).toBe(true);
      if (video) expect(download).toHaveBeenCalledWith('silent', expect.objectContaining({ trackType: 'both', height: 720 }));
      else {
        expect(retry).toHaveBeenCalledWith('silent');
        expect(download).not.toHaveBeenCalled();
      }
      expect(useMediaOptionsStore().getOptions('silent')?.trackType).toBe('both');
    } finally { wrapper.unmount(); }
  });

  it('retries the existing row in place and returns it to error if refetch fails', async () => {
    installTauriMock({});
    const media = useMediaStore();
    const id = await media.dispatchMediaInfoFetch('https://example.com/a');
    await media.dispatchMediaInfoFetch('https://example.com/newer');
    const groups = useMediaGroupStore();
    const original = groups.findGroupById(id);
    const order = [...groups.groupOrder];
    useMediaStateStore().setGroupState(id, MediaState.error);
    let reject!: (reason: Error) => void;
    installTauriMock({ media_info: () => new Promise((_resolve, fail) => {
      reject = fail;
    }) });
    const { actions } = actionsFor(original);
    const attempt = actions.retry();
    expect(useMediaStateStore().getGroupState(id)).toBe(MediaState.fetching);
    expect(groups.groupOrder).toEqual(order);
    expect(groups.findGroupById(id)).toBe(original);
    reject(new Error('queue closed'));
    await attempt;
    expect(useMediaStateStore().getGroupState(id)).toBe(MediaState.error);
    expect(groups.groupOrder).toEqual(order);
  });

  it('stops a cancelled download immediately and offers requeue for five seconds', async () => {
    vi.useFakeTimers();
    const cancelled = vi.fn();
    installTauriMock({ group_cancel: cancelled });
    const groups = useMediaGroupStore();
    groups.createGroup({ id: 'cancel-toast', url: 'https://example.com/a', total: 1, processed: 1, errored: 0, isCombined: false,
      audioCodecs: [], formats: [], filesize: 0, items: { item: { id: 'item', isLeader: true, url: 'https://example.com/a', audioCodecs: [], formats: [], filesize: 0 } } });
    useMediaStateStore().setGroupState('cancel-toast', MediaState.downloading);
    const { actions, wrapper } = actionsFor(groups.findGroupById('cancel-toast'));
    try {
      actions.cancel();
      expect(cancelled).toHaveBeenCalledWith('group_cancel', { groupId: 'cancel-toast', discardFiles: true });
      expect(groups.orderedGroups).toHaveLength(0);
      const toast = useToastStore().toasts.at(-1)!;
      expect(toast).toMatchObject({ message: 'Download cancelled', duration: 5000, action: { label: 'Undo' } });
      toast.action!.run();
      await vi.advanceTimersByTimeAsync(0);
      expect(groups.orderedGroups).toHaveLength(1);
      expect(useMediaStateStore().getGroupState(groups.orderedGroups[0].id)).toBe(MediaState.downloading);
      await vi.advanceTimersByTimeAsync(5000);
      expect(useToastStore().toasts).not.toContainEqual(toast);
    } finally { wrapper.unmount(); }
  });

  it('hides immediately, allows Undo, and cancels only after an unrevoked removal expires', async () => {
    vi.useFakeTimers();
    const cancelled: unknown[] = [];
    installTauriMock({ group_cancel: (_cmd, args) => {
      cancelled.push(args);
    } });
    const media = useMediaStore();
    const id = await media.dispatchMediaInfoFetch('https://example.com/a');
    useMediaStateStore().setGroupState(id, MediaState.downloading);
    const groups = useMediaGroupStore();
    const { actions } = actionsFor(groups.findGroupById(id));
    actions.remove();
    expect(useListVisibilityStore().isHidden(id)).toBe(true);
    expect(cancelled).toEqual([]);
    expect(groups.findGroupById(id)).toBeDefined();
    useToastStore().toasts.at(-1)!.action!.run();
    await vi.advanceTimersByTimeAsync(5000);
    expect(useListVisibilityStore().isHidden(id)).toBe(false);
    expect(groups.findGroupById(id)).toBeDefined();
    expect(cancelled).toEqual([]);
    actions.remove();
    await vi.advanceTimersByTimeAsync(5000);
    expect(groups.findGroupById(id)).toBeUndefined();
    expect(cancelled).toEqual([{ groupId: id, discardFiles: true }]);
  });

  it('takes a finished row off the list without touching its history, and survives a restart', async () => {
    vi.useFakeTimers();
    localStorage.clear();
    const engine: string[] = [];
    installTauriMock({
      library_remove: () => { engine.push('library_remove'); },
      group_cancel: () => { engine.push('group_cancel'); },
    });
    const media = useMediaStore();
    const id = await media.dispatchMediaInfoFetch('https://example.com/a');
    useMediaDestinationStore().addDestination(id, { path: '/Downloads/a.mp4', confidence: 100 });
    useMediaStateStore().setGroupState(id, MediaState.done);
    const { actions } = actionsFor(useMediaGroupStore().findGroupById(id));
    actions.remove();
    await vi.advanceTimersByTimeAsync(6000);
    expect(useMediaGroupStore().findGroupById(id)).toBeDefined();
    expect(useListVisibilityStore().isCleared(['/Downloads/a.mp4'])).toBe(true);
    expect(engine).toEqual([]);
    // A new session (fresh store) still knows the row was removed from the list.
    setActivePinia(createPinia());
    expect(useListVisibilityStore().isCleared(['/Downloads/a.mp4'])).toBe(true);
    useListVisibilityStore().restore(['/Downloads/a.mp4']);
  });

  it('forgets the removed-from-list mark once the file is trashed, so a new download to that path shows', async () => {
    localStorage.clear();
    installTauriMock({ media_trash_file: () => undefined, library_remove: () => undefined, group_cancel: () => undefined });
    const media = useMediaStore();
    const id = await media.dispatchMediaInfoFetch('https://example.com/a');
    useMediaDestinationStore().addDestination(id, { path: '/Downloads/a.mp4', confidence: 100 });
    useMediaStateStore().setGroupState(id, MediaState.done);
    useListVisibilityStore().clear(['/Downloads/a.mp4']);
    const { actions } = actionsFor(useMediaGroupStore().findGroupById(id));
    expect(await actions.trashFile()).toBe('ok');
    expect(useListVisibilityStore().isCleared(['/Downloads/a.mp4'])).toBe(false);
  });

  it('keeps only the newest removed-from-list marks, on load and on insert', () => {
    localStorage.setItem('nezzel.listCleared', JSON.stringify(Array.from({ length: MAX_CLEARED + 10 }, (_, i) => `/d/${i}`)));
    const store = useListVisibilityStore();
    expect(store.keys.size).toBe(MAX_CLEARED);
    expect(store.isCleared(['/d/9'])).toBe(false);
    store.clear(['/d/new']);
    expect(store.keys.size).toBe(MAX_CLEARED);
    expect(store.isCleared(['/d/new', `/d/${MAX_CLEARED + 9}`])).toBe(true);
    expect(store.isCleared(['/d/10'])).toBe(false);
    localStorage.clear();
  });

  it.each(['openFile', 'revealFile'] as const)('offers removal when %s cannot find the file', async (action) => {
    installTauriMock({});
    const media = useMediaStore();
    const id = await media.dispatchMediaInfoFetch('https://example.com/a');
    vi.spyOn(useMediaDestinationStore(), 'getPrimaryDestination').mockReturnValue({ path: '/gone.mp4' } as never);
    installTauriMock({
      'plugin:opener|open_path': () => Promise.reject('missing'),
      'plugin:opener|reveal_item_in_dir': () => Promise.reject('missing'),
    });
    const { actions } = actionsFor(useMediaGroupStore().findGroupById(id));
    await actions[action]();
    await flushPromises();
    const toast = useToastStore().toasts.at(-1)!;
    expect(toast).toMatchObject({ message: 'File not found', style: 'error', action: { label: 'Remove from list' } });
    toast.action!.run();
    expect(useListVisibilityStore().isHidden(id)).toBe(true);
  });
});
