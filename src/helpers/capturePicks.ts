import type { CaptureDetails } from '../captureTypes';
import { SMALL_MAX_HEIGHT } from './quickChoice';

/** Quick picks in the quick bar and the copied-link card. `best` is always first and the primary. */
export type PickKey = 'best' | 'audio' | 'small';
export interface CapturePick { key: PickKey; height: number | null; size: number | null }

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
  const kind = details.mediaKind ?? '';
  if (!details.bestHeight || kind === 'file' || kind === 'image' || kind === 'carousel') return [best];
  const picks = [best];
  if (details.hasAudio) picks.push({ key: 'audio', height: null, size: details.audioSize });
  if (details.smallHeight) picks.push({ key: 'small', height: details.smallHeight, size: details.smallSize });
  return picks;
}
