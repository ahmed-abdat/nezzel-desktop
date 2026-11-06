import { MediaState } from '../stores/media/state';
import { TrackType, type DownloadOptions, type MediaAttachment, type MediaFormat, type SourceMediaKind } from '../tauri/types/media';
import type { Group } from '../tauri/types/group';
import { fileCategory, type FileCategory } from './fileCategory';
import { siteName } from './siteName';

/**
 * What one download group looks like to the UI: its phase, site, media kind and file kind.
 * Pure functions, so the row, the inspector, the status bar and the list filters cannot drift apart.
 */

/** Where a download is in its life; the one MediaState -> phase map every screen uses. */
export type Phase = 'reading' | 'playlist' | 'ready' | 'downloading' | 'paused' | 'done' | 'error';

const phaseByState: Record<MediaState, Phase> = {
  [MediaState.fetching]: 'reading',
  [MediaState.fetchingList]: 'reading',
  [MediaState.playlistSelection]: 'playlist',
  [MediaState.configure]: 'ready',
  [MediaState.downloading]: 'downloading',
  [MediaState.downloadingList]: 'downloading',
  [MediaState.paused]: 'paused',
  [MediaState.pausedList]: 'paused',
  [MediaState.done]: 'done',
  [MediaState.error]: 'error',
};

export const phaseOf = (state: MediaState): Phase => phaseByState[state];

/** Site name for a group: the extractor of the group, else of its first item, else the link's host. */
export function groupSite(group: Pick<Group, 'extractor' | 'items' | 'url'>): string | undefined {
  return siteName(group.extractor ?? group.items[Object.keys(group.items)[0]]?.extractor, group.url);
}

/** Category of a file download: from the saved file when it exists, else the link's title. Undefined for media. */
export function groupFileKind(group: Pick<Group, 'mediaKind' | 'title' | 'url'> | undefined, savedPath?: string): FileCategory | undefined {
  return group?.mediaKind === 'file' ? fileCategory(savedPath ?? group.title ?? group.url) : undefined;
}

export type MediaKind = SourceMediaKind;
export type KindFilter = 'video' | 'audio' | 'photos' | 'files';

/** Photo posts: one photo, a carousel, or photos mixed with a clip. */
export const isPhotoPost = (kind: SourceMediaKind | undefined): boolean => kind === 'image' || kind === 'carousel' || kind === 'mixed';

/** Posts that are only pictures (no clip): no format controls. Mixed posts have a clip, so they are not here. */
export const isImageOnly = (group: Pick<Group, 'mediaKind'> | undefined): boolean => group?.mediaKind === 'image' || group?.mediaKind === 'carousel';

/** Photo posts all go under Photos; unknown kinds fall under no filter. */
export function kindFilterOf(kind: string | null | undefined): KindFilter | null {
  if (kind === 'video' || kind === 'audio') return kind;
  if (kind === 'file') return 'files';
  return isPhotoPost(kind as SourceMediaKind) ? 'photos' : null;
}

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
 * True when the group has a picture to download (used to offer "download the video instead").
 * Stricter than `mediaKind`: a codec of none/unknown does not count as a picture.
 */
export function hasVideo(group: Pick<Group, 'mediaKind' | 'formats'>): boolean {
  return group.mediaKind === 'video' || !!group.formats?.some(format => format.height
    || format.videoCodecs.some(codec => !['', 'none', 'unknown'].includes((typeof codec === 'string' ? codec : codec.id).toLowerCase())));
}

/**
 * What a photo post saves, for its row: only photos -> photos, otherwise items (a clip is in it).
 * Undefined for video/audio media, which describe themselves by quality instead.
 */
export function savedAttachments(sourceKind: SourceMediaKind | undefined, attachments: MediaAttachment[], selectedIds: string[]): { key: 'photos' | 'items'; n: number } | undefined {
  if (!isPhotoPost(sourceKind)) return undefined;
  const selected = new Set(selectedIds);
  const chosen = attachments.filter(attachment => selected.has(attachment.id));
  const n = Math.max(chosen.length, 1);
  return chosen.length > 0 && chosen.every(attachment => attachment.kind === 'image') ? { key: 'photos', n } : { key: 'items', n };
}
