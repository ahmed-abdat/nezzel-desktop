import type { CaptureDetails, CardChoice } from '../captureTypes';
import { HIGH_FPS_MIN, type CaptureChoice } from '@/generated/contract';
import { TrackType, type DownloadOptions, type MediaFormat, type SourceMediaKind } from '../tauri/types/media';
import { BEST_AUDIO, optionsForChoice, videoFormats } from './quickChoice';

/*
 * What a captured link offers: the quick bar's picks and the copied-link card's format rows. Both send a
 * `CaptureChoice` object (Rust `capture::Choice` checks it and keeps only what the offer showed), and the main
 * window turns it into download options with `optionsForCaptureChoice`.
 */

/** Photos, files and posts have one way to save: no Audio/Smaller pick, no format menu. */
const NO_MENU = new Set(['file', 'image', 'carousel']);

/** Only these supported hosts establish audio-only media before metadata arrives. */
const AUDIO_SITES = ['soundcloud.com', 'mixcloud.com', 'bandcamp.com', 'audiomack.com'];
export function isKnownAudioSite(url: string): boolean {
  try {
    const { hostname, protocol } = new URL(url);
    return ['http:', 'https:'].includes(protocol)
      && AUDIO_SITES.some(host => hostname === host || hostname.endsWith(`.${host}`));
  } catch {
    return false;
  }
}

/** Quick picks in the bar. `best` is always first and the primary. */
export type PickKey = 'best' | 'audio' | 'small';
export interface CapturePick { key: PickKey; height: number | null; size: number | null }

/** What a pick sends. Best sends none: the main window applies the user's own default (Settings), exactly like Enter. */
export function pickChoice(key: PickKey): CaptureChoice | undefined {
  if (key === 'audio') return { kind: 'audio' };
  if (key === 'small') return { kind: 'small' };
  return undefined;
}

/** No speculative picks while reading. The primary default action is rendered separately. */
export function capturePicks(details: CaptureDetails, read: boolean, url = ''): CapturePick[] {
  if (details.mediaKind === 'playlist') return [];
  const best: CapturePick = { key: 'best', height: details.bestHeight, size: details.size };
  if (!read) return isKnownAudioSite(url) ? [best, { key: 'audio', height: null, size: null }] : [];
  if (!details.bestHeight || NO_MENU.has(details.mediaKind ?? '') || details.mediaKind === 'audio') return [best];
  const picks = [best];
  if (details.hasAudio) picks.push({ key: 'audio', height: null, size: details.audioSize });
  if (details.smallHeight) picks.push({ key: 'small', height: details.smallHeight, size: details.smallSize });
  return picks;
}

/** What a card row sends: Download best, a height (+ frame rate tier) or an audio bitrate. */
export type RowChoice = Extract<CaptureChoice, { kind: 'best' | 'video' | 'audio' }>;

/** One row of the copied-link card's format menu. */
export interface CardRow {
  /** DOM key (`v-for`, `data-choice`) only: never sent, never parsed. */
  key: string;
  choice: RowChoice;
  size: number | null;
  /** The first video row: what Download best gets. */
  best: boolean;
  /** Whether a height is a ceiling rather than a known format. */
  upTo: boolean;
}

function rowKey(choice: RowChoice): string {
  if (choice.kind === 'best') return 'best';
  if (choice.kind === 'audio') return choice.kbps ? `audio:${choice.kbps}` : 'audio';
  return choice.fps ? `video:${choice.height}:${choice.fps}` : `video:${choice.height}`;
}

const row = (choice: RowChoice, size: number | null, upTo = false): CardRow =>
  ({ key: rowKey(choice), choice, size, best: choice.kind === 'best', upTo });

/** Rust sends the rows checked, deduped and ordered (videos highest first, 60 fps before 30, then audio). */
function readRows(choices: CardChoice[]): CardRow[] {
  const rows = choices.map(choice => row(choice.choice, choice.size));
  const firstVideo = rows.find(r => r.choice.kind === 'video');
  if (firstVideo) firstVideo.best = true;
  return rows;
}

/** Before the read, only a known audio host can offer a format. Afterward, use the real rows. */
export function cardRows(details: CaptureDetails, read: boolean, url = ''): CardRow[] {
  if (NO_MENU.has(details.mediaKind ?? '') || details.mediaKind === 'playlist') return [];
  if (!read) return isKnownAudioSite(url) ? [row({ kind: 'audio' }, null)] : [];
  const choices = (details.choices ?? []).filter(c => details.mediaKind !== 'audio' || c.choice.kind === 'audio');
  const rows = readRows(choices);
  return rows.length >= 2 ? rows : [];
}

/**
 * Options for a choice from the bar or the card: a quick pick, or one of the card's format rows. A height row takes
 * the read format of that height (and frame rate tier) when the group has one, else asks the engine for that height
 * as a ceiling (playlists, links read without formats). An audio row asks for audio at that bitrate.
 */
export function optionsForCaptureChoice(choice: CaptureChoice, formats: MediaFormat[], sourceKind?: SourceMediaKind): DownloadOptions {
  if (choice.kind === 'best' || choice.kind === 'small') return optionsForChoice(choice.kind, formats, sourceKind);
  if (choice.kind === 'audio') return choice.kbps ? { ...BEST_AUDIO, abr: choice.kbps } : optionsForChoice('audio', formats, sourceKind);
  const { height, fps } = choice;
  const highFps = (f: MediaFormat) => (f.fps ?? 0) >= HIGH_FPS_MIN;
  const sameTier = (f: MediaFormat) => f.height === height
    && (fps ? Math.round(f.fps ?? 0) === fps : !highFps(f));
  const video = videoFormats(formats);
  const format = video.find(sameTier) ?? video.find(f => f.height === height);
  if (format) return { trackType: TrackType.both, ...format };
  return { ...BEST_AUDIO, trackType: TrackType.both, height, ...(fps ? { fps } : {}) };
}
