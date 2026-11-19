import type { Group, PlaylistParent } from '../tauri/types/group';
import type { DownloadOptions, DownloadOverrides, EncodingOptions, TrackOptions } from '../tauri/types/media';

export type SavedDownload = {
  group: Group;
  options: DownloadOptions;
  overrides?: DownloadOverrides;
  encodings?: EncodingOptions;
  tracks?: TrackOptions;
  attachmentIds?: string[];
  /** Playlist row this download sits under (no thumbnail: it can be a signed CDN link). */
  parent?: PlaylistParent;
};
const KEY = 'nezzel.pending-downloads.v1';

/** Persist replay inputs, never probe results, signed CDN URLs or session credentials. */
function safeOverrides(overrides?: DownloadOverrides): DownloadOverrides | undefined {
  return overrides
    ? {
        output: overrides.output, subtitles: overrides.subtitles, sponsorBlock: overrides.sponsorBlock,
        input: overrides.input, inputFilters: overrides.inputFilters,
      }
    : undefined;
}
const safeLink = (url?: string) => (typeof url === 'string' && /^https?:\/\//i.test(url) ? url : undefined);
export function encodeQueue(rows: SavedDownload[]): string {
  return JSON.stringify({ version: 1, rows: rows.map(row => ({
    group: {
      id: row.group.id, url: row.group.url, title: row.group.title, mediaKind: row.group.mediaKind,
      total: row.group.total, processed: row.group.processed, errored: row.group.errored,
      isCombined: row.group.isCombined, filesize: row.group.filesize, audioCodecs: [], formats: [],
      playlistId: row.group.playlistId, playlistTitle: row.group.playlistTitle,
      playlistCount: row.group.playlistCount ?? row.group.entries?.length,
      parentId: typeof row.group.parentId === 'string' ? row.group.parentId : undefined,
      parentIndex: typeof row.group.parentIndex === 'number' ? row.group.parentIndex : undefined,
      uploader: row.group.uploader, uploaderId: row.group.uploaderId,
      attachments: row.group.attachments?.map(({ id, kind }) => ({ id, kind })),
      items: Object.fromEntries(Object.entries(row.group.items).map(([key, item]) => [key, {
        id: item.id, groupId: item.groupId, url: item.url, title: item.title, isLeader: item.isLeader,
        fetchEngine: item.fetchEngine, mediaKind: item.mediaKind, filesize: item.filesize,
        playlistChild: item.playlistChild, playlistId: item.playlistId, playlistCount: item.playlistCount, playlistIndex: item.playlistIndex,
        uploader: item.uploader, audioCodecs: [], formats: [], entries: item.entries ? [] : undefined,
        overrides: safeOverrides(item.overrides),
      }])),
    },
    options: row.options, overrides: safeOverrides(row.overrides),
    encodings: row.encodings, tracks: row.tracks, attachmentIds: row.attachmentIds,
    parent: row.parent && typeof row.parent.id === 'string' && row.parent.id === row.group.parentId
      ? { id: row.parent.id, url: safeLink(row.parent.url), title: row.parent.title, count: Number(row.parent.count) || 0, skipped: 0 }
      : undefined,
  })) });
}
export function decodeQueue(raw: string | null): SavedDownload[] {
  if (!raw || raw.length > 16 * 1024 * 1024) return [];
  try {
    const data = JSON.parse(raw);
    if (data.version !== 1 || !Array.isArray(data.rows)) return [];
    const rows = data.rows.filter((row: SavedDownload) => row?.group && typeof row.group.id === 'string'
      && typeof row.group.url === 'string' && /^https?:\/\//i.test(row.group.url)
      && row.group.items && typeof row.group.items === 'object' && !Array.isArray(row.group.items)
      && Object.values(row.group.items).every(item => item && typeof item.id === 'string' && typeof item.url === 'string')
      && row.options && ['both', 'audio', 'video'].includes(row.options.trackType));
    // Also strip auth fields from older/tampered snapshots before they reach IPC.
    const safe: SavedDownload[] = JSON.parse(encodeQueue(rows)).rows;
    for (const row of safe) {
      // Selection metadata has no reusable CDN URLs; the backend resolves the source anew.
      row.group.attachments = row.group.attachments?.map(attachment => ({ ...attachment, url: '' }));
    }
    return safe;
  } catch { return []; }
}
export function readQueue(): SavedDownload[] {
  try {
    return decodeQueue(globalThis.localStorage?.getItem(KEY) ?? null);
  } catch { return []; }
}
export function writeQueue(rows: SavedDownload[]): void {
  try {
    globalThis.localStorage?.setItem(KEY, encodeQueue(rows));
  } catch { /* Full/unavailable storage must not break a running download. */ }
}
