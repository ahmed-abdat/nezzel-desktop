import { describe, expect, it } from 'vitest';
import { hostOf, siteName } from '../../src/helpers/siteName';
import { siteForHost, siteLogo } from '../../src/helpers/siteLogo';

describe('local site logos', () => {
  it.each([
    ['vt.tiktok.com', 'tiktok'], ['pin.it', 'pinterest'], ['fb.watch', 'facebook'],
    ['redd.it', 'reddit'], ['v.redd.it', 'reddit'], ['youtu.be', 'youtube'], ['dai.ly', 'dailymotion'],
    ['on.soundcloud.com', 'soundcloud'], ['t.snapchat.com', 'snapchat'], ['lnkd.in', 'linkedin'],
    ['www.instagram.com', 'instagram'], ['twitter.com', 'x'], ['threads.com', 'threads'],
    ['vimeo.com', 'vimeo'], ['clips.twitch.tv', 'twitch'], ['example.tumblr.com', 'tumblr'],
  ])('%s resolves immediately to %s', (host, key) => {
    expect(siteForHost(host)?.key).toBe(key);
    expect(siteLogo(`https://${host}/clip`)).toBeTruthy();
  });

  it('normalizes DNS case and trailing dot, rejecting lookalikes and invalid URLs', () => {
    expect(siteForHost('VT.TIKTOK.COM.')?.key).toBe('tiktok');
    for (const host of ['notyoutube.com', 'youtube.com.evil.net', 'pin.it.evil.net', 'example.org']) {
      expect(siteForHost(host)).toBeUndefined();
      expect(siteLogo(`https://${host}/youtube.com`)).toBeUndefined();
    }
    expect(siteLogo('not a URL')).toBeUndefined();
    expect(siteLogo('file://youtube.com/video')).toBeUndefined();
  });
});

describe('siteName', () => {
  it.each([
    ['youtube:tab', 'https://www.youtube.com/watch?v=1', 'YouTube'],
    ['YoutubeClip', 'https://www.youtube.com/clip/Ugkx123', 'YouTube'],
    ['YoutubeClip', undefined, 'YouTube'],
    ['InstagramCarousel', 'https://www.instagram.com/p/abc', 'Instagram'],
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
