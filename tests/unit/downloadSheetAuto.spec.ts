import { beforeEach, describe, expect, it, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { nextTick } from 'vue';

const downloadGroup = vi.hoisted(() => vi.fn());
vi.mock('../../src/stores/media/media', () => ({ useMediaStore: () => ({ downloadGroup }) }));

import { useDownloadSheetStore } from '../../src/stores/downloadSheet';
import { useMediaGroupStore } from '../../src/stores/media/group';
import { MediaState, useMediaStateStore } from '../../src/stores/media/state';
import type { MediaFormat, MediaItem } from '../../src/tauri/types/media';

const fmt = (id: string, height?: number): MediaFormat => ({ id, height, audioCodecs: [], videoCodecs: height ? ['avc1'] : [] });

function setup(formats: MediaFormat[]) {
  const groups = useMediaGroupStore();
  const states = useMediaStateStore();
  const item = { id: 'i1', isLeader: true, formats } as unknown as MediaItem;
  groups.createGroup({ id: 'g1', total: 1, processed: 0, errored: 0, isCombined: false, items: { i1: item }, url: 'u', audioCodecs: [], formats, filesize: 0 });
  states.setGroupState('g1', MediaState.fetching);
  const sheet = useDownloadSheetStore();
  sheet.show('g1');
  return { sheet, states };
}

describe('options sheet when there is nothing to choose', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    downloadGroup.mockClear();
  });

  it('closes and downloads with best once the info shows a single way', async () => {
    const { sheet, states } = setup([fmt('img')]);
    await nextTick();
    expect(sheet.open).toBe(true);
    states.setGroupState('g1', MediaState.configure);
    await nextTick();
    expect(sheet.open).toBe(false);
    expect(downloadGroup).toHaveBeenCalledWith('g1', expect.objectContaining({ trackType: 'both', id: 'best' }));
  });

  it('stays open when there is a real choice', async () => {
    const { sheet, states } = setup([fmt('a', 1080), fmt('b', 720)]);
    states.setGroupState('g1', MediaState.configure);
    await nextTick();
    expect(sheet.open).toBe(true);
    expect(downloadGroup).not.toHaveBeenCalled();
  });

  it('never downloads when a ready row is reopened', async () => {
    const { sheet, states } = setup([fmt('img')]);
    sheet.close();
    states.setGroupState('g1', MediaState.configure);
    sheet.show('g1');
    await nextTick();
    expect(sheet.open).toBe(true);
    expect(downloadGroup).not.toHaveBeenCalled();
  });

  it('stays open when the user already touched the sheet', async () => {
    const { sheet, states } = setup([fmt('img')]);
    sheet.touch();
    states.setGroupState('g1', MediaState.configure);
    await nextTick();
    expect(sheet.open).toBe(true);
    expect(downloadGroup).not.toHaveBeenCalled();
  });
});
