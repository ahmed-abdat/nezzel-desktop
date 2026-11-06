import { describe, expect, it } from 'vitest';
import { capturePicks, cardRows, optionsForCaptureChoice, pickChoice } from '../../src/helpers/captureChoices';
import { emptyDetails, type CaptureDetails, type CardChoice } from '../../src/captureTypes';
import type { CaptureChoice } from '../../src/generated/contract';
import { MediaFormat, TrackType } from '../../src/tauri/types/media';

// Which choice objects Rust accepts is pinned in src-tauri/src/capture/choice.rs; these test only what TS does with them.

describe('quick picks in the bar', () => {
  const d = (over: Partial<CaptureDetails>): CaptureDetails => ({ ...emptyDetails(), ...over });
  const keys = (details: CaptureDetails, read = true) => capturePicks(details, read).map(pick => pick.key);

  it('offers every pick before the read: they work right away', () => {
    expect(capturePicks(emptyDetails(), false)).toEqual([
      { key: 'best', height: null, size: null },
      { key: 'audio', height: null, size: null },
      { key: 'small', height: 720, size: null },
    ]);
  });

  it.each([
    ['video with sound and a lower quality', d({ mediaKind: 'video', bestHeight: 1080, hasAudio: true, smallHeight: 720 }), ['best', 'audio', 'small']],
    ['single quality: no Smaller', d({ mediaKind: 'video', bestHeight: 720, hasAudio: true }), ['best', 'audio']],
    ['silent video: no Audio', d({ mediaKind: 'video', bestHeight: 1080, smallHeight: 480 }), ['best', 'small']],
    ['audio-only link: no video picks', d({ mediaKind: 'audio', bestHeight: null, hasAudio: true, audioSize: 9 }), ['best']],
    ['photos', d({ mediaKind: 'carousel', photos: 4 }), ['best']],
    ['file', d({ mediaKind: 'file', size: 10 }), ['best']],
  ])('%s', (_name, details, expected) => {
    expect(keys(details)).toEqual(expected);
  });

  it('carries the real size of each pick after the read', () => {
    const picks = capturePicks(d({ mediaKind: 'video', bestHeight: 1080, size: 120, hasAudio: true, audioSize: 4, smallHeight: 720, smallSize: 48 }), true);
    expect(picks.map(pick => [pick.key, pick.height, pick.size])).toEqual([['best', 1080, 120], ['audio', null, 4], ['small', 720, 48]]);
  });

  it('sends a choice for Audio and Smaller; Best sends none (the user default applies)', () => {
    expect([pickChoice('best'), pickChoice('audio'), pickChoice('small')]).toEqual([undefined, { kind: 'audio' }, { kind: 'small' }]);
  });
});

describe('cardRows: the copied-link card format menu', () => {
  const video = (height: number, size: number | null = null, fps?: number): CardChoice => ({ choice: { kind: 'video', height, ...(fps ? { fps } : {}) }, size });
  const audio = (kbps: number | null, size: number | null = null): CardChoice => ({ choice: kbps ? { kind: 'audio', kbps } : { kind: 'audio' }, size });
  const details = (patch: Partial<CaptureDetails>): CaptureDetails => ({ ...emptyDetails(), mediaKind: 'video', bestHeight: 1080, hasAudio: true, ...patch });
  const keys = (patch: Partial<CaptureDetails>, read = true) => cardRows(details(patch), read).map(row => row.key);

  it('keeps the rows Rust sent, in its order, and marks the first video as what Download best gets', () => {
    const rows = cardRows(details({ choices: [video(1080, 500, 60), video(1080, 300), video(720, 70), audio(160, 16), audio(48, 5)] }), true);
    expect(rows.map(row => row.key)).toEqual(['video:1080:60', 'video:1080', 'video:720', 'audio:160', 'audio:48']);
    expect(rows.map(row => row.size)).toEqual([500, 300, 70, 16, 5]);
    expect(rows.filter(row => row.best).map(row => row.key)).toEqual(['video:1080:60']);
    // A row sends exactly the choice Rust offered.
    expect(rows[0].choice).toEqual({ kind: 'video', height: 1080, fps: 60 });
    expect(rows[3].choice).toEqual({ kind: 'audio', kbps: 160 });
  });

  it('offers generic ceilings before the read and for playlists, but no menu for photos, files or one format', () => {
    expect(keys({}, false)).toEqual(['best', 'video:1080', 'video:720', 'video:480', 'audio']);
    expect(keys({ mediaKind: null, bestHeight: null, hasAudio: false, choices: [] })).toEqual(['best', 'video:1080', 'video:720', 'video:480', 'audio']);
    expect(keys({ mediaKind: 'carousel' }, false)).toEqual([]);
    expect(keys({ mediaKind: 'file', choices: [video(1080), audio(128)] })).toEqual([]);
    expect(keys({ choices: [video(720)] })).toEqual([]);
    expect(keys({ choices: [] })).toEqual([]); // read with a known best but no list: plain Download
    expect(keys({ choices: [video(720), audio(null)] })).toEqual(['video:720', 'audio']);
  });

  it('marks generic heights as ceilings without sizes; Download best sends best', () => {
    const rows = cardRows(details({}), false);
    expect(rows.filter(row => row.upTo).map(row => row.choice)).toEqual([1080, 720, 480].map(height => ({ kind: 'video', height })));
    expect(rows.every(row => row.size === null)).toBe(true);
    expect(rows[0].choice).toEqual({ kind: 'best' });
  });
});

describe('capture choices map to engine options', () => {
  const f = (id: string, height?: number, fps?: number, abr?: number): MediaFormat => ({ id, height, fps, abr, audioCodecs: [], videoCodecs: [] });
  const formats = [f('a1', undefined, undefined, 128), f('v4', 2160, 60), f('v1', 1080, 30), f('v7', 720, 30), f('v3', 360, 30)];
  const both = [...formats, f('v6', 1080, 60)];
  it.each([
    ['1080', { kind: 'video', height: 1080 }, both, { trackType: TrackType.both, id: 'v1', height: 1080, fps: 30 }], // ordinary row skips the 60 fps file
    ['1080 60 fps', { kind: 'video', height: 1080, fps: 60 }, both, { trackType: TrackType.both, id: 'v6', height: 1080, fps: 60 }],
    ['2160', { kind: 'video', height: 2160 }, formats, { trackType: TrackType.both, id: 'v4', height: 2160 }], // only a 60 fps file at that height
    ['720 in a playlist', { kind: 'video', height: 720 }, [], { trackType: TrackType.both, id: 'best', height: 720 }], // a ceiling for each item
    ['720 60 fps in a playlist', { kind: 'video', height: 720, fps: 60 }, [], { trackType: TrackType.both, height: 720, fps: 60 }],
    ['129 kbps', { kind: 'audio', kbps: 129 }, formats, { trackType: TrackType.audio, id: 'best', abr: 129 }],
    ['audio', { kind: 'audio' }, formats, { trackType: TrackType.audio, id: 'best' }],
    ['small', { kind: 'small' }, formats, { trackType: TrackType.both, height: 720 }],
    ['best', { kind: 'best' }, formats, { trackType: TrackType.both, height: 2160 }],
  ] as [string, CaptureChoice, MediaFormat[], object][])('%s', (_name, choice, list, expected) => {
    expect(optionsForCaptureChoice(choice, [...list], 'video')).toMatchObject(expected);
  });
});
