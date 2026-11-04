import { expect, it } from 'vitest';
import { candidateLabel } from '../../src/helpers/cookieBrowsers';

it('preserves dots in real profile names instead of treating them as directory prefixes', () => {
  expect(candidateLabel({ browser: 'firefox', profile: 'Ahmed.work' })).toBe('Firefox (Ahmed.work)');
});
