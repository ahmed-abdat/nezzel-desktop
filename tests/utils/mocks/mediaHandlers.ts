import { ensureInvokeArgsObject, IPCHandler } from '../tauriMock';
import { InvokeArgs } from '@tauri-apps/api/core';
import { emit } from '@tauri-apps/api/event';
import { EntryItem, MediaAddPayload, MediaFormat } from '../../../src/tauri/types/media';

const sampleFormats: MediaFormat[] = [
  { id: 'f1080', height: 1080, fps: 60, audioCodecs: [], videoCodecs: [] },
  { id: 'f720', height: 720, fps: 30, audioCodecs: [], videoCodecs: [] },
  { id: 'audio', abr: 120, audioCodecs: [], videoCodecs: [] },
];

export const mediaHandlers: Record<string, IPCHandler> = {
  // The detail page subscribes to the engine log; the real command always returns a string.
  logging_subscribe: (): string => '',
  media_info: (_cmd: string, args: InvokeArgs | undefined): string => {
    const { groupId, id, url } = ensureInvokeArgsObject<{ groupId: string; id: string; url: string }>(args, 'media_info');

    if (url.includes('image-carousel')) {
      void emit<MediaAddPayload>('media_add', {
        groupId, total: 1,
        item: {
          id, url, title: 'Test carousel', mediaKind: 'carousel',
          audioCodecs: [], formats: [], filesize: 0, isLeader: true,
          attachments: [1, 2, 3].map(number => ({
            id: `photo-${number}`, kind: 'image', url: `https://example.com/${number}.jpg`,
            thumbnail: '/src/assets/placeholder.png',
          })),
        },
      });
      return groupId;
    }

    // One photo or one plain file: nothing to choose, so the options sheet must skip itself.
    // Emitted after the command returns, as the engine does (media_info only queues the fetch).
    if (url.includes('single-image') || url.includes('single-file')) {
      const file = url.includes('single-file');
      setTimeout(() => void emit<MediaAddPayload>('media_add', {
        groupId, total: 1,
        item: {
          id, url, title: file ? 'report.pdf' : 'Single photo', mediaKind: file ? 'file' : 'image',
          audioCodecs: [], formats: [], filesize: file ? 2_400_000 : 0, isLeader: true,
          attachments: file ? undefined : [{ id: 'photo-1', kind: 'image', url: 'https://example.com/1.jpg', thumbnail: '/src/assets/placeholder.png' }],
        },
      }), 100);
      return groupId;
    }

    if (url.includes('playlist-small')) {
      void emitPlaylistLeader(id, url, groupId, 2);
      return groupId;
    }
    if (url.includes('playlist-large')) {
      void emitPlaylistLeader(id, url, groupId, 50);
      return groupId;
    }
    void emit<MediaAddPayload>('media_add', {
      groupId,
      total: 1,
      item: { id, url, title: 'Test Video', audioCodecs: ['aac'], formats: sampleFormats, filesize: 123, isLeader: true },
    });
    return 'test-group';
  },
  media_playlist_expand: (_cmd: string, args: InvokeArgs | undefined): string => {
    const { groupId, entries } = ensureInvokeArgsObject<{ groupId: string; entries: EntryItem[] }>(
      args,
      'media_playlist_expand',
    );

    void emitExpandedPlaylist(groupId, entries);
    return groupId;
  },
};

const emitPlaylistLeader = async (id: string, url: string, groupId: string, total: number): Promise<string> => {
  const entries = Array.from({ length: total }, (_, index) => ({
    index,
    videoUrl: `${url}?v=${index + 1}`,
  }));

  await emit<MediaAddPayload>('media_add', {
    groupId,
    total,
    item: {
      id: `${id}-pl`,
      url,
      title: 'Playlist',
      audioCodecs: ['aac'],
      formats: sampleFormats,
      filesize: 0,
      entries,
      playlistCount: total,
    },
  });
  return groupId;
};

async function emitExpandedPlaylist(groupId: string, entries: EntryItem[]): Promise<void> {
  for (const entry of entries) {
    await emit<MediaAddPayload>('media_add', {
      groupId,
      total: entries.length,
      item: {
        id: `${groupId}-${entry.index}`,
        url: entry.videoUrl,
        title: `Item ${entry.index + 1}`,
        audioCodecs: ['aac'],
        formats: sampleFormats,
        filesize: 0,
      },
    });
  }
}
