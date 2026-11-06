import { applyDocumentLocale, getDefaultLocale, i18n, type Locale } from './i18n';

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

/** Language only: i18n and the document's lang/dir. */
export function applyLanguage(language: string): Locale {
  const locale = resolveLocale(language);
  i18n.global.locale.value = locale;
  applyDocumentLocale(locale);
  return locale;
}

/** The one window bootstrap: language and theme from the Appearance settings. 'system' leaves the OS to decide. */
export function applyAppearance(appearance: { language: string; theme: string }) {
  applyLanguage(appearance.language);
  if (appearance.theme === 'light' || appearance.theme === 'dark') applyTheme(appearance.theme);
  else removeTheme();
}
