import { availableLocales, offeredLocales } from '../i18n';
import { languageOptionsLookup, type SubtitleLanguageOption } from './subtitles/languages';
import type { SelectOption } from './forms';

/** App languages, each labelled in its own script ("العربية", "Français") with the English name after. */
export function appLanguageOptions(current?: string): SelectOption[] {
  const options: SelectOption[] = [];
  // A saved language from before the list was trimmed stays visible until the user picks another.
  const shown = new Set<string>(offeredLocales);
  if (current && current !== 'system' && current in availableLocales) shown.add(current);
  for (const locale of shown) {
    const language: SubtitleLanguageOption | undefined = languageOptionsLookup.get(locale);
    if (!language) continue;
    const label = language.nativeName && language.nativeName !== language.englishName
      ? `${language.nativeName} · ${language.englishName}`
      : language.englishName;
    options.push({ label, value: language.code });
  }
  return options.sort((a, b) => String(a.value).localeCompare(String(b.value)));
}
