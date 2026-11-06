import { describe, expect, it } from 'vitest';
import { hostOf, siteName } from '../../src/helpers/siteName';

describe('siteName', () => {
  it.each([
    ['youtube:tab', 'https://www.youtube.com/watch?v=1', 'YouTube'],
    [undefined, 'https://www.youtube.com/watch?v=1', 'YouTube'],
    [undefined, 'https://youtu.be/1', 'YouTube'],
    [undefined, 'https://m.facebook.com/watch/1', 'Facebook'],
    [undefined, 'https://x.com/a/status/1', 'X'],
    [undefined, 'https://www.instagram.com/p/1/', 'Instagram'],
    [undefined, 'https://media.example.org/v/1', 'media.example.org'],
    // Lookalikes keep their real host: the login card asks for consent with this name.
    [undefined, 'https://youtube.evil.net/v/1', 'youtube.evil.net'],
    [undefined, 'https://www.instagram.com.evil.net/p/1', 'instagram.com.evil.net'],
    [undefined, 'https://notyoutube.com/v/1', 'notyoutube.com'],
    [undefined, 'https://fb.watch/abc', 'Facebook'],
    [undefined, 'https://twitter.com/a/status/1', 'X'],
    [undefined, 'https://www.twitch.tv/x', 'Twitch'],
    // Every site in the engine's table is named by host, short links included.
    [undefined, 'https://www.linkedin.com/posts/a', 'LinkedIn'],
    [undefined, 'https://redd.it/abc', 'Reddit'],
    // A known extractor wins over the host; an unknown one is capitalised as is.
    ['twitter', 'https://example.com/x', 'X'],
    ['Vimeo:channel', undefined, 'Vimeo'],
    ['bilibili', 'https://www.bilibili.com/video/1', 'Bilibili'],
    ['generic', 'https://www.example.org/a', 'example.org'],
    ['generic', 'not a url', undefined],
    [undefined, undefined, undefined],
  ])('%s %s → %s', (extractor, url, expected) => {
    expect(siteName(extractor, url)).toBe(expected);
  });
});

describe('hostOf', () => {
  it('drops scheme, path, query and www', () => {
    expect(hostOf('https://www.dropbox.com/s/abc/file.zip?dl=1&token=secret')).toBe('dropbox.com');
    expect(hostOf('not a url')).toBe('');
  });
});
