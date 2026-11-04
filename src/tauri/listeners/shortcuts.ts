import { watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { useDownloadSheetStore } from '../../stores/downloadSheet';
import { useMediaGroupStore } from '../../stores/media/group';
import { MediaState, useMediaStateStore } from '../../stores/media/state';
import { getCurrentWindow } from '@tauri-apps/api/window';
import type { MediaItem } from '../types/media';
import { listen } from '@tauri-apps/api/event';
import { ShortcutActionPayload } from '../types/shortcuts.ts';
import { useMediaStore } from '../../stores/media/media.ts';
import { readClipboardText } from '../clipboard.ts';
import { isValidUrl } from '../../helpers/url.ts';

export type ShortcutAction = 'media_add' | 'media_add_and_download' | 'download_all';

/** Runs a queue shortcut: from the in-app keys (AppLayout) or a legacy `shortcut_action` event. */
export async function runShortcutAction(action: string) {
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

/** Only the main webview owns the queue; capture panels never install media listeners. */
export async function registerCaptureListeners() {
  const media = useMediaStore();
  const sheet = useDownloadSheetStore();
  const groups = useMediaGroupStore();
  const states = useMediaStateStore();
  const captures = new Map<number, { groupId: string; awaitingItem: boolean; upgraded: boolean }>();

  async function fillCapture(id: number, item?: MediaItem | null) {
    const capture = captures.get(id);
    if (!capture?.awaitingItem) return;
    // Consume before any await: duplicate/late results must never fetch or download twice.
    capture.awaitingItem = false;
    if (!groups.findGroupById(capture.groupId)) return;
    try {
      if (item) {
        media.processMediaAddPayload({ groupId: capture.groupId, total: item.entries?.length ?? 1, item });
      } else {
        await media.fetchMediaInfoForGroup(capture.groupId);
      }
    } catch {
      // Deliberately do not report engine errors together with captured URLs.
    }
  }

  // Install the completion listener first; capture_ready enables native capture only afterwards.
  await listen<{ id: number; item: MediaItem | null }>('capture_item', ({ payload }) => {
    void fillCapture(payload.id, payload.item);
  });
  await listen<{ id: number; url: string; action: string; item?: MediaItem | null }>('capture_action', ({ payload }) => {
    if (!['download', 'options', 'adopt'].includes(payload.action)) return;
    const known = captures.get(payload.id);
    // Focus adopted the row as the card was clicked (X11): the click still starts it or opens its options.
    if (known && (payload.action === 'adopt' || known.upgraded)) return;
    const groupId = known?.groupId ?? media.createCaptureGroup(payload.url, payload.action === 'download');
    if (known) known.upgraded = true;
    else captures.set(payload.id, { groupId, awaitingItem: true, upgraded: payload.action !== 'adopt' });
    if (payload.action === 'download') {
      void media.downloadPreparedGroup(groupId).catch(() => {});
    } else if (payload.action === 'options') {
      sheet.show(groupId);
      void getCurrentWindow().show().then(() => getCurrentWindow().setFocus()).catch(() => {});
    }
    if (payload.item) void fillCapture(payload.id, payload.item);
  });
  watch(() => Object.values(groups.groups).flatMap(group => Object.values(group.items))
    .filter(item => [MediaState.downloading, MediaState.downloadingList].includes(states.getState(item.id) as MediaState))
    .map(item => item.title ?? ''), (titles) => {
    void invoke('capture_tray_progress', { count: titles.length, titles }).catch(() => {});
  }, { immediate: true });
  await invoke('capture_ready');
}
