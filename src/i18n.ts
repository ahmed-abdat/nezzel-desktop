import { createI18n } from 'vue-i18n';
import en from './locales/en.json';
import { detectBrowserLanguageCodes } from './helpers/subtitles/languages.ts';
import { RTL_LANGS } from './generated/contract';

export const availableLocales = {
  'en': true,
  'es': true,
  'nl': true,
  'it': true,
  'fr': true,
  'de': true,
  'nb': true,
  'ru': true,
  'tr': true,
  'pt-PT': true,
  'pt-BR': true,
  'zh-TW': true,
  'ar': true,
} as const;

type MessageSchema = typeof en;
export type Locale = keyof typeof availableLocales;

const rtlLocales: ReadonlySet<string> = new Set(RTL_LANGS);

export function localeDirection(locale: string): 'ltr' | 'rtl' {
  return rtlLocales.has(locale.split('-')[0]) ? 'rtl' : 'ltr';
}

/** Sets `lang` and `dir` on <html> so the whole layout mirrors for RTL locales. */
export function applyDocumentLocale(locale: Locale): void {
  document.documentElement.setAttribute('lang', locale);
  document.documentElement.setAttribute('dir', localeDirection(locale));
}

const localeAliases: Record<string, Locale> = {
  'pt': 'pt-PT',
  'pt-PT': 'pt-PT',
  'pt-BR': 'pt-BR',
  'zh': 'zh-TW',
  'zh-Hant': 'zh-TW',
  'zh-TW': 'zh-TW',
  'no': 'nb',
  'nb-NO': 'nb',
};

/** Fully translated locales: the only ones offered and auto-picked. The others remain available on demand for old configs and upstream merges. */
export const offeredLocales: ReadonlySet<Locale> = new Set<Locale>(['en', 'ar', 'fr']);

export function getDefaultLocale(): Locale {
  for (const code of detectBrowserLanguageCodes()) {
    const baseCode = code.split('-')[0];
    const match = [code in availableLocales ? code as Locale : undefined, localeAliases[code], baseCode in availableLocales ? baseCode as Locale : undefined, localeAliases[baseCode]]
      .find(locale => locale !== undefined);
    if (match && offeredLocales.has(match)) return match;
  }

  return 'en';
}

export const i18n = createI18n<[MessageSchema], Locale, false>({
  locale: 'en',
  legacy: false,
  globalInjection: false,
  fallbackLocale: 'en',
  pluralRules: {
    ar: (count, choicesLength, original) => {
      if (choicesLength !== 6) return original?.(count, choicesLength) ?? 0;
      const mod = Math.abs(count) % 100;
      return count === 0 ? 0 : count === 1 ? 1 : count === 2 ? 2 : mod >= 3 && mod <= 10 ? 3 : mod >= 11 ? 4 : 5;
    },
  },
  // Other locale schemas are installed by loadLocaleMessages before use.
  messages: { en } as Record<Locale, MessageSchema>,
});

const localeModules = import.meta.glob<{ default: MessageSchema }>(['./locales/*.json', '!./locales/en.json']);
const pendingLocales = new Map<Locale, Promise<void>>();

/** Keep English ready as the fallback; fetch each other locale only when it is needed. */
export async function loadLocaleMessages(locale: Locale): Promise<void> {
  if (i18n.global.availableLocales.includes(locale)) return;
  let pending = pendingLocales.get(locale);
  if (!pending) {
    pending = localeModules[`./locales/${locale}.json`]().then(({ default: messages }) => {
      i18n.global.setLocaleMessage(locale, messages);
    }).finally(() => pendingLocales.delete(locale));
    pendingLocales.set(locale, pending);
  }
  await pending;
}
