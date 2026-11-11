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
