import { afterEach, describe, expect, it, vi } from 'vitest';
import { applyDocumentLocale, getDefaultLocale, type Locale } from '../../src/i18n';
import { i18n } from '../../src/i18n';
import { mount } from '@vue/test-utils';
import { createPinia } from 'pinia';
import { defineComponent, h, ref } from 'vue';
import { useDiagnostic } from '../../src/composables/useDiagnostic';
import en from '../../src/locales/en.json';
import ar from '../../src/locales/ar.json';
import fr from '../../src/locales/fr.json';

type RunnerCopy = { shortMessage?: string; action?: string; message?: string };
const runnerLocales = { en, ar, fr } as unknown as Record<string, { errors: { runner: Record<string, RunnerCopy> } }>;
/** Tool and engine names mean nothing to people; error copy names what happened and what to do. */
const ENGINE_WORDS = /yt-dlp|gallery-dl|ffmpeg|aria2|deno|\bengine\b|moteur|محرك/i;

describe('download error copy', () => {
  const cases = Object.entries(runnerLocales).flatMap(([locale, messages]) =>
    Object.entries(messages.errors.runner).map(([code, copy]) => [locale, code, copy] as const));

  it.each(cases)('%s %s says what happened and what to do, without engine names', (_locale, _code, copy) => {
    expect(copy.shortMessage?.trim()).toBeTruthy();
    expect(copy.action?.trim()).toBeTruthy();
    expect(copy.action).not.toBe(copy.shortMessage);
    for (const text of [copy.shortMessage, copy.action, copy.message]) expect(text ?? '').not.toMatch(ENGINE_WORDS);
  });

  it('provides every new failure and update-notice key in all shipped locales', () => {
    const locales = import.meta.glob('../../src/locales/*.json', { eager: true, import: 'default' }) as Record<string, {
      errors: { runner: Record<string, RunnerCopy> }; toolsUpdate: { stale: string; tryNow: string; failed: string };
    }>;
    for (const [locale, messages] of Object.entries(locales)) {
      for (const code of ['facebookVideoUnavailable', 'accountAccessDenied', 'browserCookiesUnreadable', 'browserKeychainDenied', 'browserSafariAccessDenied', 'browserDatabaseLocked', 'browserProfileMissing', 'diskFull', 'outputNameTooLong', 'networkTimeout']) {
        for (const key of ['shortMessage', 'message', 'action'] as const) expect(messages.errors.runner[code]?.[key]?.trim(), `${locale}: ${code}.${key}`).toBeTruthy();
      }
      for (const key of ['stale', 'tryNow', 'failed'] as const) expect(messages.toolsUpdate?.[key]?.trim(), `${locale}: toolsUpdate.${key}`).toBeTruthy();
    }
  });

  it('has the same error codes in every locale', () => {
    const codes = Object.keys(en.errors.runner).sort();
    for (const messages of Object.values(runnerLocales)) expect(Object.keys(messages.errors.runner).sort()).toEqual(codes);
  });
});

describe('locale direction', () => {
  it('falls back to English for a new diagnostic key in an older locale', () => {
    i18n.global.locale.value = 'es';
    const component = defineComponent({
      setup() {
        const { diagnosticDisplay } = useDiagnostic(ref({
          id: 'image', groupId: 'images', level: 'error', code: 'imageEngineUnavailable',
          message: '', raw: '', component: 'gallery-dl', timestamp: 1,
        }));
        return () => h('p', diagnosticDisplay.value.message);
      },
    });
    const wrapper = mount(component, { global: { plugins: [i18n, createPinia()] } });
    try {
      expect(wrapper.text()).toBe(en.errors.runner.imageEngineUnavailable.message);
    } finally {
      wrapper.unmount();
      i18n.global.locale.value = 'en';
    }
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it.each([
    [['ar-MR'], 'ar'],
    [['ar'], 'ar'],
    [['fr-FR', 'en'], 'fr'],
    [['xx'], 'en'],
  ])('picks %j -> %s from the system languages', (languages, expected) => {
    vi.stubGlobal('navigator', { languages, language: languages[0] });
    expect(getDefaultLocale()).toBe(expected);
  });

  it.each([
    ['ar', 'rtl'],
    ['en', 'ltr'],
    ['fr', 'ltr'],
  ] as [Locale, string][])('sets lang and dir on <html> for %s', (locale, dir) => {
    applyDocumentLocale(locale);
    expect(document.documentElement.getAttribute('lang')).toBe(locale);
    expect(document.documentElement.getAttribute('dir')).toBe(dir);
  });
});

describe('Arabic playlist selection counts', () => {
  it.each([
    [0, 'لم يُحدَّد أي عنصر'],
    [1, 'عنصر واحد من 120'],
    [2, 'عنصران من 120'],
    [3, '3 من 120 محدّدة'],
    [11, '11 من 120 محدّدًا'],
    [100, '100 من 120 محدّد'],
  ])('renders the Arabic plural form for %s', (count, expected) => {
    const previous = i18n.global.locale.value;
    i18n.global.locale.value = 'ar';
    try {
      expect(i18n.global.t('inputFilters.playlistSelection.checkedCount', { count, total: 120 }, count)).toBe(expected);
    } finally { i18n.global.locale.value = previous; }
  });
  it.each([
    [0, 'تنزيل 0 فيديو', 'تنزيل كل العناصر (0)'],
    [1, 'تنزيل فيديو واحد', 'تنزيل العنصر (1)'],
    [2, 'تنزيل فيديوهين', 'تنزيل العنصرين (2)'],
    [3, 'تنزيل 3 فيديوهات', 'تنزيل كل العناصر (3)'],
    [11, 'تنزيل 11 فيديو', 'تنزيل كل العناصر (11)'],
    [100, 'تنزيل 100 فيديو', 'تنزيل كل العناصر (100)'],
  ])('counts the Arabic download actions for %s', (count, selected, all) => {
    const previous = i18n.global.locale.value;
    i18n.global.locale.value = 'ar';
    try {
      expect(i18n.global.t('inputFilters.playlistSelection.applySelection', { count }, count)).toBe(selected);
      expect(i18n.global.t('inputFilters.playlistSelection.applyFull', { count }, count)).toBe(all);
    } finally { i18n.global.locale.value = previous; }
  });
});
