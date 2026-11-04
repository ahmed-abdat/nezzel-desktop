import { reactive, readonly } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

/** Quick download shortcut as the OS took it (see the backend's ShortcutStatus). */
export interface ShortcutStatus {
  /** What the user presses now: the fallback when the default was taken. Null before the backend answers. */
  active: string | null;
  /** Nothing could be registered. */
  failed: boolean;
  /** Why it cannot work in this session: Wayland never delivers global key grabs to apps. */
  reason?: 'wayland';
}

// One listener per window. `seq` counts registrations reported after start, so a recorder can match its change.
const state = reactive({ active: null as string | null, failed: false, reason: undefined as ShortcutStatus['reason'], seq: 0, startupFailed: false });
let started = false;

function start() {
  if (started) return;
  started = true;
  listen<ShortcutStatus>('capture_shortcut_status', ({ payload }) => {
    state.active = payload.active;
    state.failed = payload.failed;
    state.reason = payload.reason;
    state.seq += 1;
  })
    .then(() => invoke<ShortcutStatus>('capture_shortcut_status'))
    .then((status) => {
      if (state.seq > 0 || !status) return;
      state.active = status.active;
      state.failed = status.failed;
      state.reason = status.reason;
      state.startupFailed = status.failed;
    })
    .catch(() => { /* not running inside Tauri */ });
}

export function useCaptureShortcut() {
  start();
  return readonly(state);
}
