import { availableLocales } from '../i18n';
import { languageOptionsLookup, type SubtitleLanguageOption } from './subtitles/languages';
import type { SelectOption } from './forms';

/** App languages, each labelled in its own script ("العربية", "Français") with the English name after. */
export function appLanguageOptions(): SelectOption[] {
  const options: SelectOption[] = [];
  for (const locale of Object.keys(availableLocales)) {
    const language: SubtitleLanguageOption | undefined = languageOptionsLookup.get(locale);
    if (!language) continue;
    const label = language.nativeName && language.nativeName !== language.englishName
      ? `${language.nativeName} · ${language.englishName}`
      : language.englishName;
    options.push({ label, value: language.code });
  }
  return options.sort((a, b) => String(a.value).localeCompare(String(b.value)));
}
