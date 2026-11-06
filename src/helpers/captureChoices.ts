import type { CaptureDetails, CardChoice } from '../captureTypes';
import { GENERIC_HEIGHTS, HIGH_FPS_MIN, SMALL_MAX_HEIGHT, type CaptureChoice } from '@/generated/contract';
import { TrackType, type DownloadOptions, type MediaFormat, type SourceMediaKind } from '../tauri/types/media';
import { BEST_AUDIO, optionsForChoice, videoFormats } from './quickChoice';

/*
 * What a captured link offers: the quick bar's picks and the copied-link card's format rows. Both send a
 * `CaptureChoice` object (Rust `capture::Choice` checks it and keeps only what the offer showed), and the main
 * window turns it into download options with `optionsForCaptureChoice`.
 */

/** Photos, files and posts have one way to save: no Audio/Smaller pick, no format menu. */
const NO_MENU = new Set(['file', 'image', 'carousel']);

/** Quick picks in the bar. `best` is always first and the primary. */
export type PickKey = 'best' | 'audio' | 'small';
export interface CapturePick { key: PickKey; height: number | null; size: number | null }

/** What a pick sends. Best sends none: the main window applies the user's own default (Settings), exactly like Enter. */
export function pickChoice(key: PickKey): CaptureChoice | undefined {
  if (key === 'audio') return { kind: 'audio' };
  if (key === 'small') return { kind: 'small' };
  return undefined;
}

/**
 * Which picks a link offers. Before the read every pick works (the engine falls back: Smaller = best ≤720p,
 * else the nearest lower), so all three show. After the read only picks that change something stay:
 * no Smaller with one quality, no Audio without sound, nothing but Download for audio-only links, photos and files.
 */
export function capturePicks(details: CaptureDetails, read: boolean): CapturePick[] {
  if (!read) {
    return [
      { key: 'best', height: null, size: null },
      { key: 'audio', height: null, size: null },
      { key: 'small', height: SMALL_MAX_HEIGHT, size: null },
    ];
  }
  const best: CapturePick = { key: 'best', height: details.bestHeight, size: details.size };
  if (!details.bestHeight || NO_MENU.has(details.mediaKind ?? '')) return [best];
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
  /** Generic rows (still reading, playlists): the height is a ceiling per item, not a known format. */
  upTo: boolean;
}

function rowKey(choice: RowChoice): string {
  if (choice.kind === 'best') return 'best';
  if (choice.kind === 'audio') return choice.kbps ? `audio:${choice.kbps}` : 'audio';
  return choice.fps ? `video:${choice.height}:${choice.fps}` : `video:${choice.height}`;
}

const row = (choice: RowChoice, size: number | null, upTo = false): CardRow =>
  ({ key: rowKey(choice), choice, size, best: choice.kind === 'best', upTo });

function genericRows(withAudio: boolean): CardRow[] {
  return [
    row({ kind: 'best' }, null),
    ...GENERIC_HEIGHTS.map(height => row({ kind: 'video', height }, null, true)),
    ...(withAudio ? [row({ kind: 'audio' }, null)] : []),
  ];
}

/** Rust sends the rows checked, deduped and ordered (videos highest first, 60 fps before 30, then audio). */
function readRows(choices: CardChoice[]): CardRow[] {
  const rows = choices.map(choice => row(choice.choice, choice.size));
  const firstVideo = rows.find(r => r.choice.kind === 'video');
  if (firstVideo) firstVideo.best = true;
  return rows;
}

/**
 * Rows for the card's ▾ menu, or [] when the card shows a plain Download button. Before the read the menu
 * already works with generic ceilings (the engine falls back to the nearest lower height); after the read
 * it lists the link's own formats. A menu needs at least two ways to save.
 */
export function cardRows(details: CaptureDetails, read: boolean): CardRow[] {
  if (NO_MENU.has(details.mediaKind ?? '')) return [];
  if (!read) return genericRows(true);
  const rows = readRows(details.choices ?? []);
  // Read, but no format list (a playlist or a page of links): generic ceilings apply to every item.
  if (rows.length === 0) return details.bestHeight || details.mediaKind === 'audio' ? [] : genericRows(details.hasAudio || !details.mediaKind);
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
