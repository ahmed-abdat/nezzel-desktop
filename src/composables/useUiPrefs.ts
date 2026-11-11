import { persistedRef, flagCodec, textCodec, choiceCodec } from '../helpers/persistedRef';
import type { QuickChoice } from '../helpers/quickChoice';
import { IN_APP_SHORTCUT_DEFAULTS } from '../helpers/shortcut';

/** Small UI-only preferences kept in the webview (no engine meaning). */
const skipOptionsSheet = persistedRef('nezzel.skipOptionsSheet', flagCodec, false);
const defaultChoice = persistedRef<QuickChoice>('nezzel.defaultChoice', choiceCodec<QuickChoice>(['best', 'audio', 'small']), 'best');
const rememberNoticeShown = persistedRef('nezzel.rememberNoticeShown', flagCodec, false);

const compactRows = persistedRef('nezzel.view.compact', flagCodec, false);
const showThumbnails = persistedRef('nezzel.view.thumbnails', flagCodec, true);
const sidebarCollapsed = persistedRef('nezzel.view.sidebarCollapsed', flagCodec, false);

/** In-app keys (only while Nezzel is focused); defaults live in helpers/shortcut. Empty = off. */
const inAppShortcuts = {
  downloadAll: persistedRef('nezzel.shortcut.downloadAll', textCodec, IN_APP_SHORTCUT_DEFAULTS.downloadAll),
  addAndDownload: persistedRef('nezzel.shortcut.addAndDownload', textCodec, IN_APP_SHORTCUT_DEFAULTS.addAndDownload),
};

export function useUiPrefs() {
  return { skipOptionsSheet, defaultChoice, rememberNoticeShown, compactRows, showThumbnails, sidebarCollapsed, inAppShortcuts };
}
