import { describe, expect, it } from 'vitest';
import { capturePicks } from '../../src/helpers/capturePicks';
import { emptyDetails, type CaptureDetails } from '../../src/captureTypes';

const d = (over: Partial<CaptureDetails>): CaptureDetails => ({ ...emptyDetails(), ...over });
const keys = (details: CaptureDetails, read = true) => capturePicks(details, read).map(pick => pick.key);

describe('quick picks in the bar and card', () => {
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
});
