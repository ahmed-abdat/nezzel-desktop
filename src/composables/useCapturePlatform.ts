import { reactive, readonly } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import type { CaptureSession, LinuxDesktop } from '@/generated/contract';

/** Desktop session facts the capture copy depends on (see the backend's CapturePlatform). */
export interface CapturePlatform {
  session: CaptureSession;
  /** A copy in another app raises the card while Nezzel is in the background. False on GNOME Wayland. */
  backgroundClipboard: boolean;
  /** Whose tray words the copy uses (Linux). */
  desktop: LinuxDesktop;
}

// Optimistic until the backend answers: every desktop but GNOME-style Wayland can.
const state = reactive<CapturePlatform>({ session: 'other', backgroundClipboard: true, desktop: 'other' });
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
