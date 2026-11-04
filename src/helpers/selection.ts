/** Pure selection logic for the downloads list. `order` is the visible (filtered, searched) row order. */

/** Cmd/Ctrl+click: flips one row. */
export function toggleId(selected: ReadonlySet<string>, id: string): Set<string> {
  const next = new Set(selected);
  if (!next.delete(id)) next.add(id);
  return next;
}

/** Shift+click: every visible row between the anchor and the target, both included. No anchor in view: just the target. */
export function rangeIds(order: readonly string[], anchor: string | null, target: string): Set<string> {
  const to = order.indexOf(target);
  if (to < 0) return new Set();
  const from = anchor == null ? -1 : order.indexOf(anchor);
  if (from < 0) return new Set([target]);
  const [start, end] = from <= to ? [from, to] : [to, from];
  return new Set(order.slice(start, end + 1));
}

export function selectAllIds(order: readonly string[]): Set<string> {
  return new Set(order);
}

/** Drops rows that left the list (removed, or filtered out). */
export function pruneIds(selected: ReadonlySet<string>, order: readonly string[]): Set<string> {
  const visible = new Set(order);
  return new Set([...selected].filter(id => visible.has(id)));
}

export type BatchPhase = 'reading' | 'playlist' | 'ready' | 'downloading' | 'paused' | 'done' | 'error';
export interface BatchEntry { id: string; phase: BatchPhase; hasFile: boolean }

/** Which selected rows each batch action applies to. An empty list means the action is hidden. */
export function batchTargets(entries: readonly BatchEntry[]) {
  const ids = (pick: (entry: BatchEntry) => boolean) => entries.filter(pick).map(entry => entry.id);
  return {
    download: ids(e => e.phase === 'ready'),
    pause: ids(e => e.phase === 'downloading'),
    resume: ids(e => e.phase === 'paused'),
    retry: ids(e => e.phase === 'error'),
    remove: ids(() => true),
    deleteFiles: ids(e => e.phase === 'done' && e.hasFile),
  };
}
