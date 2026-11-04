import { describe, expect, it } from 'vitest';
import { siteName } from '../../src/helpers/siteName';

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
    ['generic', 'not a url', undefined],
  ])('%s %s → %s', (extractor, url, expected) => {
    expect(siteName(extractor, url)).toBe(expected);
  });
});
