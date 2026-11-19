import { describe, expect, it } from 'vitest';
import { editDistance, normalizeSearch, searchSettings, type SearchDoc } from '../../src/helpers/settingsSearch';
import { buildSearchDocs, SETTINGS_INDEX, type IndexTranslator } from '../../src/components/settings/settingsIndex';
import en from '../../src/locales/en.json';
import ar from '../../src/locales/ar.json';
import fr from '../../src/locales/fr.json';

const messages: Record<string, unknown> = { en, ar, fr };
const lookup = (lang: string, key: string): string | undefined => {
  const value = key.split('.').reduce<unknown>((node, part) => (node as Record<string, unknown> | undefined)?.[part], messages[lang]);
  return typeof value === 'string' ? value : undefined;
};

/** The real registry and locale files, translated the way vue-i18n would (placeholders emptied, en fallback). */
function docsFor(locale: string, isMac = false): SearchDoc[] {
  const tr: IndexTranslator = {
    text: (key, lang = locale) => (lookup(lang, key) ?? lookup('en', key) ?? key).replace(/\{\w+\}/g, ''),
    has: (key, lang) => lookup(lang, key) !== undefined,
    locale,
    isMac,
  };
  return buildSearchDocs(SETTINGS_INDEX, tr);
}
const top = (locale: string, query: string) => searchSettings(docsFor(locale), query).map(hit => hit.id);

describe('settings search: real settings, every language', () => {
  it.each([
    // English: words, synonyms, typos.
    ['en', 'ads', 'sponsorBlock'],
    ['en', 'sponsor', 'sponsorBlock'],
    ['en', 'folder', 'saveFolder'],
    ['en', 'where downloads go', 'saveFolder'],
    ['en', 'dark', 'theme'],
    ['en', 'theme', 'theme'],
    ['en', 'tray', 'tray'],
    ['en', 'proxy', 'proxy'],
    ['en', 'notifcations', 'notifications'],
    ['en', 'languge', 'language'],
    ['en', 'subtitels', 'subtitles'],
    ['en', 'aria', 'aria2'],
    ['en', 'start at login', 'autoStart'],
    // Arabic: hamza seats, taa marbuta, alef maqsura, harakat, tatweel, the article, typos.
    ['ar', 'إعلانات', 'sponsorBlock'],
    ['ar', 'اعلانات', 'sponsorBlock'],
    ['ar', 'الإعلانات', 'sponsorBlock'],
    ['ar', 'مجلد', 'saveFolder'],
    ['ar', 'مُجَلَّد', 'saveFolder'],
    ['ar', 'مـجـلد', 'saveFolder'],
    ['ar', 'الخلفيه', 'tray'],
    ['ar', 'لغه', 'language'],
    ['ar', 'داكن', 'theme'],
    ['ar', 'بروكسى', 'proxy'],
    ['ar', 'اشعارت', 'notifications'],
    ['ar', 'كوكيز', 'autoLogins'],
    // French: accents optional, typos.
    ['fr', 'publicités', 'sponsorBlock'],
    ['fr', 'publicites', 'sponsorBlock'],
    ['fr', 'dossier', 'saveFolder'],
    ['fr', 'thème sombre', 'theme'],
    ['fr', 'langue', 'language'],
    ['fr', 'sous titres', 'subtitles'],
    ['fr', 'notifcations', 'notifications'],
    // Another language than the UI: synonyms work everywhere, English names too.
    ['en', 'إعلانات', 'sponsorBlock'],
    ['en', 'dossier', 'saveFolder'],
    ['ar', 'proxy', 'proxy'],
    ['ar', 'theme', 'theme'],
    ['fr', 'مجلد', 'saveFolder'],
  ])('%s "%s" → %s first', (locale, query, expected) => {
    expect(top(locale, query)[0]).toBe(expected);
  });

  it('finds rows hidden behind "Show more", Logins advanced and Developer options', () => {
    expect(top('en', 'precise cuts')[0]).toBe('sponsorBlockPrecise');
    expect(top('en', 'cookies.txt')[0]).toBe('cookieFile');
    expect(top('en', 'extractor')[0]).toBe('extractorArgs');
    expect(top('ar', 'مطورين')).toContain('extractorArgs');
  });

  it('gives several guesses for a broad word, and nothing for noise', () => {
    expect(top('en', 'speed')).toEqual(expect.arrayContaining(['aria2', 'pieces', 'parallelDownloads']));
    expect(top('en', 'qqqzzz')).toEqual([]);
    expect(top('en', '   ')).toEqual([]);
  });

  it('ignores filler words and keeps the best guess when one word matches nothing', () => {
    expect(top('en', 'change the theme')[0]).toBe('theme');
    expect(top('fr', 'changer la langue')[0]).toBe('language');
  });

  it('only offers macOS-only rows on macOS', () => {
    expect(searchSettings(docsFor('en', false), 'codec').map(hit => hit.id)).not.toContain('keepOriginalCodec');
    expect(searchSettings(docsFor('en', true), 'codec').map(hit => hit.id)[0]).toBe('keepOriginalCodec');
  });
});

describe('settings search: ranking', () => {
  const docs: SearchDoc[] = [
    { id: 'fuzzy', label: 'Thme', synonyms: [] },
    { id: 'hint', label: 'Colours', hint: 'Pick a theme for the window', synonyms: [] },
    { id: 'synonym', label: 'Look', synonyms: ['window theme'] },
    { id: 'prefix', label: 'Themes gallery', synonyms: [] },
    { id: 'exact', label: 'Theme', synonyms: [] },
    { id: 'none', label: 'Proxy', synonyms: ['network'] },
  ];

  it('exact label > label prefix > synonym word > hint > typo', () => {
    expect(searchSettings(docs, 'theme').map(hit => hit.id)).toEqual(['exact', 'prefix', 'synonym', 'hint', 'fuzzy']);
  });

  it('a query that is exactly a curated keyword beats a label that only starts with it', () => {
    const folders: SearchDoc[] = [{ id: 'names', label: 'Folders and file names', synonyms: [] }, { id: 'save', label: 'Save to', synonyms: ['folder'] }];
    expect(searchSettings(folders, 'folder').map(hit => hit.id)).toEqual(['save', 'names']);
  });

  it('typo budget: none under 4 letters, 1 for 4-6, 2 from 7', () => {
    const words: SearchDoc[] = [{ id: 'n', label: 'notifications', synonyms: [] }, { id: 'p', label: 'pin', synonyms: [] }];
    expect(searchSettings(words, 'pn').map(hit => hit.id)).toEqual([]);
    expect(searchSettings(words, 'notificatoins').map(hit => hit.id)).toEqual(['n']);
    expect(searchSettings(words, 'notfictaoins').map(hit => hit.id)).toEqual([]);
  });
});

describe('settings search: text folding', () => {
  it.each([
    ['Thème', 'theme'],
    ['أإآٱ', 'اااا'],
    ['مُجَلَّد', 'مجلد'],
    ['مـجـلد', 'مجلد'],
    ['الخلفية', 'الخلفيه'],
    ['بروكسى', 'بروكسي'],
    ['مؤقت، جديد؟', 'موقت جديد'],
    ['cookies.txt', 'cookies txt'],
  ])('%s → %s', (input, expected) => {
    expect(normalizeSearch(input)).toBe(expected);
  });

  it('counts an adjacent swap as one edit', () => {
    expect(editDistance('teh', 'the')).toBe(1);
    expect(editDistance('folder', 'foldre')).toBe(1);
    expect(editDistance('kitten', 'sitting')).toBe(3);
    expect(editDistance('abc', 'abcdef', 1)).toBe(2);
  });
});
