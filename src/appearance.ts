import { applyDocumentLocale, getDefaultLocale, i18n, loadLocaleMessages, type Locale } from './i18n';

/** The `language` setting is a locale code or 'system' (the OS language). */
export function resolveLocale(language: string): Locale {
  return language === 'system' ? getDefaultLocale() : language as Locale;
}

export function applyTheme(theme: 'light' | 'dark') {
  if (typeof document === 'undefined') return;
  document.body.setAttribute('data-theme', theme);
  const meta = document.querySelector('meta[name="color-scheme"]');
  if (meta) meta.setAttribute('content', theme);
}

export function removeTheme() {
  document.body.removeAttribute('data-theme');
  const meta = document.querySelector('meta[name="color-scheme"]');
  if (meta) meta.removeAttribute('content');
}

let languageRequest = 0;

/** Language only: i18n and the document's lang/dir. */
export async function applyLanguage(language: string): Promise<Locale> {
  const request = ++languageRequest;
  const locale = resolveLocale(language);
  await loadLocaleMessages(locale);
  // A slower import must not undo a newer choice or a failed-save rollback.
  if (request !== languageRequest) return locale;
  i18n.global.locale.value = locale;
  applyDocumentLocale(locale);
  return locale;
}

/** The one window bootstrap: language and theme from the Appearance settings. 'system' leaves the OS to decide. */
export async function applyAppearance(appearance: { language: string; theme: string }) {
  const language = applyLanguage(appearance.language);
  if (appearance.theme === 'light' || appearance.theme === 'dark') applyTheme(appearance.theme);
  else removeTheme();
  await language;
}
