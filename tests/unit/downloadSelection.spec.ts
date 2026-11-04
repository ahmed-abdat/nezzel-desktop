import { describe, expect, it } from 'vitest';
import { batchTargets, pruneIds, rangeIds, selectAllIds, toggleId } from '../../src/helpers/selection';

const order = ['a', 'b', 'c', 'd', 'e'];

describe('selection helpers', () => {
  it.each([
    [[], 'a', ['a']],
    [['a'], 'a', []],
    [['a', 'b'], 'c', ['a', 'b', 'c']],
  ])('toggle %j + %s', (start, id, expected) => {
    expect([...toggleId(new Set(start), id)].sort()).toEqual(expected);
  });

  it('toggle does not mutate its input', () => {
    const start = new Set(['a']);
    toggleId(start, 'b');
    expect([...start]).toEqual(['a']);
  });

  it.each([
    ['forward', 'b', 'd', ['b', 'c', 'd']],
    ['backward', 'd', 'b', ['b', 'c', 'd']],
    ['same row', 'c', 'c', ['c']],
    ['no anchor', null, 'c', ['c']],
    ['anchor gone from view', 'zzz', 'c', ['c']],
    ['target not visible', 'a', 'zzz', []],
  ])('range %s', (_name, anchor, target, expected) => {
    expect([...rangeIds(order, anchor, target)].sort()).toEqual(expected);
  });

  it('range follows the filtered order, not the full list', () => {
    // b and d are visible; c is filtered out and must not be picked up.
    expect([...rangeIds(['a', 'b', 'd', 'e'], 'a', 'd')]).toEqual(['a', 'b', 'd']);
  });

  it('select all covers exactly the visible rows', () => {
    expect([...selectAllIds(['a', 'd'])]).toEqual(['a', 'd']);
  });

  it('prune drops rows that disappeared', () => {
    expect([...pruneIds(new Set(['a', 'c', 'x']), ['a', 'b', 'c'])]).toEqual(['a', 'c']);
    expect(pruneIds(new Set(['a']), []).size).toBe(0);
  });

  it('batch targets pick the rows each action fits', () => {
    const t = batchTargets([
      { id: '1', phase: 'ready', hasFile: false },
      { id: '2', phase: 'downloading', hasFile: false },
      { id: '3', phase: 'paused', hasFile: false },
      { id: '4', phase: 'error', hasFile: false },
      { id: '5', phase: 'done', hasFile: true },
      { id: '6', phase: 'done', hasFile: false },
    ]);
    expect(t).toEqual({
      download: ['1'], pause: ['2'], resume: ['3'], retry: ['4'],
      remove: ['1', '2', '3', '4', '5', '6'], deleteFiles: ['5'],
    });
  });
});
