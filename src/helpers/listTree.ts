import { matchesSearch, matchesView, type ListRow, type ListView, type RowStatus } from './listFilter';

/** A flat list row that may belong to a playlist (`parentId`) at a position (`parentIndex`). */
export type TreeInput = ListRow & { id: string; parentId?: string; parentIndex?: number };

export type ParentMeta = { id: string; title?: string };

/** One top-level line: a single download, or a playlist with the children this view shows. */
export type TreeNode<T extends TreeInput>
  = | { kind: 'row'; id: string; row: T }
    | {
      kind: 'playlist'; id: string; parent: ParentMeta;
      /** Every child in the list (aggregate, actions). */
      all: T[];
      /** Children this view shows (filters, search). */
      shown: T[];
      status: RowStatus; needsAttention: boolean; title: string; site: string;
    };

/** Where a playlist sits: still running anywhere → in progress; else needs you; else failed; else finished. */
export function parentStatus(children: Pick<ListRow, 'status' | 'needsAttention' | 'cleared'>[]): { status: RowStatus; needsAttention: boolean } {
  const live = children.filter(child => !child.cleared);
  const pool = live.length ? live : children;
  if (pool.some(child => child.status === 'active' && !child.needsAttention)) return { status: 'active', needsAttention: false };
  const attention = pool.find(child => child.needsAttention);
  if (attention) return { status: attention.status, needsAttention: true };
  if (pool.some(child => child.status === 'failed')) return { status: 'failed', needsAttention: false };
  if (pool.some(child => child.status === 'cancelled')) return { status: 'cancelled', needsAttention: true };
  return { status: 'done', needsAttention: false };
}

/**
 * Folds playlist children under one node placed where its first child is. Children are in playlist
 * order. A playlist shows when it, or one of its children, matches the search; the children shown are
 * those that match the view (and the search, unless the playlist title itself matched).
 */
export function buildListTree<T extends TreeInput>(rows: T[], parents: Record<string, ParentMeta>, view?: ListView, query = ''): TreeNode<T>[] {
  const nodes: TreeNode<T>[] = [];
  const byParent = new Map<string, T[]>();
  for (const row of rows) {
    const parent = row.parentId ? parents[row.parentId] : undefined;
    if (!parent) {
      if (!view || (matchesView(row, view) && matchesSearch(row, query))) nodes.push({ kind: 'row', id: row.id, row });
      continue;
    }
    const list = byParent.get(parent.id);
    if (list) {
      list.push(row);
      continue;
    }
    const children = [row];
    byParent.set(parent.id, children);
    nodes.push({ kind: 'playlist', id: parent.id, parent, all: children, shown: [], status: 'active', needsAttention: false, title: parent.title ?? '', site: row.site });
  }
  return nodes.filter((node) => {
    if (node.kind === 'row') return true;
    node.all.sort((a, b) => (a.parentIndex ?? 0) - (b.parentIndex ?? 0));
    Object.assign(node, parentStatus(node.all));
    const titleHit = matchesSearch({ title: node.title, site: node.site }, query);
    node.shown = view
      ? node.all.filter(child => matchesView(child, view) && (titleHit || matchesSearch(child, query)))
      : node.all.filter(child => !child.cleared);
    // A status chip shows the playlist under that status, with the children that have it.
    if (view && view.status !== 'all') Object.assign(node, parentStatus(node.shown));
    return node.shown.length > 0;
  });
}
