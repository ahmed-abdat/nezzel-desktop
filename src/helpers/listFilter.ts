import type { MediaState } from '../stores/media/state';
import { phaseOf, type KindFilter } from './groupView';

/**
 * The downloads list has two independent filters: the sidebar picks a type, the chips above the
 * list pick a status. They combine (e.g. failed videos).
 */
export type TypeFilter = 'all' | KindFilter;
export type StatusFilter = 'all' | 'active' | 'failed' | 'done';
export interface ListView { type: TypeFilter; status: StatusFilter }

export type RowStatus = 'active' | 'done' | 'failed';

const TYPES: readonly TypeFilter[] = ['all', 'video', 'audio', 'photos', 'files'];
const STATUSES: readonly StatusFilter[] = ['all', 'active', 'failed', 'done'];

/**
 * Route query → view; unknown values fall back to "all". Old single-filter links (`?filter=`)
 * still work: a kind becomes the type, active/failed/done become the status, finished shows all.
 */
export function parseListView(query: { type?: unknown; status?: unknown; filter?: unknown }): ListView {
  const pick = <T extends string>(value: unknown, allowed: readonly T[]): T | null =>
    (allowed as readonly string[]).includes(String(value)) ? value as T : null;
  const legacy = String(query.filter ?? '');
  return {
    type: pick(query.type, TYPES) ?? pick(legacy, TYPES) ?? 'all',
    status: pick(query.status, STATUSES) ?? pick(legacy, STATUSES) ?? 'all',
  };
}

/** View → route query, leaving out defaults so the plain list is just `{ name: 'home' }`. */
export function viewQuery(view: ListView): Record<string, string> {
  const query: Record<string, string> = {};
  if (view.type !== 'all') query.type = view.type;
  if (view.status !== 'all') query.status = view.status;
  return query;
}

export function statusOf(state: MediaState | undefined): RowStatus {
  const phase = state === undefined ? undefined : phaseOf(state);
  if (phase === 'done') return 'done';
  return phase === 'error' ? 'failed' : 'active';
}

export interface ListRow {
  status: RowStatus;
  kind: KindFilter | null;
  title: string;
  site: string;
  /** Finished row the user removed from the list: hidden from "All", still under "Completed". */
  cleared?: boolean;
}

export function matchesView(row: Pick<ListRow, 'status' | 'kind' | 'cleared'>, view: ListView): boolean {
  if (view.type !== 'all' && row.kind !== view.type) return false;
  if (view.status === 'all') return !row.cleared;
  return row.status === view.status;
}

/**
 * Folds text for search: case, Latin accents, Arabic harakat/tatweel, and the Arabic letter
 * variants people type interchangeably (أ إ آ ٱ → ا, ى → ي, ة → ه, ؤ → و, ئ → ي).
 */
export function foldForSearch(text: string): string {
  return text
    .normalize('NFKD')
    .replace(/\p{M}/gu, '')
    .replace(/ـ/g, '')
    .replace(/[آأإٱ]/g, 'ا')
    .replace(/ى/g, 'ي')
    .replace(/ة/g, 'ه')
    .replace(/ؤ/g, 'و')
    .replace(/ئ/g, 'ي')
    .toLocaleLowerCase('en')
    .replace(/\s+/g, ' ')
    .trim();
}

/** Every word of the query must appear in the title or the site name. Empty query matches everything. */
export function matchesSearch(row: Pick<ListRow, 'title' | 'site'>, query: string): boolean {
  const words = foldForSearch(query).split(' ').filter(Boolean);
  if (words.length === 0) return true;
  const haystack = foldForSearch(`${row.title} ${row.site}`);
  return words.every(word => haystack.includes(word));
}

export function filterRows<T extends ListRow>(rows: T[], view: ListView, query: string): T[] {
  return rows.filter(row => matchesView(row, view) && matchesSearch(row, query));
}

/** Sidebar counts per type (any status), and chip counts per status within the selected type. */
export function countView(rows: Pick<ListRow, 'status' | 'kind' | 'cleared'>[], type: TypeFilter) {
  const types = Object.fromEntries(TYPES.map(f => [f, 0])) as Record<TypeFilter, number>;
  const statuses = Object.fromEntries(STATUSES.map(f => [f, 0])) as Record<StatusFilter, number>;
  for (const row of rows) {
    for (const filter of TYPES) {
      if (matchesView(row, { type: filter, status: 'all' })) types[filter] += 1;
    }
    for (const filter of STATUSES) {
      if (matchesView(row, { type, status: filter })) statuses[filter] += 1;
    }
  }
  return { types, statuses };
}

/** Ids "Remove from list" takes off the main list: done rows not already removed, never active or failed ones. */
export function finishedIds<T extends { id: string; status: RowStatus; cleared?: boolean }>(rows: T[]): string[] {
  return rows.filter(row => row.status === 'done' && !row.cleared).map(row => row.id);
}

/** A choice or a live consent prompt needs the user, even while the engine waits. */
export function needsAttention(state: MediaState, loginNeeded: boolean): boolean {
  return loginNeeded || ['ready', 'playlist'].includes(phaseOf(state));
}
