import { foldForSearch } from './listFilter';

/**
 * Settings search: one document per setting row, matched word by word against what people type,
 * in any of the shipped languages. Pure: the view builds the documents from the row registry.
 *
 * Ranking (per query word, best field wins): label word exact > label word prefix > synonym >
 * section name > hint > typo-tolerant match. On top of that, a label equal to the whole query wins
 * outright, and a query that is exactly one of a setting's curated keywords ("folder" → Save to)
 * outranks a label that merely starts with it ("Folders and file names").
 */
export interface SearchDoc {
  id: string;
  label: string;
  hint?: string;
  section?: string;
  /** Other words people use for this setting, in every language (comma-separated lists already split). */
  synonyms: string[];
}

export interface SearchHit {
  id: string;
  score: number;
}

interface Weights { exact: number; prefix: number; fuzzy: number }
const LABEL: Weights = { exact: 100, prefix: 80, fuzzy: 22 };
const SYNONYM: Weights = { exact: 70, prefix: 60, fuzzy: 20 };
const SECTION: Weights = { exact: 45, prefix: 40, fuzzy: 12 };
const HINT: Weights = { exact: 40, prefix: 35, fuzzy: 10 };

/** Little words that carry no meaning in a settings query ("change the folder", "dossier de", "مجلد في"). */
const STOP_WORDS = new Set([
  'the', 'a', 'an', 'to', 'of', 'for', 'in', 'on', 'my', 'how', 'i', 'and', 'or', 'is',
  'le', 'la', 'les', 'l', 'de', 'des', 'du', 'd', 'un', 'une', 'mon', 'ma', 'mes', 'pour', 'et', 'en', 'au', 'aux',
  'في', 'من', 'على', 'الى', 'عن', 'و', 'او',
]);

/** Arabic attached article and conjunctions: "المجلد", "والصوت", "بالمتصفح" also match "مجلد", "صوت", "متصفح". */
const ARABIC_PREFIX = /^(?:وال|بال|كال|فال|لل|ال)(?=\p{L}{2})/u;

/** Folds case, Latin accents, Arabic harakat/tatweel/hamza seats and punctuation, then collapses spaces. */
export function normalizeSearch(text: string): string {
  return foldForSearch(text).replace(/[^\p{L}\p{N}]+/gu, ' ').trim();
}

function wordsOf(text: string | undefined): string[] {
  if (!text) return [];
  const out = new Set<string>();
  for (const word of normalizeSearch(text).split(' ')) {
    if (!word) continue;
    out.add(word);
    const bare = word.replace(ARABIC_PREFIX, '');
    if (bare !== word) out.add(bare);
  }
  return [...out];
}

/** Words of the query that matter: stop words dropped (unless that leaves nothing), Arabic article stripped. */
export function queryWords(query: string): string[] {
  const all = normalizeSearch(query).split(' ').filter(Boolean);
  const meaningful = all.filter(word => !STOP_WORDS.has(word));
  return (meaningful.length ? meaningful : all).map(word => word.replace(ARABIC_PREFIX, ''));
}

/** Typos allowed for a word of this length: none under 4 letters, 1 for 4-6, 2 from 7. */
export function allowedTypos(length: number): number {
  return length >= 7 ? 2 : length >= 4 ? 1 : 0;
}

/** Optimal string alignment distance (Damerau-Levenshtein with adjacent swaps), capped: returns max + 1 once over. */
export function editDistance(a: string, b: string, max = Infinity): number {
  if (Math.abs(a.length - b.length) > max) return max + 1;
  let prevPrev: number[] = [];
  let prev = Array.from({ length: b.length + 1 }, (_, j) => j);
  for (let i = 1; i <= a.length; i++) {
    const row = [i];
    let rowMin = i;
    for (let j = 1; j <= b.length; j++) {
      const cost = a[i - 1] === b[j - 1] ? 0 : 1;
      let value = Math.min(prev[j] + 1, row[j - 1] + 1, prev[j - 1] + cost);
      if (i > 1 && j > 1 && a[i - 1] === b[j - 2] && a[i - 2] === b[j - 1]) value = Math.min(value, prevPrev[j - 2] + 1);
      row.push(value);
      rowMin = Math.min(rowMin, value);
    }
    if (rowMin > max) return max + 1;
    prevPrev = prev;
    prev = row;
  }
  return prev[b.length];
}

function wordScore(token: string, words: string[], weights: Weights, allowPrefix: boolean): number {
  let best = 0;
  const typos = allowedTypos(token.length);
  for (const word of words) {
    if (word === token) return weights.exact;
    if (allowPrefix && word.startsWith(token)) {
      best = Math.max(best, weights.prefix);
      continue;
    }
    if (!typos) continue;
    const whole = editDistance(token, word, typos);
    if (whole <= typos) best = Math.max(best, weights.fuzzy - whole);
    // A typo in a word still being typed: "notifcat" → "notifications".
    if (token.length >= 5 && word.length > token.length) {
      const head = Math.min(editDistance(token, word.slice(0, token.length), typos), editDistance(token, word.slice(0, token.length + 1), typos));
      if (head <= typos) best = Math.max(best, weights.fuzzy - head - 1);
    }
  }
  return best;
}

interface Prepared {
  doc: SearchDoc;
  label: string[];
  synonyms: string[];
  section: string[];
  hint: string[];
  labelText: string;
  synonymTexts: string[];
}

export function prepareDocs(docs: SearchDoc[]): Prepared[] {
  return docs.map(doc => ({
    doc,
    label: wordsOf(doc.label),
    synonyms: [...new Set(doc.synonyms.flatMap(wordsOf))],
    section: wordsOf(doc.section),
    hint: wordsOf(doc.hint),
    labelText: normalizeSearch(doc.label),
    synonymTexts: doc.synonyms.map(normalizeSearch),
  }));
}

/**
 * Ranked matches for `query`. Every meaningful word must match some field; when no setting matches
 * every word, the settings matching the most words are returned instead (best guess, never a dead end
 * because of one stray word). Ties keep registry order (tab order on screen).
 */
export function searchSettings(docs: SearchDoc[] | Prepared[], query: string): SearchHit[] {
  const tokens = queryWords(query);
  if (!tokens.length) return [];
  const prepared = (docs.length && 'doc' in docs[0] ? docs : prepareDocs(docs as SearchDoc[])) as Prepared[];
  const phrase = normalizeSearch(query);
  const scored = prepared.map((entry, order) => {
    let score = 0;
    let matched = 0;
    for (const token of tokens) {
      const single = token.length === 1;
      const best = Math.max(
        wordScore(token, entry.label, LABEL, true),
        wordScore(token, entry.synonyms, SYNONYM, !single),
        single ? 0 : wordScore(token, entry.section, SECTION, true),
        single ? 0 : wordScore(token, entry.hint, HINT, true),
      );
      if (best > 0) {
        matched++;
        score += best;
      }
    }
    if (entry.labelText === phrase) score += 1000;
    else if (entry.synonymTexts.includes(phrase)) score += 50;
    else if (entry.labelText.startsWith(phrase)) score += 15;
    return { id: entry.doc.id, score, matched, order };
  });
  const most = Math.max(...scored.map(hit => hit.matched));
  if (most === 0 || most < Math.ceil(tokens.length / 2)) return [];
  return scored
    .filter(hit => hit.matched === most)
    .sort((a, b) => b.score - a.score || a.order - b.order)
    .map(({ id, score }) => ({ id, score }));
}
