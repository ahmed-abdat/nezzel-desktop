import { reactive, readonly } from 'vue';
import { invoke } from '@tauri-apps/api/core';

/** Desktop session facts the capture copy depends on (see the backend's CapturePlatform). */
export interface CapturePlatform {
  session: 'x11' | 'wayland' | 'other';
  /** A copy in another app raises the card while Nezzel is in the background. False on GNOME Wayland. */
  backgroundClipboard: boolean;
}

// Optimistic until the backend answers: every desktop but GNOME-style Wayland can.
const state = reactive<CapturePlatform>({ session: 'other', backgroundClipboard: true });
let asked = false;

export function useCapturePlatform() {
  if (!asked) {
    asked = true;
    invoke<CapturePlatform>('capture_platform')
      .then((value) => {
        if (value) Object.assign(state, value);
      })
      .catch(() => { /* not running inside Tauri */ });
  }
  return readonly(state);
}
