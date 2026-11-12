import { describe, expect, it } from 'vitest';
import { mergeParsedUrlImports, parseUrlFileText, parseUrlInputText } from '../../src/helpers/urlImport.ts';
import { normalizePastedUrl } from '../../src/helpers/url.ts';

describe('urlImport', () => {
  it('preserves unchanged URLs byte for byte, including signed query values', () => {
    const url = 'https://cdn.example.com/file?signature=a\'b%2fc+%20&next=%2Fpath';
    expect(normalizePastedUrl(url)).toBe(url);
  });

  it('normalizes pasted hosts and boundary direction marks without losing share keys', () => {
    expect(normalizePastedUrl('\u200b\u2060\ufeffhttps://youtu.be/abc?si=x\ufeff\u2060\u200b')).toBe('https://youtu.be/abc?si=x');
    expect(parseUrlInputText(
      '\u200f\u2067youtube.com/watch?v=YsKxkP_kGbE&t=10s&si=abc\u2069\u200f '
      + 'HTTPS://M.YOUTUBE.COM/watch?v=YsKxkP_kGbE '
      + 'https://fxtwitter.com/ANI/status/1823253560127041999/photo/1 '
      + 'dropbox.com/scl/fi/abc/report.pdf?rlkey=secret&dl=0',
    )).toEqual({ urls: [
      'https://youtube.com/watch?v=YsKxkP_kGbE&t=10s&si=abc',
      'HTTPS://M.YOUTUBE.COM/watch?v=YsKxkP_kGbE',
      'https://x.com/ANI/status/1823253560127041999/photo/1',
      'https://dropbox.com/scl/fi/abc/report.pdf?rlkey=secret&dl=0',
    ], skipped: 0 });
    expect(parseUrlInputText('hello youtube.com.evil.test/watch?v=abc')).toEqual({ urls: [], skipped: 2 });
  });

  it('parses multiple input urls separated by commas, whitespace, and newlines', () => {
    const result = parseUrlInputText(
      'https://example.com/a, https://example.com/b\nhttps://example.com/c\tinvalid',
    );

    expect(result.urls).toEqual([
      'https://example.com/a',
      'https://example.com/b',
      'https://example.com/c',
    ]);
    expect(result.skipped).toBe(1);
  });

  it('recognizes a scheme only at the start, preserving nested URL query values', () => {
    expect(parseUrlInputText('youtu.be/YsKxkP_kGbE?next=https://example.com/path')).toEqual({ urls: ['https://youtu.be/YsKxkP_kGbE?next=https://example.com/path'], skipped: 0 });
  });

  it('requires public bare hosts and recognized file-share paths', () => {
    expect(parseUrlInputText('evil.test@youtube.com/x youtube.com:8080/x github.com/o/r pixeldrain.com/')).toEqual({ urls: [], skipped: 4 });
    expect(parseUrlInputText('www.github.com/octocat/Hello-World/blob/master/README')).toEqual({ urls: ['https://www.github.com/octocat/Hello-World/blob/master/README'], skipped: 0 });
  });

  it('extracts urls from csv-like file content', () => {
    const result = parseUrlFileText(
      'title,url\nExample,https://example.com/watch?v=1\nTwo,https://example.com/watch?v=2',
    );

    expect(result.urls).toEqual([
      'https://example.com/watch?v=1',
      'https://example.com/watch?v=2',
    ]);
  });

  it('deduplicates merged imports', () => {
    const result = mergeParsedUrlImports([
      { urls: ['https://example.com/a', 'https://example.com/b'], skipped: 1 },
      { urls: ['https://example.com/b', 'https://example.com/c'], skipped: 2 },
    ]);

    expect(result.urls).toEqual([
      'https://example.com/a',
      'https://example.com/b',
      'https://example.com/c',
    ]);
    expect(result.skipped).toBe(3);
  });
});
