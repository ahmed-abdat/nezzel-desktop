import { expect, it } from 'vitest';
import { browserName, candidateLabel } from '../../src/helpers/cookieBrowsers';

it('preserves dots in real profile names instead of treating them as directory prefixes', () => {
  expect(candidateLabel({ browser: 'firefox', profile: 'Ahmed.work' })).toBe('Firefox (Ahmed.work)');
});

it('names engine browser keys and leaves Safari and profile-less candidates bare', () => {
  expect(browserName('msedge')).toBe('Msedge'); // not a UI key: only capitalised
  expect(browserName('EDGE')).toBe('Edge');
  expect(candidateLabel({ browser: 'safari', profile: 'Default' })).toBe('Safari');
  expect(candidateLabel({ browser: 'brave', profile: '' })).toBe('Brave');
});
