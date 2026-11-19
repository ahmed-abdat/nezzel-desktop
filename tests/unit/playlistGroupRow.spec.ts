import HomeView from '../../src/views/app/HomeView.vue';
import { flushPromises, mount, type VueWrapper } from '@vue/test-utils';
import { createMemoryHistory, createRouter } from 'vue-router';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { i18n } from '../../src/i18n';
import { installTauriMock } from '../utils/tauriMock';
import { useMediaGroupStore } from '../../src/stores/media/group';
import { MediaState, useMediaStateStore } from '../../src/stores/media/state';
import { useMediaOptionsStore } from '../../src/stores/media/options';
import { usePlaylistStore } from '../../src/stores/media/playlist';
import { useToastStore } from '../../src/stores/toast';
import { TrackType } from '../../src/tauri/types/media';
import { playlistAggregate } from '../../src/helpers/playlistAggregate';
import { buildListTree, type TreeInput } from '../../src/helpers/listTree';

const { countUsage } = vi.hoisted(() => ({ countUsage: vi.fn() }));
vi.mock('../../src/tauri/usage', () => ({ countUsage }));

/** A playlist row with `states.length` children, the i-th child in states[i]. */
function seedPlaylist(states: MediaState[], id = 'pl') {
  const groups = useMediaGroupStore();
  const stateStore = useMediaStateStore();
  usePlaylistStore().register({ id, url: 'https://youtube.com/playlist?list=x', title: 'Artemis I: Path to the Pad', count: states.length, skipped: 0 });
  const children = states.map((state, index) => {
    const childId = `${id}-c${index}`;
    const item = { id: `${childId}-i`, isLeader: true, url: `https://youtube.com/watch?v=${index}`, title: `Part ${index + 1}`, audioCodecs: [], formats: [], filesize: 0 };
    stateStore.setState(item.id, state);
    useMediaOptionsStore().setOptions(childId, { trackType: TrackType.both });
    return { ...item, id: childId, parentId: id, parentIndex: index, total: 1, processed: 1, errored: 0, isCombined: false, items: { [item.id]: item } };
  });
  groups.prependGroups(children);
  return children;
}

async function mountHome(): Promise<VueWrapper> {
  const router = createRouter({ history: createMemoryHistory(), routes: [{ name: 'home', path: '/', component: HomeView }] });
  await router.push('/');
  const view = mount(HomeView, { attachTo: document.body, global: { plugins: [router, i18n] } });
  await flushPromises();
  return view;
}

let view: VueWrapper | undefined;
afterEach(() => {
  view?.unmount();
  view = undefined;
  document.documentElement.dir = 'ltr';
  countUsage.mockClear();
});

describe('playlist group row', () => {
  it('shows one row for the playlist with its items under it, and closes/opens with the chevron', async () => {
    seedPlaylist([MediaState.done, MediaState.downloading, MediaState.downloading]);
    view = await mountHome();
    const parent = view.get('li[data-playlist]');
    expect(parent.text()).toContain('Artemis I');
    expect(view.findAll('li.row[aria-level="2"]')).toHaveLength(3);
    const toggle = view.get('[data-testid="playlist-toggle"]');
    expect(toggle.attributes('aria-expanded')).toBe('true');
    await toggle.trigger('click');
    expect(view.findAll('li.row[aria-level="2"]')).toHaveLength(0);
    expect(toggle.attributes('aria-expanded')).toBe('false');
  });

  it('arrow keys open and close the row, mirrored in Arabic', async () => {
    document.documentElement.dir = 'rtl';
    seedPlaylist([MediaState.downloading, MediaState.downloading], 'rtl');
    view = await mountHome();
    const parent = view.get('li[data-playlist]');
    await parent.trigger('keydown', { key: 'ArrowRight' });
    expect(view.get('[data-testid="playlist-toggle"]').attributes('aria-expanded')).toBe('false');
    await parent.trigger('keydown', { key: 'ArrowLeft' });
    expect(view.get('[data-testid="playlist-toggle"]').attributes('aria-expanded')).toBe('true');
  });

  it('Pause all pauses each running item once and counts one pause', async () => {
    const cancelled: string[] = [];
    installTauriMock({ group_cancel: (_cmd, args) => void cancelled.push((args as { groupId: string }).groupId) });
    const children = seedPlaylist([MediaState.done, MediaState.downloading, MediaState.downloading]);
    view = await mountHome();
    await view.get('li[data-playlist] button[aria-label="Pause all"]').trigger('click');
    await flushPromises();
    expect(cancelled.sort()).toEqual([children[1].id, children[2].id]);
    expect(countUsage).toHaveBeenCalledTimes(1);
    expect(useMediaStateStore().getGroupState(children[1].id)).toBe(MediaState.paused);
    expect(view.get('li[data-playlist]').text()).toContain('Resume all');
  });

  it('Cancel all leaves saved items alone, and one Undo brings every item back', async () => {
    const started: string[] = [];
    installTauriMock({ group_cancel: () => undefined, media_download: (_cmd, args) => void started.push((args as { groupId: string }).groupId) });
    const children = seedPlaylist([MediaState.done, MediaState.downloading, MediaState.paused]);
    view = await mountHome();
    await view.get('li[data-playlist] button[aria-label="Cancel all"]').trigger('click');
    await flushPromises();
    const groups = useMediaGroupStore();
    expect(groups.findGroupById(children[0].id)).toBeDefined();
    const toast = useToastStore().toasts.at(-1)!;
    expect(toast.message).toBe('Playlist cancelled');
    toast.action!.run();
    await flushPromises();
    expect(started).toHaveLength(2);
  });

  it('folds saved items and waiting items behind one-line buttons', async () => {
    seedPlaylist([...Array(4).fill(MediaState.done), MediaState.downloading, ...Array(6).fill(MediaState.downloading)], 'folds');
    view = await mountHome();
    expect(view.get('[data-testid="playlist-show-saved"]').text()).toBe('Show 4 saved');
    // No progress yet: every running item counts as waiting; 3 show, the rest fold.
    expect(view.get('[data-testid="playlist-more-waiting"]').text()).toBe('4 more waiting');
    await view.get('[data-testid="playlist-more-waiting"]').trigger('click');
    await view.get('[data-testid="playlist-show-saved"]').trigger('click');
    expect(view.findAll('li.row[aria-level="2"]')).toHaveLength(11);
  });
});

// No wall-clock limits (testing policy: no clock-coupled tests). The budget is structural: one pass over the
// children, and folds that never mount more than a page of rows. Timings are logged for the record.
describe('playlist performance', () => {
  it('aggregate + list tree over 1000 items run in one pass', () => {
    const rows: TreeInput[] = Array.from({ length: 1000 }, (_, i) => ({ id: `c${i}`, status: 'active', kind: 'video', title: `Item ${i}`, site: 'youtube.com', parentId: 'p', parentIndex: i }));
    const facts = rows.map((_, i) => ({ phase: i < 400 ? 'done' as const : 'downloading' as const, queued: i > 404, percent: 30, size: 1e6, speedBps: 1e5 }));
    const start = performance.now();
    for (let i = 0; i < 10; i++) {
      playlistAggregate(facts);
      buildListTree(rows, { p: { id: 'p', title: 'Big' } }, { type: 'all', status: 'all' }, 'item 9');
    }
    console.info(`[perf] aggregate + tree over 1000 items: ${((performance.now() - start) / 10).toFixed(2)} ms`);
  });

  it('a 1000-item playlist mounts with its folds closed and opens a page of 100 rows at a time', async () => {
    seedPlaylist([...Array(400).fill(MediaState.done), ...Array(600).fill(MediaState.downloading)], 'big');
    const mountStart = performance.now();
    view = await mountHome();
    const mountMs = performance.now() - mountStart;
    expect(view.findAll('li.row[aria-level="2"]')).toHaveLength(3);
    expect(view.get('[data-testid="playlist-show-saved"]').text()).toBe('Show 100 saved');
    const openStart = performance.now();
    await view.get('[data-testid="playlist-more-waiting"]').trigger('click');
    const openMs = performance.now() - openStart;
    expect(view.findAll('li.row[aria-level="2"]')).toHaveLength(103);
    console.info(`[perf] 1000-item playlist: mount ${mountMs.toFixed(0)} ms, next 100 rows ${openMs.toFixed(0)} ms (jsdom)`);
  });
});

describe('playlist engine fan-out', () => {
  it('Cancel all on a big playlist never has more than 8 engine cancels in flight, and Undo still restores', async () => {
    let inFlight = 0;
    let peak = 0;
    const releases: (() => void)[] = [];
    installTauriMock({
      group_cancel: () => new Promise<void>((resolve) => {
        inFlight++;
        peak = Math.max(peak, inFlight);
        releases.push(() => {
          inFlight--;
          resolve();
        });
      }),
      media_download: () => undefined,
    });
    const children = seedPlaylist(Array(30).fill(MediaState.downloading), 'fanout');
    view = await mountHome();
    await view.get('li[data-playlist] button[aria-label="Cancel all"]').trigger('click');
    await flushPromises();
    expect(inFlight).toBe(8);
    while (releases.length) {
      releases.shift()!();
      await flushPromises();
    }
    expect(peak).toBe(8);
    // Rows flipped at once (re-keyed snapshots), not as the engine calls drained.
    expect(useMediaGroupStore().orderedGroups.filter(g => g.parentId === 'fanout')).toHaveLength(children.length);
    expect(useToastStore().toasts.at(-1)?.message).toBe('Playlist cancelled');
  });
});
