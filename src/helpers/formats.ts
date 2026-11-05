import { MediaFormat } from '../tauri/types/media';

export function sortFormats(formats: MediaFormat[]): MediaFormat[] {
  const clone = [...formats];
  return clone.sort(sortFormat);
}

export function sortFormat(a: MediaFormat, b: MediaFormat): number {
  if (a.height && b.height) {
    const heightDiff = b.height - a.height;
    return heightDiff !== 0 ? heightDiff : (b.fps ?? 0) - (a.fps ?? 0);
  } else if (a.abr && b.abr) {
    return b.abr - a.abr;
  } else {
    return b.id.localeCompare(a.id);
  }
}

export function nearestBy<T>(
  items: T[],
  score: (x: T) => number,
  tieBreak?: (a: T, b: T) => number,
): T | undefined {
  let best: T | undefined;
  let bestScore = Infinity;
  for (const it of items) {
    const s = score(it);
    if (s < bestScore || (s === bestScore && tieBreak && best && tieBreak(it, best) > 0)) {
      best = it;
      bestScore = s;
    }
  }
  return best;
}

export function approxAudio(formats: MediaFormat[], targetAbr?: number) {
  const audio = formats.filter(f => f.abr != null);
  if (!audio.length) return undefined;

  if (targetAbr == null) {
    return audio.reduce((best, f) => ((f.abr ?? 0) > (best.abr ?? 0) ? f : best), audio[0]);
  }

  return nearestBy(
    audio,
    f => Math.abs((f.abr ?? 0) - targetAbr),
    (a, b) => (a.abr ?? 0) - (b.abr ?? 0),
  );
}

export function approxVideo(
  formats: MediaFormat[],
  targetHeight?: number,
  targetFps?: number,
) {
  const video = formats.filter(f => f.height != null);
  if (!video.length) return undefined;

  if (targetHeight == null && targetFps == null) {
    return video.reduce((best, f) => {
      const [bh, bf] = [best.height ?? 0, best.fps ?? 0];
      const [h, fps] = [f.height ?? 0, f.fps ?? 0];
      if (h !== bh) return h > bh ? f : best;
      if (fps !== bf) return fps > bf ? f : best;
      return best;
    }, video[0]);
  }

  if (targetHeight != null) {
    const sameHeight = video.filter(f => f.height === targetHeight);
    if (sameHeight.length) {
      if (targetFps == null) {
        return sameHeight.reduce(
          (best, f) => ((f.fps ?? 0) > (best.fps ?? 0) ? f : best),
          sameHeight[0],
        );
      }

      return nearestBy(
        sameHeight,
        f => Math.abs((f.fps ?? 0) - targetFps),
        (a, b) => (a.fps ?? 0) - (b.fps ?? 0),
      );
    }
  }

  return nearestBy(
    video,
    (f) => {
      const dh = Math.abs((f.height ?? 0) - (targetHeight ?? 0));
      const df = targetFps == null ? 0 : Math.abs((f.fps ?? 0) - targetFps);
      return dh * 1000 + df;
    },
    (a, b) =>
      ((a.height ?? 0) - (b.height ?? 0))
      || ((a.fps ?? 0) - (b.fps ?? 0)),
  );
}

/** Frame rates below this are the normal rate for a height and say nothing worth reading (24/25/30/48). */
const HIGH_FPS = 50;

/** Display label for a video quality: `1080p`, with the frame rate only when it is high (`1080p60`). */
export function qualityLabel(height: number, fps?: number | null): string {
  return fps != null && fps >= HIGH_FPS ? `${height}p${fps}` : `${height}p`;
}

/**
 * One entry per height for a quality list: the highest frame rate wins, so `1080p24 / 1080p30`
 * shows once. Formats without a height (audio) pass through. Order follows the input.
 * `keep` pins a specific format (e.g. the current selection) as its height's entry.
 */
export function oneFormatPerHeight<T extends Pick<MediaFormat, 'height' | 'fps'>>(formats: T[], keep?: (format: T) => boolean): T[] {
  const best = new Map<number, T>();
  for (const format of formats) {
    if (format.height == null) continue;
    const current = best.get(format.height);
    if (!current) best.set(format.height, format);
    else if (keep?.(current)) continue;
    else if (keep?.(format) || (format.fps ?? 0) > (current.fps ?? 0)) best.set(format.height, format);
  }
  return formats.filter(format => format.height == null || best.get(format.height) === format);
}
