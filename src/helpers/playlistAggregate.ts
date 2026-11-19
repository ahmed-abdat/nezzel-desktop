import type { Phase } from './groupView';

/** What the playlist row needs to know about one of its downloads. */
export type PlaylistChildFacts = {
  phase: Phase;
  /** Engine says it waits for a free slot. */
  queued?: boolean;
  /** 0-100 while downloading or paused, when the engine has a number. */
  percent?: number | null;
  /** Expected size in bytes (estimate or engine total), 0/undefined when unknown. */
  size?: number;
  /** Bytes on disk so far for an unfinished download. */
  downloaded?: number;
  speedBps?: number;
  needsLogin?: boolean;
};

export type PlaylistPhase = 'reading' | 'ready' | 'downloading' | 'waiting' | 'paused' | 'failed' | 'done' | 'cancelled';

export type PlaylistAggregate = {
  phase: PlaylistPhase;
  total: number;
  done: number;
  failed: number;
  cancelled: number;
  active: number;
  queued: number;
  paused: number;
  ready: number;
  reading: number;
  /** Items that still count towards "N of M": everything but cancelled. */
  countable: number;
  /** 0-1, item based so it never runs backwards when a size arrives. */
  fraction: number;
  savedBytes: number;
  /** Null unless every counted download has a known size. */
  totalBytes: number | null;
  speedBps: number;
  /** Null unless sizes are all known and something is moving. */
  etaSecs: number | null;
  needsYou: boolean;
};

const MAX_ETA_SECS = 99 * 3600;

type Counter = 'done' | 'failed' | 'cancelled' | 'paused' | 'ready' | 'reading' | 'active' | 'queued';
const COUNTER: Record<string, Counter> = {
  done: 'done', error: 'failed', cancelled: 'cancelled', paused: 'paused', ready: 'ready', playlist: 'ready',
  reading: 'reading', downloading: 'active', queued: 'queued',
};

/** One pass over the children: counts, item-based progress, bytes, speed and ETA. Pure, so the table test owns it. */
export function playlistAggregate(children: PlaylistChildFacts[]): PlaylistAggregate {
  const a: PlaylistAggregate = {
    phase: 'done', total: children.length, done: 0, failed: 0, cancelled: 0, active: 0, queued: 0, paused: 0,
    ready: 0, reading: 0, countable: 0, fraction: 0, savedBytes: 0, totalBytes: 0, speedBps: 0, etaSecs: null, needsYou: false,
  };
  let partial = 0;
  let sizesKnown = true;
  for (const child of children) {
    if (child.needsLogin) a.needsYou = true;
    const size = child.size && child.size > 0 ? child.size : 0;
    const key = COUNTER[child.phase === 'downloading' && child.queued ? 'queued' : child.phase];
    if (key) a[key]++;
    if (child.phase === 'done') a.savedBytes += size;
    if (child.phase === 'cancelled' || child.phase === 'error') continue;
    if (child.phase === 'downloading' || child.phase === 'paused') {
      const pct = Math.min(100, Math.max(0, child.percent ?? 0));
      partial += pct / 100;
      a.savedBytes += child.downloaded ?? (size * pct) / 100;
      if (child.phase === 'downloading' && !child.queued) a.speedBps += Math.max(0, child.speedBps ?? 0);
    }
    if (size) a.totalBytes = (a.totalBytes ?? 0) + size;
    else if (child.phase !== 'done') sizesKnown = false;
  }
  a.countable = a.total - a.cancelled;
  const progressing = a.countable - a.failed;
  a.fraction = progressing > 0 ? Math.min(1, (a.done + partial) / progressing) : a.done > 0 ? 1 : 0;
  if (!sizesKnown) a.totalBytes = null;
  if (a.totalBytes !== null && a.speedBps > 0 && a.active > 0) {
    const eta = Math.max(0, a.totalBytes - a.savedBytes) / a.speedBps;
    a.etaSecs = eta <= MAX_ETA_SECS ? Math.round(eta) : null;
  }
  if (a.ready > 0) a.needsYou = true;
  a.phase = phaseOfAggregate(a);
  return a;
}

function phaseOfAggregate(a: PlaylistAggregate): PlaylistPhase {
  const moving = a.active + a.queued;
  if (a.reading > 0 && moving + a.paused + a.done === 0) return 'reading';
  if (a.needsYou && moving === 0) return 'ready';
  if (a.active > 0) return 'downloading';
  if (a.queued > 0) return 'waiting';
  if (a.paused > 0) return 'paused';
  if (a.reading > 0) return 'reading';
  if (a.failed > 0) return 'failed';
  if (a.cancelled > 0) return 'cancelled';
  return 'done';
}
