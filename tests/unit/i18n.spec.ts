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
      for (const code of ['accountAccessDenied', 'browserCookiesUnreadable', 'browserKeychainDenied', 'browserSafariAccessDenied', 'browserDatabaseLocked', 'browserProfileMissing', 'diskFull', 'outputNameTooLong', 'networkTimeout']) {
        for (const key of ['shortMessage', 'message', 'action'] as const) expect(messages.errors.runner[code]?.[key]?.trim(), `${locale}: ${code}.${key}`).toBeTruthy();
      }
      for (const key of ['stale', 'tryNow', 'failed'] as const) expect(messages.toolsUpdate?.[key]?.trim(), `${locale}: toolsUpdate.${key}`).toBeTruthy();
    }
  });

  it.each(Object.entries(runnerLocales).flatMap(([locale, messages]) =>
    ['browserKeychainDenied', 'browserDpapiFailed', 'browserAppBound', 'browserSafariAccessDenied'].map(code => [locale, code, messages] as const),
  ))('%s displays the localized remedy for %s', (locale, code, messages) => {
    i18n.global.locale.value = locale as Locale;
    const component = defineComponent({
      setup() {
        const { diagnosticDisplay } = useDiagnostic(ref({ id: 'item', groupId: 'group', level: 'error', code, component: null, message: 'untrusted cookie failure', raw: '', timestamp: 1 }));
        return () => h('p', diagnosticDisplay.value.action);
      },
    });
    const wrapper = mount(component, { global: { plugins: [i18n, createPinia()] } });
    try {
      const action = messages.errors.runner[code]?.action;
      expect(action).toBeTruthy();
      expect(wrapper.text()).toBe(action);
    } finally {
      wrapper.unmount();
      i18n.global.locale.value = 'en';
    }
  });

  it.each([
    ['en', 'Nothing was found at this link'],
    ['ar', 'لم يُعثر على محتوى في هذا الرابط'],
    ['fr', 'Aucun contenu trouvé à ce lien'],
  ] as const)('%s shows a neutral message for a plain HTTP 404', (locale, expected) => {
    i18n.global.locale.value = locale;
    const wrapper = mount(defineComponent({
      setup() {
        const { diagnosticDisplay } = useDiagnostic(ref({
          id: 'page', groupId: 'page', level: 'error', code: 'notFound404',
          message: 'HTTP Error 404: Not Found', raw: '', component: null, timestamp: 1,
        }));
        return () => h('p', diagnosticDisplay.value.shortMessage);
      },
    }), { global: { plugins: [i18n, createPinia()] } });
    try {
      expect(wrapper.text()).toBe(expected);
    } finally {
      wrapper.unmount();
      i18n.global.locale.value = 'en';
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

describe('batch report title counts', () => {
  it.each([
    ['ar', 0, 'الإبلاغ عن التنزيلات الفاشلة (0)'],
    ['ar', 1, 'الإبلاغ عن التنزيل الفاشل (1)'],
    ['ar', 2, 'الإبلاغ عن التنزيلين الفاشلين (2)'],
    ['ar', 3, 'الإبلاغ عن 3 تنزيلات فاشلة'],
    ['ar', 11, 'الإبلاغ عن 11 تنزيلًا فاشلًا'],
    ['ar', 100, 'الإبلاغ عن 100 تنزيل فاشل'],
    ['en', 1, 'Report 1 failed download'],
    ['en', 3, 'Report 3 failed downloads'],
    ['fr', 1, 'Signaler 1 téléchargement échoué'],
    ['fr', 3, 'Signaler 3 téléchargements échoués'],
  ] as const)('renders %s count %s', (locale, n, expected) => {
    const previous = i18n.global.locale.value;
    i18n.global.locale.value = locale;
    try {
      expect(i18n.global.t('report.batchTitle', { n: String(n) }, n)).toBe(expected);
    } finally { i18n.global.locale.value = previous; }
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
