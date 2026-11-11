import { getCurrentScope, onScopeDispose, shallowRef } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { useDownloadSheetStore } from '../../stores/downloadSheet';
import { useMediaGroupStore } from '../../stores/media/group';
import { MediaState, useMediaStateStore } from '../../stores/media/state';
import { useMediaStore } from '../../stores/media/media';
import { useReportStore } from '../../stores/report';
import type { MediaItem } from '../types/media';
import { CAPTURE_CLAIMS, CAPTURE_PULL_MS, type CaptureChoice } from '@/generated/contract';

/** What Rust queued for the main window (`capture::outbox`), numbered in order. */
export type CaptureMessage = { seq: number } & (
  /** A claim on an offer; `id` is the row (an upgrade names the row focus adopted). */
  | { kind: 'action'; id: number; url: string; action: string; item: MediaItem | null; choice: CaptureChoice | null }
  /** The read item of a row whose claim went out first; Rust sends one per row. */
  | { kind: 'item'; id: number; item: MediaItem | null }
  /** The clipboard's supported link for the link field ('' = none). */
  | { kind: 'prefill'; url: string }
  /** A failed read → Report: the reason word only; the link goes along only if the user ticks "Include the link". */
  | { kind: 'report'; url: string; reason: string }
);

/** The newest clipboard link for the link field (useClipboard); null until the first prefill message. */
export const capturePrefill = shallowRef<string | null>(null);

/** One row per capture id: an upgrade (focus adopted it, then a click) finds the same row. */
export const captureGroupId = (id: number) => `capture-${id}`;

/**
 * The main window's only way in from the capture windows: pull everything after the last seq handled, on the
 * `capture_wake` event, on focus and every `CAPTURE_PULL_MS` (Linux can hold events for a hidden window,
 * tauri#3654). Listening comes first, then `capture_ready` turns native capture on.
 */
export async function startCaptureDelivery() {
  // Read before the first await: the effect scope is no longer current after it.
  const scope = getCurrentScope();
  const media = useMediaStore();
  const sheet = useDownloadSheetStore();
  const groups = useMediaGroupStore();
  const states = useMediaStateStore();
  const optionChoices = new Map<string, CaptureChoice>();
  let lastSeq = 0;

  function prepareOptions(groupId: string) {
    const choice = optionChoices.get(groupId);
    optionChoices.delete(groupId);
    return media.prepareCaptureGroup(groupId, choice);
  }

  async function fillRow(id: number, item: MediaItem | null) {
    const groupId = captureGroupId(id);
    if (!groups.findGroupById(groupId)) {
      optionChoices.delete(groupId);
      return;
    }
    try {
      if (item) media.processMediaAddPayload({ groupId, total: item.entries?.length ?? 1, item });
      else await media.fetchMediaInfoForGroup(groupId);
      // Apply a pending Options choice using the read source kind and formats, never a guessed video ceiling.
      if (optionChoices.has(groupId) || (groups.findGroupById(groupId)?.entries && sheet.open && sheet.groupId === groupId)) {
        await prepareOptions(groupId);
      }
    } catch {
      // Deliberately do not report engine errors together with captured URLs.
    }
  }

  function openPicker(groupId: string) {
    sheet.show(groupId);
    // Focus may already have adopted and read this row before the Options click.
    if (groups.findGroupById(groupId)?.entries || (optionChoices.has(groupId) && states.getGroupState(groupId) === MediaState.configure)) {
      void prepareOptions(groupId).catch(() => {});
    }
    void getCurrentWindow().unminimize().then(() => getCurrentWindow().show()).then(() => getCurrentWindow().setFocus()).catch(() => {});
  }

  function handle(message: CaptureMessage) {
    switch (message.kind) {
      case 'action': {
        if (!(CAPTURE_CLAIMS as readonly string[]).includes(message.action)) return;
        const groupId = captureGroupId(message.id);
        // An existing row is one focus adopted; Rust sends the click that upgrades it once.
        if (!groups.findGroupById(groupId)) media.createCaptureGroup(groupId, message.url);
        if (message.action === 'download') {
          void media.downloadPreparedGroup(groupId, message.choice ?? undefined).then((result) => {
            if (result === 'playlist') openPicker(groupId);
          }).catch(() => {});
        } else if (message.action === 'options') {
          // Keep the choice until the read establishes whether this is a playlist, video or audio source.
          if (message.choice) optionChoices.set(groupId, message.choice);
          openPicker(groupId);
        }
        if (message.item) void fillRow(message.id, message.item);
        return;
      }
      case 'item':
        void fillRow(message.id, message.item);
        return;
      case 'prefill':
        capturePrefill.value = message.url;
        return;
      case 'report':
        useReportStore().openReport({
          context: { errorCode: `capture:${message.reason}`, exitCode: null, attempts: [] },
          url: message.url || null,
          extractor: null,
        });
    }
  }

  async function pull() {
    let messages: CaptureMessage[];
    try {
      messages = await invoke<CaptureMessage[]>('capture_pull', { after: lastSeq });
    } catch {
      return; // Not ready yet, or a test without the command.
    }
    // Pulls can overlap (wake, focus, timer): the seq alone says what is new.
    for (const message of messages) {
      if (message.seq <= lastSeq) continue;
      lastSeq = message.seq;
      handle(message);
    }
  }

  await listen('capture_wake', () => void pull());
  const timer = setInterval(() => void pull(), CAPTURE_PULL_MS);
  const onFocus = () => void pull();
  window.addEventListener('focus', onFocus);
  scope?.run(() => onScopeDispose(() => {
    clearInterval(timer);
    window.removeEventListener('focus', onFocus);
  }));
  await pull();
  await invoke('capture_ready');
}
