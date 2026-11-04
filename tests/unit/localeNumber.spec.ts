import { describe, expect, it } from 'vitest';
import { formatCount, formatPercent, formatSize, formatSpeed } from '../../src/helpers/localeNumber';

describe('UI numbers use Latin digits in every locale', () => {
  it.each([
    ['ar', 1234, /^1[,٬]?234$/],
    ['en', 1234, /^1,234$/],
    ['fr', 1234, /^1\s?234$/],
  ])('%s count %d', (locale, value, expected) => {
    expect(formatCount(value, locale)).toMatch(expected);
  });

  it.each([
    ['ar', 42],
    ['en', 42],
    ['fr', 42],
  ])('%s percent has no Arabic-Indic digits', (locale, value) => {
    const text = formatPercent(value, locale);
    expect(text).toMatch(/42/);
    expect(text).not.toMatch(/[٠-٩]/);
  });

  it('clamps out-of-range progress', () => {
    expect(formatPercent(140, 'en')).toBe('100%');
    expect(formatPercent(-3, 'en')).toBe('0%');
  });

  it.each([
    ['en', 412_000_000, /^412 MB$/],
    ['en', 1_900_000_000, /^1\.9 GB$/],
    ['en', 900, /^900 byte/],
    ['ar', 412_000_000, /^412\s?\S+/],
  ])('%s size %d', (locale, bytes, expected) => {
    const text = formatSize(bytes, locale);
    expect(text).toMatch(expected);
    expect(text).not.toMatch(/[٠-٩]/);
  });

  it('speed reads per second and keeps Latin digits in Arabic', () => {
    expect(formatSpeed(21_400_000, 'en')).toBe('21.4 MB/s');
    expect(formatSpeed(21_400_000, 'ar')).toMatch(/21[.٫]4/);
  });
});
