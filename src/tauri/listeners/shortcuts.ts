import { listen } from '@tauri-apps/api/event';
import type { ShortcutAction, ShortcutActionPayload } from '../types/shortcuts.ts';
import { useMediaStore } from '../../stores/media/media.ts';
import { readClipboardText } from '../clipboard.ts';
import { isValidUrl } from '../../helpers/url.ts';

/** Runs a queue shortcut: from the in-app keys (AppLayout) or a legacy `shortcut_action` event. */
export async function runShortcutAction(action: ShortcutAction) {
  const mediaStore = useMediaStore();
  switch (action) {
    case 'media_add': {
      const url = await readClipboardText() ?? '';
      if (!isValidUrl(url)) return;
      await mediaStore.dispatchMediaInfoFetch(url, true);
      break;
    }
    case 'media_add_and_download': {
      const url = await readClipboardText() ?? '';
      if (!isValidUrl(url)) return;
      await mediaStore.addAndDownload(url, true);
      break;
    }
    case 'download_all': {
      await mediaStore.downloadAllGroups(true);
      break;
    }
    default: {
      console.warn(`Unknown shortcut action: ${action}`);
    }
  }
}

export function registerShortcutListeners() {
  void listen<ShortcutActionPayload>('shortcut_action', event => runShortcutAction(event.payload.action));
}
