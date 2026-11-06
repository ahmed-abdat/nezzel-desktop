import { computed, onMounted, onBeforeUnmount, watchEffect, ref } from 'vue';
import { useSettingsStore } from '../stores/settings';
import { Theme } from '../tauri/types/config';
import { applyTheme, removeTheme } from '../appearance';

export { applyTheme };

export function useTheme() {
  const store = useSettingsStore();

  const systemTheme = ref('dark' as Theme);
  const theme = computed<Theme>({
    get: () => store.settings.appearance.theme ?? 'system',
    set: val => void store.patchNow({ appearance: { theme: val } }),
  });

  let cleanup: (() => void) | undefined;

  onMounted(() => {
    watchEffect(() => theme.value === 'system' ? removeTheme() : applyTheme(theme.value));

    if (typeof window !== 'undefined') {
      const mql = window.matchMedia('(prefers-color-scheme: dark)');
      systemTheme.value = mql.matches ? 'dark' : 'light';
      const handler = () => {
        systemTheme.value = mql.matches ? 'dark' : 'light';
        if (theme.value === 'system') removeTheme();
      };
      mql.addEventListener?.('change', handler);
      cleanup = () => mql.removeEventListener?.('change', handler);
    }
  });

  onBeforeUnmount(() => cleanup?.());

  function setTheme(next: Theme) {
    theme.value = next;
  }

  return { theme, systemTheme, setTheme };
}
