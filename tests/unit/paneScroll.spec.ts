import { describe, expect, it } from 'vitest';
import { paneScrollTop, type ScrollKeyOrigin } from '../../src/helpers/paneScroll';

const pane = { scrollTop: 100, clientHeight: 400, scrollHeight: 1400 };

describe('scroll keys reach the content pane', () => {
  it.each([
    ['PageDown', false, 'page', 450],
    ['PageDown', false, 'shell', 450],
    ['PageUp', false, 'shell', 0],
    ['End', false, 'shell', 1000],
    ['Home', false, 'page', 0],
    [' ', false, 'page', 450],
    [' ', true, 'page', 0],
    ['ArrowDown', false, 'page', 140],
    // In the shell Space activates the focused control and arrows move between controls.
    [' ', false, 'shell', null],
    ['ArrowDown', false, 'shell', null],
    // Inside the pane, in a menu or a dialog, the browser or the control already handles the key.
    ['PageDown', false, 'native', null],
    ['a', false, 'page', null],
  ] as [string, boolean, ScrollKeyOrigin, number | null][])('%j (shift %s) from %s -> %s', (key, shift, origin, expected) => {
    expect(paneScrollTop(key, shift, origin, pane)).toBe(expected);
  });

  it('leaves a pane that does not scroll alone', () => {
    expect(paneScrollTop('PageDown', false, 'page', { scrollTop: 0, clientHeight: 400, scrollHeight: 400 })).toBeNull();
  });
});
