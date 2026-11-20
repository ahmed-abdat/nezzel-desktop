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

/**
 * Centres `target` in the pane that scrolls it, and moves nothing else. `scrollIntoView` also scrolls every
 * ancestor that has room, the `overflow: hidden` window shell included: a settings search pick pushed the
 * sidebar and toolbar out of the window, with no way to scroll them back.
 */
export function centreInPane(target: HTMLElement, smooth = false) {
  let pane = target.parentElement;
  // A scroller with nothing to scroll (a short list with a max height) cannot bring the row into view.
  while (pane && !(/auto|scroll/.test(getComputedStyle(pane).overflowY) && pane.scrollHeight > pane.clientHeight)) pane = pane.parentElement;
  if (!pane) return;
  const box = target.getBoundingClientRect();
  const view = pane.getBoundingClientRect();
  pane.scrollTo?.({ top: pane.scrollTop + box.top - view.top - (view.height - box.height) / 2, behavior: smooth ? 'smooth' : 'auto' });
}

/** The window shell stays put whatever asks it to scroll (a focus, an anchor, a stray scrollIntoView). */
export function pinWindowShell(doc: Document = document) {
  const roots = [doc.documentElement, doc.body];
  const reset = () => {
    for (const el of roots) {
      if (el.scrollTop) el.scrollTop = 0;
      if (el.scrollLeft) el.scrollLeft = 0;
    }
  };
  // The root's scroll event names the document in some engines and the root element in others.
  doc.addEventListener('scroll', (event) => {
    if (event.target === doc || roots.includes(event.target as HTMLElement)) reset();
  }, { capture: true, passive: true });
}
