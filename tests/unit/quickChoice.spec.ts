import { describe, expect, it } from 'vitest';
import { choiceForOptions, hasRealChoice, optionsForChoice } from '../../src/helpers/quickChoice';
import { MediaFormat, TrackType } from '../../src/tauri/types/media';

const f = (id: string, height?: number, fps?: number, abr?: number): MediaFormat => ({
  id, height, fps, abr, audioCodecs: [], videoCodecs: [],
});

const formats = [f('a1', undefined, undefined, 128), f('v4', 2160, 60), f('v1', 1080, 30), f('v7', 720, 30), f('v3', 360, 30)];

describe('quick choices map to engine options', () => {
  it.each([
    ['video', 'best', TrackType.both],
    ['carousel', 'best', TrackType.both],
    ['mixed', 'best', TrackType.both],
    ['video', 'audio', TrackType.audio],
    ['file', 'audio', TrackType.both],
    ['image', 'audio', TrackType.both],
  ] as const)('keeps %s tracks for the %s choice without reported qualities', (kind, choice, trackType) => {
    expect(optionsForChoice(choice, [], kind).trackType).toBe(trackType);
  });
  it.each([
    ['best', formats, { trackType: TrackType.both, height: 2160, fps: 60 }],
    ['small', formats, { trackType: TrackType.both, height: 720 }],
    ['small', [f('v4', 2160), f('v1', 1080)], { trackType: TrackType.both, height: 1080 }],
    ['best', [f('pdf')], { trackType: TrackType.both, id: 'best' }],
    ['audio', [f('missing', 720)], { trackType: TrackType.audio, id: 'best' }],
    ['audio', [{ ...f('silent', 720), audioCodecs: ['none'] } as MediaFormat], { trackType: TrackType.both, height: 720 }],
    ['audio', [{ ...f('unknown', 720), audioCodecs: ['unknown'] } as MediaFormat], { trackType: TrackType.audio, id: 'best' }],
    ['audio', formats, { trackType: TrackType.audio, id: 'best' }],
    ['best', [f('a1', undefined, undefined, 128)], { trackType: TrackType.audio, id: 'best' }],
  ] as const)('%s', (choice, list, expected) => {
    expect(optionsForChoice(choice, [...list])).toMatchObject(expected);
  });

  it('keeps the Audio label when video codecs have not arrived yet', () => {
    expect(choiceForOptions({ trackType: TrackType.audio, id: 'best' }, [], 'video')).toBe('audio');
  });

  it.each([
    [undefined, 'best'],
    [{ trackType: TrackType.both, height: 2160, fps: 60 }, 'best'],
    [{ trackType: TrackType.both, height: 720, fps: 30 }, 'small'],
    [{ trackType: TrackType.both, height: 1080, fps: 30 }, 'custom'],
    [{ trackType: TrackType.audio, id: 'best' }, 'audio'],
    [{ trackType: TrackType.audio, abr: 128 }, 'custom'],
    [{ trackType: TrackType.video, height: 2160, fps: 60 }, 'custom'],
  ] as const)('reads %j back as %s', (options, expected) => {
    expect(choiceForOptions(options, formats)).toBe(expected);
  });
});

describe('hasRealChoice', () => {
  const video = (id: string, height: number): MediaFormat => ({ ...f(id, height), videoCodecs: ['avc1'] });
  const audio = (id: string, abr: number): MediaFormat => ({ ...f(id, undefined, undefined, abr), audioCodecs: ['mp4a'] });
  const image = (id: string): MediaFormat => f(id);

  it.each<[string, Parameters<typeof hasRealChoice>[0], boolean]>([
    ['single image', { formats: [image('img')] }, false],
    ['unknown audio codec', { formats: [{ ...video('v1', 720), audioCodecs: ['unknown'] }], audioCodecs: ['unknown'] }, true],
    ['none audio codec', { formats: [{ ...video('v1', 720), audioCodecs: ['none'] }] }, false],
    ['file', { formats: [], mediaKind: 'file' }, false],
    ['unknown source, nothing read', { formats: [] }, true],
    ['image kind', { formats: [], mediaKind: 'image' }, false],
    ['one image attachment', { formats: [], mediaKind: 'carousel', attachments: [{ kind: 'image' }] }, false],
    ['two image attachments', { formats: [], mediaKind: 'carousel', attachments: [{ kind: 'image' }, { kind: 'image' }] }, true],
    ['one video attachment, no audio evidence', { formats: [], mediaKind: 'video' }, true],
    ['single-image post', { formats: [image('i1')], total: 1 }, false],
    ['3-image post', { formats: [image('i1')], total: 3 }, true],
    ['post with entries', { formats: [image('i1')], entries: [{ index: 0, videoUrl: 'u' }] }, true],
    ['video 1 height with audio', { formats: [video('v1', 720), audio('a1', 128)] }, true],
    ['video 1 height, missing audio codec', { formats: [video('v1', 720)] }, true],
    ['video explicit silence', { formats: [{ ...video('v1', 720), audioCodecs: ['none'] }] }, false],
    ['all formats explicitly silent', { formats: [{ ...video('v1', 720), audioCodecs: ['none'] }, { ...video('v2', 720), audioCodecs: [{ id: 'none', label: 'None' }] }], audioCodecs: ['unknown'] }, false],
    ['silence plus unknown', { formats: [{ ...video('v1', 720), audioCodecs: ['none'] }, { ...video('v2', 720), audioCodecs: ['unknown'] }] }, true],
    ['video 3 heights', { formats: [video('v1', 1080), video('v2', 720), video('v3', 360)] }, true],
    ['audio one format', { formats: [audio('a1', 128)] }, false],
    ['audio two formats', { formats: [audio('a1', 128), audio('a2', 64)] }, true],
    ['playlist', { formats: [], isCombined: true, total: 12 }, true],
  ])('%s', (_name, media, expected) => {
    expect(hasRealChoice(media)).toBe(expected);
  });
});
