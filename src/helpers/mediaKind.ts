import { TrackType, type DownloadOptions, type MediaAttachment, type MediaFormat, type SourceMediaKind } from '../tauri/types/media';

export type MediaKind = SourceMediaKind;

/**
 * What the user gets from a download, for the sidebar's Types filter.
 * Audio when they chose audio only, or when the source has no video stream at all; otherwise video.
 * Before the formats are read (empty list) we cannot know, so it counts as video.
 * Some extractors leave the video codec out; a format with a height still has a picture.
 */
export function mediaKind(formats: MediaFormat[], options?: Pick<DownloadOptions, 'trackType'>, sourceKind?: SourceMediaKind): MediaKind {
  if (sourceKind === 'image' || sourceKind === 'carousel' || sourceKind === 'mixed' || sourceKind === 'file') return sourceKind;
  if (sourceKind === 'audio') return 'audio';
  if (options?.trackType === TrackType.audio) return 'audio';
  if (sourceKind === 'video') return 'video';
  if (formats.length === 0) return 'video';
  return formats.some(format => format.videoCodecs.length > 0 || (format.height ?? 0) > 0) ? 'video' : 'audio';
}

/**
 * What a photo post saves, for its row: only photos → photos, otherwise items (a clip is in it).
 * Undefined for video/audio media, which describe themselves by quality instead.
 */
export function savedAttachments(sourceKind: SourceMediaKind | undefined, attachments: MediaAttachment[], selectedIds: string[]): { key: 'photos' | 'items'; n: number } | undefined {
  if (sourceKind !== 'image' && sourceKind !== 'carousel' && sourceKind !== 'mixed') return undefined;
  const selected = new Set(selectedIds);
  const chosen = attachments.filter(attachment => selected.has(attachment.id));
  const n = Math.max(chosen.length, 1);
  return chosen.length > 0 && chosen.every(attachment => attachment.kind === 'image') ? { key: 'photos', n } : { key: 'items', n };
}

/** Only passive formats launch a viewer; installers, scripts and unknown types open their folder. */
export function canOpenFile(path: string): boolean {
  const extension = path.split(/[\\/]/).pop()?.split('.').pop()?.toLowerCase();
  return !!extension && ['pdf', 'txt', 'csv', 'json', 'jpg', 'jpeg', 'png', 'webp', 'gif', 'avif', 'bmp', 'mp3', 'm4a', 'flac', 'wav', 'ogg', 'opus', 'mp4', 'mkv', 'webm', 'mov'].includes(extension);
}
