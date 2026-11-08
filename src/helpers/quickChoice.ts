import { DownloadOptions, MediaCodec, MediaFormat, TrackType, type SourceMediaKind } from '../tauri/types/media';
import type { EntryItem } from '../tauri/types/media';
import { sortFormats } from './formats';
import { SMALL_MAX_HEIGHT } from '@/generated/contract';

/** The three plain choices of the download sheet. `custom` = picked in Advanced. */
export type QuickChoice = 'best' | 'audio' | 'small';

export const BEST_AUDIO: DownloadOptions = Object.freeze({
  trackType: TrackType.audio,
  id: 'best',
  audioCodecs: [],
  videoCodecs: [],
  audioTrackIds: [],
  videoTrackIds: [],
}) as DownloadOptions;

function codecId(codec: MediaCodec): string {
  return (typeof codec === 'string' ? codec : codec.id).trim().toLowerCase();
}

/** Missing codecs leave audio possible; only explicit silence rules it out for video. */
export function hasAudio(media: Pick<ChoiceSubject, 'formats' | 'audioCodecs' | 'mediaKind'>): boolean {
  if (media.mediaKind === 'file' || media.mediaKind === 'image') return false;
  if (media.formats.length > 0 && media.formats.every(f => f.audioCodecs.length > 0 && f.audioCodecs.every(c => codecId(c) === 'none'))) return false;
  return media.mediaKind === 'video' || media.mediaKind === 'mixed' || media.mediaKind === 'audio'
    || media.formats.some(f => f.height || f.videoCodecs.some(c => codecId(c) !== 'none')
      || (f.abr ?? 0) > 0 || f.audioCodecs.some(c => codecId(c) !== 'none'))
    || !!media.audioCodecs?.some(c => codecId(c) !== 'none');
}

export function videoFormats(formats: MediaFormat[]): MediaFormat[] {
  return sortFormats(formats.filter(f => f.height));
}

/** Options the engine receives for a plain choice, shaped like the Advanced picker's output. */
export function optionsForChoice(choice: QuickChoice, formats: MediaFormat[], sourceKind?: SourceMediaKind): DownloadOptions {
  const video = videoFormats(formats);
  const videoSource = sourceKind === 'video' || sourceKind === 'carousel' || sourceKind === 'mixed';
  const audio = hasAudio({ formats, mediaKind: sourceKind });
  if (audio && (choice === 'audio' || (video.length === 0 && !videoSource))) {
    return { ...BEST_AUDIO };
  }
  // Direct videos and post attachments may have no quality list; save both tracks by default. Smaller still
  // caps the height: the engine turns it into best ≤720p, else the nearest lower ([height<=720] fallbacks).
  if (video.length === 0) return choice === 'small' && videoSource
    ? { ...BEST_AUDIO, trackType: TrackType.both, height: SMALL_MAX_HEIGHT }
    : { ...BEST_AUDIO, trackType: TrackType.both };
  if (choice === 'small') {
    const small = video.find(f => (f.height ?? 0) <= SMALL_MAX_HEIGHT) ?? video[video.length - 1];
    return { trackType: TrackType.both, ...small };
  }
  return { trackType: TrackType.both, ...video[0] };
}

/** Main-window Start and quick-panel Best share the same saved-options/default policy. */
export function defaultDownloadOptions(
  media: Pick<ChoiceSubject, 'formats' | 'audioCodecs' | 'mediaKind'>,
  stored: DownloadOptions | undefined,
  defaultChoice: QuickChoice,
): DownloadOptions {
  return !stored || (stored.trackType === TrackType.audio && !hasAudio(media))
    ? optionsForChoice(defaultChoice, media.formats ?? [], media.mediaKind)
    : stored;
}

/** Which plain choice an options object corresponds to, or `custom`. */
export function choiceForOptions(options: DownloadOptions | undefined, formats: MediaFormat[], sourceKind?: SourceMediaKind): QuickChoice | 'custom' {
  if (!options) return 'best';
  if (options.trackType === TrackType.audio) {
    return !hasAudio({ formats, mediaKind: sourceKind }) ? 'best' : !options.abr ? 'audio' : 'custom';
  }
  if (options.trackType === TrackType.video) return 'custom';
  const video = videoFormats(formats);
  if (video.length === 0) return 'best';
  const same = (a: DownloadOptions, b: DownloadOptions) => a.height === b.height && (a.fps ?? null) === (b.fps ?? null);
  if (same(options, optionsForChoice('best', formats))) return 'best';
  if (same(options, optionsForChoice('small', formats))) return 'small';
  return 'custom';
}

/** Height the choice resolves to for these formats (undefined for audio or unknown). */
export function choiceHeight(choice: QuickChoice, formats: MediaFormat[]): number | undefined {
  return optionsForChoice(choice, formats).height;
}

/** The part of a read group `hasRealChoice` looks at. */
export interface ChoiceSubject {
  formats: MediaFormat[];
  audioCodecs?: MediaCodec[];
  entries?: EntryItem[];
  isCombined?: boolean;
  total?: number;
  mediaKind?: SourceMediaKind;
  attachments?: { kind: 'image' | 'video' }[];
}

/**
 * Whether the options sheet has anything to pick once the link is read. False for a photo or file,
 * a single video with one quality and no sound track, or one audio format: there is only one way
 * to download it. True for playlists and multi-item posts, several heights, or video that has
 * audio (audio only is then a real alternative).
 */
export function hasRealChoice(media: ChoiceSubject): boolean {
  // Posts: several attachments = the picker is a real choice; photos and files have no formats to pick.
  if ((media.attachments?.length ?? 0) > 1) return true;
  if (media.mediaKind === 'file' || media.mediaKind === 'image') return false;
  if (media.attachments?.length === 1 && media.attachments[0].kind === 'image') return false;
  // Nothing known about the source (no kind, no formats): keep the sheet rather than guess.
  if (!media.mediaKind && !media.attachments?.length && media.formats.length === 0 && !(media.total && media.total > 1)) return true;
  if (media.isCombined || (media.entries?.length ?? 0) > 0 || (media.total ?? 1) > 1) return true;
  const video = media.formats.filter(f => f.height || f.videoCodecs.length > 0);
  const audioAvailable = hasAudio(media);
  if (video.length > 0 || media.mediaKind === 'video' || media.mediaKind === 'mixed') {
    const heights = new Set(video.map(f => f.height ?? 0));
    return heights.size >= 2 || audioAvailable;
  }
  const audio = media.formats.filter(f => (f.abr ?? 0) > 0 || f.audioCodecs.some(c => codecId(c) !== 'none'));
  return audio.length >= 2;
}
