import { buildListTree, parentStatus, type TreeInput } from '../../src/helpers/listTree';
import { describe, expect, it } from 'vitest';
import { categoryNoticeCount, needsAttention, countView, filterRows, finishedIds, matchesSearch, parseListView, statusOf, viewQuery, type ListRow, type ListView } from '../../src/helpers/listFilter';
import { kindFilterOf } from '../../src/helpers/groupView';
import { MediaState } from '../../src/stores/media/state';

const row = (id: string, status: ListRow['status'], kind: ListRow['kind'], title: string, site = ''): ListRow & { id: string } => ({ id, status, kind, title, site });

const rows = [
  row('a', 'active', 'video', 'Lofi beats to study to', 'YouTube'),
  row('b', 'done', 'audio', 'مَرْحَبًا بِالعَالَمِ', 'SoundCloud'),
  row('c', 'failed', 'video', 'Café del Mar live', 'Vimeo'),
  row('d', 'done', 'video', 'أغنية الصباح', 'TikTok'),
  row('e', 'done', 'files', 'report.pdf', 'Drive'),
];
const ids = (list: { id: string }[]) => list.map(r => r.id);

describe('statusOf', () => {
  it.each([
    [MediaState.done, 'done'],
    [MediaState.error, 'failed'],
    [MediaState.cancelled, 'cancelled'],
    [MediaState.downloading, 'active'],
    [MediaState.paused, 'active'],
    [MediaState.configure, 'active'],
    [MediaState.fetching, 'active'],
    [undefined, 'active'],
  ] as const)('%s → %s', (state, expected) => {
    expect(statusOf(state)).toBe(expected);
  });
});

describe('parseListView', () => {
  it('keeps cancelled rows in All without counting them as active, completed, or failed', () => {
    const cancelled = [row('cancelled', statusOf(MediaState.cancelled), 'video', 'Clip')];
    expect(ids(filterRows(cancelled, { type: 'all', status: 'all' }, ''))).toEqual(['cancelled']);
    expect(countView(cancelled, 'all').statuses).toEqual({ all: 1, active: 0, failed: 0, done: 0 });
  });
  it.each([
    [{}, { type: 'all', status: 'all' }],
    [{ type: 'video', status: 'failed' }, { type: 'video', status: 'failed' }],
    [{ type: 'nope', status: 'done' }, { type: 'all', status: 'done' }],
    [{ type: 'nope', status: 'nope' }, { type: 'all', status: 'all' }],
    // Old single-filter links keep working.
    [{ filter: 'files' }, { type: 'files', status: 'all' }],
    [{ filter: 'failed' }, { type: 'all', status: 'failed' }],
    [{ filter: 'finished' }, { type: 'all', status: 'all' }],
  ] as const)('%o', (query, expected) => {
    expect(parseListView(query)).toEqual(expected);
  });

  it('round-trips through the route query, leaving defaults out', () => {
    expect(viewQuery({ type: 'all', status: 'all' })).toEqual({});
    const view: ListView = { type: 'photos', status: 'active' };
    expect(parseListView(viewQuery(view))).toEqual(view);
  });
});

const v = (type: ListView['type'], status: ListView['status'] = 'all'): ListView => ({ type, status });

describe('filterRows', () => {
  it.each([
    [v('all'), '', ['a', 'b', 'c', 'd', 'e']],
    [v('all', 'active'), '', ['a']],
    [v('all', 'failed'), '', ['c']],
    [v('video'), '', ['a', 'd']],
    [v('video', 'failed'), '', []],
    [v('audio', 'failed'), '', []],
    [v('files'), '', ['e']],
    [v('all'), 'youtube', ['a']],
    [v('all'), 'LOFI study', ['a']],
    [v('all'), 'cafe', ['c']],
    [v('all'), 'مرحبا العالم', ['b']],
    [v('all'), 'اغنيه', ['d']],
    [v('audio'), 'cafe', []],
    [v('all'), '   ', ['a', 'b', 'c', 'd', 'e']],
  ] as const)('view %o, query "%s"', (view, query, expected) => {
    expect(ids(filterRows(rows, view, query))).toEqual(expected);
  });

  it.each(['video', 'audio', 'photos', 'files'] as const)('excludes failures from %s but keeps active and finished rows', (kind) => {
    const list = [row('failed', 'failed', kind, ''), row('active', 'active', kind, ''), row('done', 'done', kind, '')];
    expect(ids(filterRows(list, v(kind), ''))).toEqual(['active', 'done']);
    expect(ids(filterRows(list, v('all', 'failed'), ''))).toEqual(['failed']);
  });

  it('matches harakat typed in the query against bare text', () => {
    expect(matchesSearch({ title: 'مرحبا', site: '' }, 'مَرْحَبًا')).toBe(true);
  });
});

describe('category notices', () => {
  const list = [
    row('v1', 'failed', 'video', ''), row('v2', 'failed', 'video', ''), row('a1', 'failed', 'audio', ''),
    { ...row('login', 'failed', 'video', ''), needsAttention: true },
    { ...row('ready', 'active', 'video', ''), needsAttention: true },
    row('done', 'done', 'video', ''), row('running', 'active', 'video', ''),
  ];

  it.each([['video', 4], ['audio', 1], ['photos', 0], ['files', 0]] as const)('counts hidden failures and attention rows only for %s', (type, expected) => {
    expect(categoryNoticeCount(list, type)).toBe(expected);
  });

  it('counts two video failures separately from an audio failure', () => {
    expect(categoryNoticeCount(list.slice(0, 3), 'video')).toBe(2);
  });

  it('hides attention rows from categories but preserves them in All downloads', () => {
    expect(ids(filterRows(list, v('video'), ''))).toEqual(['done', 'running']);
    expect(ids(filterRows(list, v('all'), ''))).toEqual(list.map(row => row.id));
    expect(ids(filterRows(list, v('all', 'failed'), ''))).toEqual(['v1', 'v2', 'a1', 'login']);
  });
});

describe('countView', () => {
  it('keeps photo posts and files out of the Video and Audio filters', () => {
    expect(['video', 'audio', 'image', 'carousel', 'mixed', 'file', null].map(kindFilterOf)).toEqual(['video', 'audio', 'photos', 'photos', 'photos', 'files', null]);
  });

  it('counts types over every status, and statuses within the picked type', () => {
    const all = [...rows, row('g', 'done', 'photos', 'Weekend hack', 'X')];
    expect(countView(all, 'all')).toEqual({ types: { all: 6, video: 2, audio: 1, photos: 1, files: 1 }, statuses: { all: 6, active: 1, failed: 1, done: 4 } });
    expect(countView(all, 'video').statuses).toEqual({ all: 2, active: 1, failed: 0, done: 1 });
  });
});

describe('finishedIds', () => {
  it('returns done rows only, never active or failed', () => {
    expect(finishedIds(rows)).toEqual(['b', 'd', 'e']);
  });

  it('skips rows already removed from the list', () => {
    expect(finishedIds([...rows, { ...row('f', 'done', 'video', 'x'), cleared: true }])).toEqual(['b', 'd', 'e']);
  });
});

describe('rows removed from the list', () => {
  const list = [...rows, { ...row('f', 'done', 'video', 'Old clip'), cleared: true }];

  it('leave All and every type, but stay under Completed', () => {
    expect(ids(filterRows(list, { type: 'all', status: 'all' }, ''))).not.toContain('f');
    expect(ids(filterRows(list, { type: 'video', status: 'all' }, ''))).not.toContain('f');
    expect(ids(filterRows(list, { type: 'all', status: 'done' }, ''))).toEqual(['b', 'd', 'e', 'f']);
    expect(ids(filterRows(list, { type: 'video', status: 'done' }, ''))).toEqual(['d', 'f']);
  });

  it('do not count in the sidebar, only in the Completed chip', () => {
    const { types, statuses } = countView(list, 'all');
    expect(types.all).toBe(5);
    expect(statuses).toEqual({ all: 5, active: 1, failed: 1, done: 4 });
  });
});

describe('needsAttention', () => {
  it.each([
    [MediaState.configure, false, true],
    [MediaState.playlistSelection, false, true],
    [MediaState.fetching, true, true],
    [MediaState.error, true, true],
    [MediaState.fetching, false, false],
    [MediaState.downloading, false, false],
    [MediaState.paused, false, false],
    [MediaState.done, false, false],
    [MediaState.error, false, false],
  ] as const)('state %s, login %s → %s', (state, login, expected) => {
    expect(needsAttention(state, login)).toBe(expected);
  });
});

describe('buildListTree', () => {
  const row = (id: string, status: ListRow['status'], extra: Partial<TreeInput> = {}): TreeInput => ({ id, status, kind: 'video', title: id, site: 'youtube.com', ...extra });
  const parents = { p: { id: 'p', title: 'Artemis series' } };
  const all: ListView = { type: 'all', status: 'all' };

  it('folds children under one node where the first child was, in playlist order', () => {
    const rows = [row('single', 'done'), row('c2', 'active', { parentId: 'p', parentIndex: 1 }), row('c1', 'done', { parentId: 'p', parentIndex: 0 }), row('last', 'done')];
    const tree = buildListTree(rows, parents, all);
    expect(tree.map(node => node.id)).toEqual(['single', 'p', 'last']);
    const node = tree[1];
    expect(node.kind === 'playlist' && node.all.map(r => r.id)).toEqual(['c1', 'c2']);
  });

  it.each<[string, TreeInput['status'][], string, boolean]>([
    ['any child running → in progress', ['done', 'active', 'failed'], 'active', false],
    ['all finished but one failed → failed', ['done', 'failed'], 'failed', false],
    ['all finished → done', ['done', 'done'], 'done', false],
    ['cancelled children need the user', ['done', 'cancelled'], 'cancelled', true],
  ])('section: %s', (_name, statuses, status, attention) => {
    const rows = statuses.map((s, i) => row(`c${i}`, s, { parentId: 'p', parentIndex: i, needsAttention: s === 'cancelled' }));
    expect(parentStatus(rows)).toEqual({ status, needsAttention: attention });
  });

  it('search: the playlist title shows all children; a child title shows only matching children', () => {
    const rows = [row('Launch day', 'done', { parentId: 'p', parentIndex: 0 }), row('Rollout', 'done', { parentId: 'p', parentIndex: 1 })];
    const byTitle = buildListTree(rows, parents, all, 'artemis')[0];
    expect(byTitle.kind === 'playlist' && byTitle.shown).toHaveLength(2);
    const byChild = buildListTree(rows, parents, all, 'launch')[0];
    expect(byChild.kind === 'playlist' && byChild.shown.map(r => r.id)).toEqual(['Launch day']);
    expect(buildListTree(rows, parents, all, 'nothing')).toEqual([]);
  });

  it('status chip: the playlist shows only children with that status, under that status', () => {
    const rows = [row('a', 'done', { parentId: 'p', parentIndex: 0 }), row('b', 'failed', { parentId: 'p', parentIndex: 1 }), row('c', 'active', { parentId: 'p', parentIndex: 2 })];
    const [node] = buildListTree(rows, parents, { type: 'all', status: 'failed' });
    expect(node.kind === 'playlist' && node.shown.map(r => r.id)).toEqual(['b']);
    expect(node.kind === 'playlist' && node.status).toBe('failed');
  });

  it('children of an unknown parent stay plain rows', () => {
    expect(buildListTree([row('x', 'done', { parentId: 'gone' })], parents, all).map(node => node.kind)).toEqual(['row']);
  });
});
