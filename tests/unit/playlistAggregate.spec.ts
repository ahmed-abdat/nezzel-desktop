import { describe, expect, it } from 'vitest';
import { playlistAggregate, type PlaylistChildFacts } from '../../src/helpers/playlistAggregate';

const done = (size = 100): PlaylistChildFacts => ({ phase: 'done', size });
const running = (percent: number, size = 100, speedBps = 10): PlaylistChildFacts => ({ phase: 'downloading', percent, size, speedBps });
const queued = (size = 100): PlaylistChildFacts => ({ phase: 'downloading', queued: true, size });

describe('playlistAggregate', () => {
  it.each<[string, PlaylistChildFacts[], Partial<ReturnType<typeof playlistAggregate>>]>([
    ['downloading: items-based progress, bytes, speed, ETA', [done(), running(50), queued()],
      { phase: 'downloading', done: 1, active: 1, queued: 1, fraction: 0.5, savedBytes: 150, totalBytes: 300, speedBps: 10, etaSecs: 15 }],
    ['waiting: nothing moving yet', [queued(), queued()], { phase: 'waiting', fraction: 0, speedBps: 0, etaSecs: null }],
    ['paused beats done when nothing runs', [done(), { phase: 'paused', percent: 40, size: 100 }], { phase: 'paused', fraction: 0.7 }],
    ['running and paused mixed: downloading', [running(10), { phase: 'paused', percent: 0 }], { phase: 'downloading' }],
    ['partial failure while running stays downloading', [running(10), { phase: 'error' }], { phase: 'downloading', failed: 1 }],
    ['all terminal with a failure: failed', [done(), { phase: 'error' }], { phase: 'failed', fraction: 1 }],
    ['all done', [done(), done()], { phase: 'done', fraction: 1, savedBytes: 200 }],
    ['cancelled children leave the count', [done(), { phase: 'cancelled' }], { phase: 'cancelled', countable: 1 }],
    ['ready children need the user', [{ phase: 'ready' }, { phase: 'ready' }], { phase: 'ready', needsYou: true }],
    ['login prompt while others wait: needs you', [queued(), { phase: 'reading', needsLogin: true }], { phase: 'waiting', needsYou: true }],
    ['still reading', [{ phase: 'reading' }], { phase: 'reading' }],
  ])('%s', (_name, children, expected) => {
    expect(playlistAggregate(children)).toMatchObject(expected);
  });

  it('unknown sizes: no total and no ETA, saved bytes still add up', () => {
    const a = playlistAggregate([done(100), running(50, 0, 10), queued(0)]);
    expect(a.totalBytes).toBeNull();
    expect(a.etaSecs).toBeNull();
    expect(a.savedBytes).toBe(100);
  });

  it('a size arriving never moves the bar backwards (item based)', () => {
    const before = playlistAggregate([done(0), running(50, 0)]).fraction;
    const after = playlistAggregate([done(0), running(50, 5000)]).fraction;
    expect(after).toBe(before);
  });

  it('ETA hidden above 99 hours and when speed is 0', () => {
    expect(playlistAggregate([running(0, 1e12, 1)]).etaSecs).toBeNull();
    expect(playlistAggregate([running(10, 100, 0)]).etaSecs).toBeNull();
  });
});
