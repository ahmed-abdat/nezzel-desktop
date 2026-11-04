import { onBeforeUnmount, ref } from 'vue';

/** Reactive `matchMedia`. False where matchMedia is missing (tests). */
export function useMediaQuery(query: string) {
  const mql = typeof window !== 'undefined' && window.matchMedia ? window.matchMedia(query) : null;
  const matches = ref(mql?.matches ?? false);
  const update = (event: MediaQueryListEvent) => {
    matches.value = event.matches;
  };
  mql?.addEventListener?.('change', update);
  onBeforeUnmount(() => mql?.removeEventListener?.('change', update));
  return matches;
}
