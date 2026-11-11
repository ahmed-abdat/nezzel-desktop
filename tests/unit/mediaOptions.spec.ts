import { describe, it, expect } from 'vitest';
import { useMediaOptionsStore } from '../../src/stores/media/options';
import { TrackType, MediaFormat } from '../../src/tauri/types/media';
import { useMediaGroupStore } from '../../src/stores/media/group';

describe('media options store', () => {
  it('keeps attachment selection per group, in source order, independently of format choices', () => {
    const groupStore = useMediaGroupStore();
    const store = useMediaOptionsStore();
    groupStore.createGroup({
      id: 'photos', url: 'https://example.com/post', total: 1, processed: 1, errored: 0,
      isCombined: false, audioCodecs: [], formats: [], filesize: 0, items: {},
      mediaKind: 'mixed',
      attachments: [
        { id: 'photo-1', kind: 'image', url: 'https://example.com/1.jpg' },
        { id: 'video-2', kind: 'video', url: 'https://example.com/2' },
        { id: 'photo-3', kind: 'image', url: 'https://example.com/3.jpg' },
      ],
    });
    groupStore.createGroup({
      ...groupStore.groups.photos,
      id: 'other',
      attachments: groupStore.groups.photos.attachments?.map(attachment => ({ ...attachment })),
    });
    expect(store.getAttachmentSelection('photos')).toEqual(['photo-1', 'video-2', 'photo-3']);
    store.setAttachmentSelection('photos', ['photo-3', 'photo-3', 'gone', 'photo-1']);
    store.applyGlobalOptions({ trackType: TrackType.audio });
    expect(store.getAttachmentSelection('photos')).toEqual(['photo-1', 'photo-3']);
    expect(store.getAttachmentSelection('other')).toEqual(['photo-1', 'video-2', 'photo-3']);
    groupStore.groups.photos.attachments!.shift();
    expect(store.getAttachmentSelection('photos')).toEqual(['photo-3']);
    store.setAttachmentSelection('photos', []);
    expect(store.getAttachmentSelection('photos')).toEqual([]);
    store.removeOptions('photos');
    expect(store.getAttachmentSelection('photos')).toEqual(['video-2', 'photo-3']);
  });

  it('isolates playlist checks by group and removes them with the group options', () => {
    const store = useMediaOptionsStore();
    const group = {
      id: 'playlist', url: 'https://example.com/list', total: 2, processed: 0, errored: 0,
      isCombined: false, audioCodecs: [], formats: [], filesize: 0, items: {},
      entries: [0, 1].map(index => ({ index, videoUrl: `https://example.com/${index}` })),
    };
    store.setPlaylistSelection(group, [0]);
    expect(store.getPlaylistSelection(group)).toEqual([0]);
    expect(store.getPlaylistSelection({ ...group, id: 'other' })).toEqual([0, 1]);
    store.removeOptions(group.id);
    expect(store.getPlaylistSelection(group)).toEqual([0, 1]);
  });

  it('stores and retrieves options for groups', () => {
    const store = useMediaOptionsStore();
    store.setOptions('g1', { trackType: TrackType.audio });
    expect(store.getOptions('g1')).toEqual({ trackType: TrackType.audio });
    store.removeOptions('g1');
    expect(store.getOptions('g1')).toBeUndefined();
  });

  it('applies global options to all groups', () => {
    const groupStore = useMediaGroupStore();
    const store = useMediaOptionsStore();
    const fmt720: MediaFormat = { id: '720', height: 720, fps: 30, audioCodecs: [], videoCodecs: [] };
    const fmt480: MediaFormat = { id: '480', height: 480, fps: 30, audioCodecs: [], videoCodecs: [] };

    groupStore.createGroup({
      id: 'g1',
      url: 'g1',
      total: 1,
      processed: 1,
      errored: 0,
      isCombined: true,
      audioCodecs: [],
      formats: [fmt720, fmt480],
      filesize: 0,
      items: { a: { id: 'a', url: 'a', audioCodecs: [], formats: [fmt720, fmt480], filesize: 0, isLeader: true } },
    });
    groupStore.createGroup({
      id: 'g2',
      url: 'g2',
      total: 1,
      processed: 1,
      errored: 0,
      isCombined: true,
      audioCodecs: [],
      formats: [fmt720, fmt480],
      filesize: 0,
      items: { b: { id: 'b', url: 'b', audioCodecs: [], formats: [fmt480], filesize: 0, isLeader: true } },
    });

    store.applyGlobalOptions({ trackType: TrackType.both, ...fmt720 });

    expect(store.getOptions('g1')).toEqual({ trackType: TrackType.both, ...fmt720 });
    expect(store.getOptions('g2')).toEqual({ trackType: TrackType.both, ...fmt720 });

    store.applyGlobalOptions({ trackType: TrackType.audio });
    expect(store.getOptions('g1')).toEqual({ trackType: TrackType.audio });
    expect(store.getOptions('g2')).toEqual({ trackType: TrackType.audio });
  });
});
