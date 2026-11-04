import { computed, ref } from 'vue';
import { Platform } from '../tauri/types/app.ts';
import { invoke } from '@tauri-apps/api/core';

/**
 * The OS the webview reports, so shortcut keycaps are right on first paint. WKWebView says "Macintosh",
 * WebView2 "Windows", WebKitGTK "Linux"/"X11". The backend's answer (get_platform) confirms it.
 */
export function platformFromUserAgent(userAgent: string): Platform {
  if (/Mac/i.test(userAgent)) return Platform.MacOS;
  if (/Win/i.test(userAgent)) return Platform.Windows;
  if (/Linux|X11/i.test(userAgent)) return Platform.Linux;
  return Platform.Unknown;
}

// One answer per window: every component used to start at Unknown (PC keycaps on a Mac) until its own
// get_platform reply arrived, and kept Unknown for good when that call failed.
const platform = ref<Platform>(typeof navigator === 'undefined' ? Platform.Unknown : platformFromUserAgent(navigator.userAgent));
let asked = false;

function askBackend() {
  if (asked) return;
  asked = true;
  invoke<Platform>('get_platform')
    .then((value) => {
      if (value && value !== Platform.Unknown) platform.value = value;
    })
    .catch(e => console.warn('Unable to retrieve platform from Rust.', e));
}

export function usePlatform() {
  askBackend();
  const isWindows = computed(() => platform.value === Platform.Windows);
  const isMac = computed(() => platform.value === Platform.MacOS);
  return { isWindows, isMac, platform };
}

/** Tests only: forget the backend answer so the next usePlatform() asks again. */
export function resetPlatformForTests(value?: Platform) {
  asked = false;
  platform.value = value ?? (typeof navigator === 'undefined' ? Platform.Unknown : platformFromUserAgent(navigator.userAgent));
}
