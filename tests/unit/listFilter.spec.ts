import { describe, expect, it } from 'vitest';
import { countView, filterRows, finishedIds, kindFilterOf, matchesSearch, parseListView, statusOf, viewQuery, type ListRow, type ListView } from '../../src/helpers/listFilter';
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
    [v('video'), '', ['a', 'c', 'd']],
    [v('video', 'failed'), '', ['c']],
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

  it('matches harakat typed in the query against bare text', () => {
    expect(matchesSearch({ title: 'مرحبا', site: '' }, 'مَرْحَبًا')).toBe(true);
  });
});

describe('countView', () => {
  it('keeps photo posts and files out of the Video and Audio filters', () => {
    expect(['video', 'audio', 'image', 'carousel', 'mixed', 'file', null].map(kindFilterOf)).toEqual(['video', 'audio', 'photos', 'photos', 'photos', 'files', null]);
  });

  it('counts types over every status, and statuses within the picked type', () => {
    const all = [...rows, row('g', 'done', 'photos', 'Weekend hack', 'X')];
    expect(countView(all, 'all')).toEqual({ types: { all: 6, video: 3, audio: 1, photos: 1, files: 1 }, statuses: { all: 6, active: 1, failed: 1, done: 4 } });
    expect(countView(all, 'video').statuses).toEqual({ all: 3, active: 1, failed: 1, done: 1 });
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
