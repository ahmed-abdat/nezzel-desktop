import { expect, it } from 'vitest';
import { i18n, type Locale } from '../../src/i18n';
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

it.each([
  ['en', 'Personal', 'Personal'], ['ar', 'Personal', 'شخصي'], ['fr', 'Personal', 'Personnel'],
  ['en', 'Work', 'Work'], ['ar', 'Work', 'عمل'], ['fr', 'Work', 'Travail'],
  ['en', 'Banking', 'Banking'], ['ar', 'Banking', 'معاملات مصرفية'], ['fr', 'Banking', 'Banque'],
  ['en', 'Shopping', 'Shopping'], ['ar', 'Shopping', 'تسوّق'], ['fr', 'Shopping', 'Achats'],
])('localizes the built-in %s Firefox container %s', (locale, container, label) => {
  const previous = i18n.global.locale.value;
  i18n.global.locale.value = locale as Locale;
  try {
    expect(candidateLabel({ browser: 'firefox', profile: `main · userContext${container}.label` })).toBe(`Firefox (main · ${label})`);
  } finally { i18n.global.locale.value = previous; }
});
