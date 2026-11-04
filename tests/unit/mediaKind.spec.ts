import { describe, expect, it } from 'vitest';
import { mediaKind, savedAttachments, canOpenFile } from '../../src/helpers/mediaKind';
import { TrackType, type MediaFormat } from '../../src/tauri/types/media';

const fmt = (video: boolean, audio = true): MediaFormat => ({
  id: Math.random().toString(36),
  audioCodecs: audio ? ['mp4a'] : [],
  videoCodecs: video ? ['avc1'] : [],
});

describe('mediaKind', () => {
  it('uses an explicit audio source kind before formats arrive', () => {
    expect(mediaKind([], undefined, 'audio')).toBe('audio');
  });

  it.each([
    [undefined, 'video'],
    [{ trackType: TrackType.audio }, 'audio'],
  ] as const)('keeps direct videos with unknown codecs unless audio is selected: %o', (options, expected) => {
    expect(mediaKind([fmt(false)], options, 'video')).toBe(expected);
  });

  it.each(['image', 'carousel', 'mixed', 'file'] as const)('keeps %s sources distinct from audio/video format choices', (kind) => {
    expect(mediaKind([], { trackType: TrackType.audio }, kind)).toBe(kind);
  });

  it.each([
    ['not read yet', [], undefined, 'video'],
    ['video source, default choice', [fmt(true), fmt(false)], undefined, 'video'],
    ['video source, audio-only choice', [fmt(true)], { trackType: TrackType.audio }, 'audio'],
    ['audio-only source (SoundCloud)', [fmt(false), fmt(false)], undefined, 'audio'],
    ['video-only track choice stays video', [fmt(true, false)], { trackType: TrackType.video }, 'video'],
    ['codec not reported but has a height', [{ ...fmt(false, false), height: 1080 }], undefined, 'video'],
  ] as const)('%s', (_label, formats, options, expected) => {
    expect(mediaKind([...formats], options)).toBe(expected);
  });
});

describe('savedAttachments', () => {
  const photo = (id: string) => ({ id, kind: 'image' as const, url: `https://cdn.example/${id}.jpg` });
  const clip = { id: 'v', kind: 'video' as const, url: 'https://cdn.example/v.mp4' };
  it.each([
    ['video', [photo('a')], ['a'], undefined],
    ['image', [photo('a')], ['a'], { key: 'photos', n: 1 }],
    ['carousel', [photo('a'), photo('b'), photo('c')], ['a', 'c'], { key: 'photos', n: 2 }],
    ['mixed', [photo('a'), clip], ['a', 'v'], { key: 'items', n: 2 }],
  ] as const)('%s post → %o', (kind, attachments, selected, expected) => {
    expect(savedAttachments(kind, [...attachments], [...selected])).toEqual(expected);
  });
});

describe('downloaded file open policy', () => {
  it.each([
    ['C:\\Downloads\\report.PDF', true], ['archive.zip', false], ['installer.exe', false],
    ['disk.iso', false], ['disk.dmg', false], ['script.command', false],
    ['shortcut.lnk', false], ['macro.docm', false], ['unknown', false], ['photo.png', true],
  ])('%s can launch a viewer: %s', (path, expected) => {
    expect(canOpenFile(path as string)).toBe(expected);
  });
});
