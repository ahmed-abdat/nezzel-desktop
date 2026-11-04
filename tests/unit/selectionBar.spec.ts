import { flushPromises, mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import SelectionBar from '../../src/components/downloads/SelectionBar.vue';
import { i18n } from '../../src/i18n';
import { useMediaStore } from '../../src/stores/media/media';
import { useDownloadSelection } from '../../src/composables/useDownloadSelection';
import { useMediaGroupStore } from '../../src/stores/media/group';
import { useMediaStateStore, MediaState } from '../../src/stores/media/state';
import { useToastStore } from '../../src/stores/toast';
import { useListHiddenStore } from '../../src/stores/listHidden';
import { installTauriMock } from '../utils/tauriMock';
import type { Group } from '../../src/tauri/types/group';

const group = (id: string) => ({ id, title: id, items: {} }) as unknown as Group;

describe('SelectionBar', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    useDownloadSelection().clear();
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
  });

  it('shows the count and removes every selected row through the per-row remove', async () => {
    vi.useFakeTimers();
    const groups = [group('a'), group('b'), group('c')];
    const selection = useDownloadSelection();
    selection.setOrder(['a', 'b', 'c']);
    selection.selectAll();
    const del = vi.spyOn(useMediaStore(), 'deleteGroup').mockImplementation(() => undefined as never);

    const wrapper = mount(SelectionBar, { props: { groups }, global: { plugins: [i18n] } });
    expect(wrapper.get('[role="toolbar"]').text()).toContain('3 selected');

    await wrapper.findAll('button').find(b => b.text() === 'Remove from list')!.trigger('click');
    expect(del).not.toHaveBeenCalled();
    expect([...useListHiddenStore().ids]).toEqual(['a', 'b', 'c']);
    expect(useToastStore().toasts).toHaveLength(1);
    useToastStore().toasts[0].action!.run();
    await vi.advanceTimersByTimeAsync(5000);
    expect([...useListHiddenStore().ids]).toEqual([]);
    expect(del).not.toHaveBeenCalled();
    expect(selection.count.value).toBe(0);
  });
  it.each([
    ['Download', MediaState.configure, 'media_download', 'Started 1'],
    ['Resume', MediaState.paused, 'media_download', 'Started 1'],
    ['Pause', MediaState.downloading, 'group_cancel', 'Paused 1'],
    ['Try again', MediaState.error, 'media_info', 'Retrying 1'],
  ] as const)('%s disables the entire bar until the batch settles and reports partial failure', async (label, state, command, message) => {
    installTauriMock({});
    const media = useMediaStore();
    const ids = await media.addUrlBatch(['https://example.com/a', 'https://example.com/b']);
    const groups = ids.map(id => useMediaGroupStore().findGroupById(id));
    ids.forEach(id => useMediaStateStore().setGroupState(id, state));
    const selection = useDownloadSelection();
    selection.setOrder(ids);
    selection.selectAll();
    const pending: { resolve: (value: unknown) => void; reject: (reason: Error) => void }[] = [];
    installTauriMock({ [command]: () => new Promise((resolve, reject) => {
      pending.push({ resolve, reject });
    }) });
    const wrapper = mount(SelectionBar, { props: { groups }, global: { plugins: [i18n] } });
    await wrapper.findAll('button').find(b => b.text() === label)!.trigger('click');
    expect(wrapper.findAll('button').every(b => b.attributes('disabled') !== undefined)).toBe(true);
    expect(pending).toHaveLength(2);
    pending[0].resolve('ok');
    pending[1].reject(new Error('engine unavailable'));
    await flushPromises();
    expect(useToastStore().toasts.map(toast => toast.message)).toContain(message);
    expect(useToastStore().toasts.map(toast => toast.message)).toContain('1 action failed');
    expect(useToastStore().toasts).toHaveLength(2);
    expect(wrapper.findAll('button').every(b => b.attributes('disabled') === undefined)).toBe(true);
  });
});
