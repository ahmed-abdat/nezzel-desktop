import { getCurrentScope, onScopeDispose, watch } from 'vue';
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
import type { QuickChoice } from '../../helpers/quickChoice';
import { isValidUrl } from '../../helpers/url.ts';
import { useReportStore } from '../../stores/report';

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

/** How often the main window pulls capture actions an event may not have delivered. */
export const PULL_MS = 1500;

/** Capture ids the main window remembers to dedupe late or repeated deliveries; Rust drops unpulled ones
 *  after the same two minutes, so nothing older can arrive. */
export const CAPTURE_MEMORY_MS = 120_000;
export const CAPTURE_MEMORY_CAP = 50;

/** Forgets captures past the TTL, then the oldest past the cap (Map keeps insertion order). */
export function pruneCaptures<T extends { at: number }>(captures: Map<number, T>, now: number) {
  for (const [id, capture] of captures) {
    if (now - capture.at >= CAPTURE_MEMORY_MS) captures.delete(id);
  }
  for (const id of captures.keys()) {
    if (captures.size <= CAPTURE_MEMORY_CAP) break;
    captures.delete(id);
  }
}

/** Only the main webview owns the queue; capture panels never install media listeners. */
export async function registerCaptureListeners() {
  // Read before the first await: the effect scope is no longer current after it.
  const scope = getCurrentScope();
  const media = useMediaStore();
  const sheet = useDownloadSheetStore();
  const groups = useMediaGroupStore();
  const states = useMediaStateStore();
  const captures = new Map<number, { groupId: string; awaitingItem: boolean; upgraded: boolean; at: number }>();

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

  type CaptureActionPayload = { id: number; url: string; action: string; item?: MediaItem | null; choice?: QuickChoice | null };
  type CaptureItemPayload = { id: number; item: MediaItem | null };

  /** Events and the pull both land here; the capture id makes a second delivery a no-op. */
  function onCaptureAction(payload: CaptureActionPayload) {
    if (!['download', 'options', 'adopt'].includes(payload.action)) return;
    const known = captures.get(payload.id);
    // Focus adopted the row as the card was clicked (X11): the click still starts it or opens its options.
    if (known && (payload.action === 'adopt' || known.upgraded)) return;
    // The same link was just added from the bar or the field: focus must not adopt it as a second row.
    if (!known && payload.action === 'adopt' && media.hasRecentCapture(payload.url)) return;
    const groupId = known?.groupId ?? media.createCaptureGroup(payload.url, payload.action === 'download');
    if (known) known.upgraded = true;
    else {
      pruneCaptures(captures, Date.now());
      captures.set(payload.id, { groupId, awaitingItem: true, upgraded: payload.action !== 'adopt', at: Date.now() });
    }
    if (payload.action === 'download') {
      void media.downloadPreparedGroup(groupId, payload.choice ?? undefined).catch(() => {});
    } else if (payload.action === 'options') {
      sheet.show(groupId);
      void getCurrentWindow().show().then(() => getCurrentWindow().setFocus()).catch(() => {});
    }
    if (payload.item) void fillCapture(payload.id, payload.item);
  }

  /** Linux can hold an event for a hidden window until the next input (tauri#3654): pull what Rust sent. */
  async function pullPending() {
    try {
      const pending = await invoke<{ actions: CaptureActionPayload[]; items: CaptureItemPayload[] }>('capture_take_pending');
      for (const action of pending.actions) onCaptureAction(action);
      for (const item of pending.items) void fillCapture(item.id, item.item);
    } catch {
      // Not ready yet, or a test without the command: the events still work.
    }
  }

  // Install the completion listener first; capture_ready enables native capture only afterwards.
  await listen<CaptureItemPayload>('capture_item', ({ payload }) => {
    void fillCapture(payload.id, payload.item);
  });
  // A failed read in the bar or card → Report: the reason word only; the link goes along only if the user
  // ticks "Include the link" in the dialog (privacy rule).
  await listen<{ url: string; reason: string }>('capture_report', ({ payload }) => {
    useReportStore().openReport({
      context: { errorCode: `capture:${payload.reason}`, exitCode: null, attempts: [] },
      url: payload.url || null,
      extractor: null,
    });
  });
  await listen<CaptureActionPayload>('capture_action', ({ payload }) => onCaptureAction(payload));
  const timer = setInterval(() => void pullPending(), PULL_MS);
  const onFocus = () => void pullPending();
  window.addEventListener('focus', onFocus);
  scope?.run(() => onScopeDispose(() => {
    clearInterval(timer);
    window.removeEventListener('focus', onFocus);
  }));
  watch(() => Object.values(groups.groups).flatMap(group => Object.values(group.items))
    .filter(item => [MediaState.downloading, MediaState.downloadingList].includes(states.getState(item.id) as MediaState))
    .map(item => item.title ?? ''), (titles) => {
    void invoke('capture_tray_progress', { count: titles.length, titles }).catch(() => {});
  }, { immediate: true });
  await invoke('capture_ready');
}
