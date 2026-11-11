import { describe, expect, it } from 'vitest';
import {
  applyPlaylistSelectionToEntries,
  buildPlaylistItemsSpec,
  normalizePlaylistSelection,
  type PlaylistSelectionRow,
} from '../../src/helpers/playlistSelection.ts';

const entries = Array.from({ length: 12 }, (_, index) => ({
  index: index + 1,
  videoUrl: `https://example.com/watch?v=${index + 1}`,
}));

describe('playlist selection helpers', () => {
  it.each<{ name: string; row: PlaylistSelectionRow; expected: number; spec: string }>([
    { name: 'simple range', row: { id: 'range', type: 'range', start: 1, end: 1 }, expected: 1, spec: '1:1' },
    { name: 'advanced single item', row: { id: 'single', type: 'single', index: 2 }, expected: 2, spec: '2' },
  ])('applies the $name to a two-item playlist', ({ row, expected, spec }) => {
    const selection = { rows: [row] };
    const smallPlaylist = entries.slice(0, 2);
    expect(applyPlaylistSelectionToEntries(smallPlaylist, selection)).toEqual([smallPlaylist[expected - 1]]);
    expect(buildPlaylistItemsSpec(selection, smallPlaylist)).toBe(spec);
  });

  it('normalizes rows and removes empty values', () => {
    expect(normalizePlaylistSelection({
      rows: [
        { id: 'single', type: 'single', index: 4.8 },
        { id: 'empty', type: 'range', start: null, end: null },
        { id: 'range', type: 'range', start: 1, end: 5 },
      ],
    })).toEqual({
      rows: [
        { id: 'single', type: 'single', index: 4 },
        { id: 'range', type: 'range', start: 1, end: 5 },
      ],
    });
  });

  it('builds playlist item specs from structured rows', () => {
    expect(buildPlaylistItemsSpec({
      rows: [
        { id: 'single', type: 'single', index: 7 },
        { id: 'range-a', type: 'range', start: 1, end: 3 },
        { id: 'reverse', type: 'range', start: -4, end: -2 },
      ],
    }, entries)).toBe('7,1:3,-4:-2');
  });

  it('applies single and range selection locally', () => {
    expect(applyPlaylistSelectionToEntries(entries, {
      rows: [
        { id: 'single', type: 'single', index: 7 },
        { id: 'range', type: 'range', start: 1, end: 3 },
      ],
    }).map(entry => entry.index)).toEqual([7, 1, 2, 3]);
  });

  it('supports negative indexes and reverse ranges', () => {
    expect(applyPlaylistSelectionToEntries(entries, {
      rows: [
        { id: 'single', type: 'single', index: -1 },
        { id: 'range', type: 'range', start: -2, end: -4 },
      ],
    }).map(entry => entry.index)).toEqual([12, 11, 10, 9]);
  });

  it('uses the full playlist when no rows are configured', () => {
    expect(applyPlaylistSelectionToEntries(entries, { rows: [] }).map(entry => entry.index)).toEqual(
      entries.map(entry => entry.index),
    );
  });
});

describe('checklist selection', () => {
  it('keeps exactly the checked positions, including none, in playlist order', () => {
    const entries = [0, 3, 8].map(index => ({ index, videoUrl: `https://example.com/${index}` }));
    expect(applyPlaylistSelectionToEntries(entries, { rows: [], positions: [2, 0] })).toEqual([entries[0], entries[2]]);
    expect(buildPlaylistItemsSpec({ rows: [], positions: [0, 2] }, entries)).toBe('1,9');
    expect(applyPlaylistSelectionToEntries(entries, { rows: [], positions: [] })).toEqual([]);
  });
  it('omits the filter when all 10,000 entries are selected', () => {
    const entries = Array.from({ length: 10_000 }, (_, index) => ({ index, videoUrl: `https://example.com/${index}` }));
    expect(buildPlaylistItemsSpec({ rows: [], positions: entries.map((_, position) => position) }, entries)).toBeNull();
  });
  it('compresses consecutive original indices, preserving gaps', () => {
    const entries = [0, 2, 3, 4, 6, 8].map(index => ({ index, videoUrl: `https://example.com/${index}` }));
    expect(buildPlaylistItemsSpec({ rows: [], positions: [0, 1, 2, 3, 4] }, entries)).toBe('1,3:5,7');
  });
});
