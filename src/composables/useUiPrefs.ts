import { ref, watch } from 'vue';
import type { QuickChoice } from '../helpers/quickChoice';

/** Small UI-only preferences kept in the webview (no engine meaning). */
const SKIP_KEY = 'nezzel.skipOptionsSheet';

function read(key: string): boolean {
  try {
    return globalThis.localStorage?.getItem(key) === '1';
  } catch {
    return false;
  }
}

const skipOptionsSheet = ref(read(SKIP_KEY));
watch(skipOptionsSheet, (value) => {
  try {
    globalThis.localStorage?.setItem(SKIP_KEY, value ? '1' : '0');
  } catch { /* storage unavailable: keep in memory */ }
});

const CHOICE_KEY = 'nezzel.defaultChoice';
const storedChoice = (() => {
  try {
    return globalThis.localStorage?.getItem(CHOICE_KEY);
  } catch {
    return null;
  }
})();
const defaultChoice = ref<QuickChoice>(storedChoice === 'audio' || storedChoice === 'small' ? storedChoice : 'best');
watch(defaultChoice, (value) => {
  try {
    globalThis.localStorage?.setItem(CHOICE_KEY, value);
  } catch { /* storage unavailable: keep in memory */ }
});

/** Boolean view preference persisted in the webview; `fallback` when nothing is stored yet. */
function persistedFlag(key: string, fallback: boolean) {
  let stored: string | null = null;
  try {
    stored = globalThis.localStorage?.getItem(key) ?? null;
  } catch { /* storage unavailable */ }
  const flag = ref(stored == null ? fallback : stored === '1');
  watch(flag, (value) => {
    try {
      globalThis.localStorage?.setItem(key, value ? '1' : '0');
    } catch { /* storage unavailable: keep in memory */ }
  });
  return flag;
}

const compactRows = persistedFlag('nezzel.view.compact', false);
const showThumbnails = persistedFlag('nezzel.view.thumbnails', true);
const sidebarCollapsed = persistedFlag('nezzel.view.sidebarCollapsed', false);

/** String preference persisted in the webview; `fallback` when nothing is stored yet. */
function persistedText(key: string, fallback: string) {
  let stored: string | null = null;
  try {
    stored = globalThis.localStorage?.getItem(key) ?? null;
  } catch { /* storage unavailable */ }
  const text = ref(stored ?? fallback);
  watch(text, (value) => {
    try {
      globalThis.localStorage?.setItem(key, value);
    } catch { /* storage unavailable: keep in memory */ }
  });
  return text;
}

/** In-app keys (only while Nezzel is focused). ⌘ on macOS, Ctrl elsewhere. Empty = off. */
export const IN_APP_SHORTCUT_DEFAULTS = {
  downloadAll: 'CommandOrControl+Enter',
  addAndDownload: 'CommandOrControl+Shift+Enter',
} as const;
const inAppShortcuts = {
  downloadAll: persistedText('nezzel.shortcut.downloadAll', IN_APP_SHORTCUT_DEFAULTS.downloadAll),
  addAndDownload: persistedText('nezzel.shortcut.addAndDownload', IN_APP_SHORTCUT_DEFAULTS.addAndDownload),
};

export function useUiPrefs() {
  return { skipOptionsSheet, defaultChoice, compactRows, showThumbnails, sidebarCollapsed, inAppShortcuts };
}
