import { describe, expect, it } from 'vitest';
import { appendLogLine } from '../../src/helpers/groupLog';

describe('appendLogLine', () => {
  it.each([
    ['', 'main', 'main'],
    ['main', 'WARNING', 'main\nWARNING'],
    ['main\n', 'WARNING', 'main\nWARNING'],
    ['Opus', '[youtube] next\n', 'Opus\n[youtube] next\n'],
    ['first\r\n', 'second', 'first\r\nsecond'],
    ['first', '', 'first\n'],
  ])('joins %j and %j', (current, line, expected) => {
    expect(appendLogLine(current, line)).toBe(expected);
  });
});
