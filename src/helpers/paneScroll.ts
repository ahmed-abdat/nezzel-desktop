/** Where a scroll key was pressed: on the page itself (nothing focused), in the shell around the content
 *  pane (sidebar, toolbar buttons, status bar), or somewhere the browser already handles it. */
export type ScrollKeyOrigin = 'page' | 'shell' | 'native';

export interface PaneMetrics { scrollTop: number; clientHeight: number; scrollHeight: number }

const LINE = 40;

/**
 * The content pane scrolls inside the window (the window body never does), so PageDown, Space, arrows,
 * Home and End pressed with focus on the page or in the shell would scroll nothing. Returns the pane's new
 * scrollTop, or null to leave the key alone. Space and arrows only from the page: in the shell they
 * activate or move between controls.
 */
export function paneScrollTop(key: string, shift: boolean, origin: ScrollKeyOrigin, pane: PaneMetrics): number | null {
  if (origin === 'native') return null;
  const max = Math.max(0, pane.scrollHeight - pane.clientHeight);
  if (max === 0) return null;
  const page = Math.max(LINE, Math.round(pane.clientHeight * 0.875));
  const fromPageOnly = key === ' ' || key === 'ArrowDown' || key === 'ArrowUp';
  if (fromPageOnly && origin !== 'page') return null;
  const targets: Record<string, number> = {
    'PageDown': pane.scrollTop + page,
    'PageUp': pane.scrollTop - page,
    'Home': 0,
    'End': max,
    ' ': pane.scrollTop + (shift ? -page : page),
    'ArrowDown': pane.scrollTop + LINE,
    'ArrowUp': pane.scrollTop - LINE,
  };
  const next = targets[key];
  if (next === undefined) return null;
  return Math.min(max, Math.max(0, next));
}
