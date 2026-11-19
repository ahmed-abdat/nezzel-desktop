import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import { SETTINGS_INDEX } from '../../src/components/settings/settingsIndex';
import en from '../../src/locales/en.json';
import ar from '../../src/locales/ar.json';
import fr from '../../src/locales/fr.json';

/**
 * Settings search is only as good as its index: every row in the Settings tabs must carry a
 * `search-id` registered in settingsIndex.ts with the same label key, and every registered id must
 * still exist on screen. Adding a row without registering it fails here.
 */
const SRC = join(__dirname, '../../src');
const TAB_FILES: [string, string][] = [
  ...readdirSync(join(SRC, 'views/app/settings')).map(name => [join(SRC, 'views/app/settings', name), name] as [string, string]),
  ...readdirSync(join(SRC, 'components/settings')).filter(name => name.endsWith('.vue')).map(name => [join(SRC, 'components/settings', name), name] as [string, string]),
];
const TAB_OF_FILE: Record<string, string> = {
  'SettingsGeneralTab.vue': 'general',
  'SettingsLoginsTab.vue': 'logins',
  'SettingsAboutTab.vue': 'about',
};

interface TemplateRow { file: string; tab: string; id?: string; element: string }

/** The opening tag: up to the first `>` outside an attribute value. */
function openingTag(element: string): string {
  let quoted = false;
  for (let i = 0; i < element.length; i++) {
    if (element[i] === '"') quoted = !quoted;
    else if (element[i] === '>' && !quoted) return element.slice(0, i + 1);
  }
  return element;
}

function templateRows(): TemplateRow[] {
  return TAB_FILES.flatMap(([path, name]) => {
    const source = readFileSync(path, 'utf8');
    const template = source.slice(0, source.indexOf('<script'));
    const rows: TemplateRow[] = [];
    for (const match of template.matchAll(/<settings-(link-)?row\b[\s\S]*?(?:\/>|<\/settings-row>)/g)) {
      const element = match[0];
      const opening = openingTag(element);
      rows.push({ file: name, tab: TAB_OF_FILE[name] ?? 'advanced', id: /\ssearch-id="([^"]+)"/.exec(opening)?.[1], element });
    }
    return rows;
  });
}

describe('settings search index', () => {
  const rows = templateRows();
  const byId = new Map(SETTINGS_INDEX.map(entry => [entry.id, entry]));

  it('finds the rows of every tab', () => {
    expect(rows.length).toBeGreaterThanOrEqual(50);
  });

  it('every row in the Settings tabs is registered, on the right tab, with its label key', () => {
    for (const row of rows) {
      expect(row.id, `${row.file}: a settings row without search-id:\n${row.element.slice(0, 160)}`).toBeTruthy();
      const entry = byId.get(row.id!);
      expect(entry, `${row.file}: search-id "${row.id}" is not in settingsIndex.ts`).toBeDefined();
      expect(entry!.tab, `${row.id}`).toBe(row.tab);
      expect(row.element, `${row.id}: label key`).toContain(`'${entry!.label}'`);
      if (entry!.labelMac) expect(row.element, `${row.id}: mac label key`).toContain(`'${entry!.labelMac}'`);
      if (entry!.hint) expect(row.element, `${row.id}: hint key`).toContain(`'${entry!.hint}'`);
    }
  });

  it('every registered row exists exactly once', () => {
    const ids = rows.map(row => row.id);
    for (const entry of SETTINGS_INDEX) expect(ids.filter(id => id === entry.id), entry.id).toHaveLength(1);
    expect(new Set(SETTINGS_INDEX.map(entry => entry.id)).size).toBe(SETTINGS_INDEX.length);
    for (const entry of SETTINGS_INDEX) if (entry.parent) expect(byId.has(entry.parent), entry.id).toBe(true);
  });

  it('every row has synonyms in English, Arabic and French', () => {
    const synonyms = (messages: unknown) => (messages as { settings: { search: { synonyms: Record<string, string> } } }).settings.search.synonyms;
    for (const [lang, messages] of Object.entries({ en, ar, fr })) {
      for (const entry of SETTINGS_INDEX) expect(synonyms(messages)[entry.id]?.trim(), `${lang}: ${entry.id}`).toBeTruthy();
      expect(Object.keys(synonyms(messages)).sort(), lang).toEqual(SETTINGS_INDEX.map(entry => entry.id).sort());
    }
  });
});
