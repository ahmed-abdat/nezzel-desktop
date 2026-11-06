import { describe, it, expect } from 'vitest';
import { formatBytes, formatDuration } from '../../src/helpers/units';

describe('units', () => {
  describe('formatBytes', () => {
    it('formats bytes into human readable string', () => {
      expect(formatBytes(1024)).toBe('1.0 KB');
      expect(formatBytes(1536)).toBe('1.5 KB');
      expect(formatBytes(1048576)).toBe('1.0 MB');
    });
  });

  describe('formatDuration', () => {
    it('does not pad the leading unit (the ETA and row length)', () => {
      expect(formatDuration(65)).toBe('1:05');
      expect(formatDuration(3725)).toBe('1:02:05');
      expect(formatDuration(-4)).toBe('0:00');
      expect(formatDuration(59.9)).toBe('0:59');
    });

    it('pads the leading unit with pad (tiles, inspector, chapters)', () => {
      expect(formatDuration(65, { pad: true })).toBe('01:05');
      expect(formatDuration(3725, { pad: true })).toBe('01:02:05');
      expect(formatDuration(-4, { pad: true })).toBe('00:00');
      expect(formatDuration(2463.8, { pad: true })).toBe('41:03');
    });
  });
});
