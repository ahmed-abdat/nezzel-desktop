import { nextTick, ref, watch, type Ref } from 'vue';

/**
 * Settings search → the row it picked. The search asks for a row id; every disclosure ("Show more",
 * Logins advanced, Developer options) that holds that row inside its panel opens itself, then the row
 * is scrolled into view, briefly highlighted and its control focused.
 */
const request = ref<{ id: string; seq: number } | null>(null);
let seq = 0;

export const settingSelector = (id: string) => `[data-setting-id="${id}"]`;

/** For a disclosure: call `open` when the requested row sits inside `panel` (rendered but hidden). */
export function useOpenOnReveal(panel: Ref<HTMLElement | null | undefined>, open: () => void) {
  watch(request, (value) => {
    if (value && panel.value?.querySelector(settingSelector(value.id))) open();
  }, { flush: 'sync' });
}

const isHidden = (el: Element) => !!el.closest('[hidden], [style*="display: none"]');

const HIGHLIGHT_MS = 1800;
const FOCUSABLE = 'input:not([type="hidden"]):not([disabled]), textarea:not([disabled]), button:not([disabled]), [role="switch"], [role="combobox"], [tabindex]:not([tabindex="-1"])';

/**
 * Opens what hides the row, then scrolls to it, highlights it and focuses its control. Falls back to
 * `parentId` when the row itself is not on screen (it only shows in some states). Returns the row.
 */
export async function revealSetting(root: ParentNode, id: string, parentId?: string): Promise<HTMLElement | null> {
  let target: HTMLElement | null = null;
  for (const candidate of parentId ? [id, parentId] : [id]) {
    request.value = { id: candidate, seq: ++seq };
    // Disclosure panels need a render (and reka's Presence another) before the row is visible.
    for (let tick = 0; tick < 3; tick++) {
      await nextTick();
      const el = root.querySelector<HTMLElement>(settingSelector(candidate));
      if (el && !isHidden(el)) {
        target = el;
        break;
      }
    }
    if (target) break;
  }
  if (!target) return null;

  const reduce = typeof matchMedia === 'function' && matchMedia('(prefers-reduced-motion: reduce)').matches;
  target.scrollIntoView?.({ block: 'center', behavior: reduce ? 'auto' : 'smooth' });
  target.removeAttribute('data-search-hit');
  void target.offsetWidth; // restart the fade when the same row is picked twice
  target.setAttribute('data-search-hit', '');
  const row = target;
  setTimeout(() => row.removeAttribute('data-search-hit'), HIGHLIGHT_MS);
  const control = target.matches('a, button') ? target : target.querySelector<HTMLElement>(FOCUSABLE);
  control?.focus({ preventScroll: true });
  return target;
}
