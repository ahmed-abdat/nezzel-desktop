import { onScopeDispose, ref } from 'vue';

/**
 * Full-text tooltip only for text that is actually cut off. Checked on pointer enter, so the tip
 * always belongs to the element under the pointer (native `title` tooltips linger and float over
 * other rows when the list reorders). A `[data-clip]` child, when present, is the box that truncates.
 * After the first hover the element is watched, so a window or sidebar resize keeps the state true.
 */
export function useOverflowTip() {
  const clipped = ref(false);
  let observed: HTMLElement | null = null;
  const observer = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(() => check());

  function check() {
    if (observed) clipped.value = observed.scrollWidth > observed.clientWidth + 1 || observed.scrollHeight > observed.clientHeight + 1;
  }

  function measure(event: Event) {
    const host = event.currentTarget;
    const el = host instanceof HTMLElement ? (host.querySelector<HTMLElement>('[data-clip]') ?? host) : null;
    if (!el) return;
    if (el !== observed) {
      if (observed) observer?.unobserve(observed);
      observed = el;
      observer?.observe(el);
    }
    check();
  }

  onScopeDispose(() => {
    observer?.disconnect();
    observed = null;
  });
  return { clipped, measure };
}
