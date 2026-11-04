import { ref, onMounted, onUnmounted, type Ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

export interface UseClipboardOptions { interval?: number; doPolling: Ref<boolean> }
export interface UseClipboardResponse { content: Ref<string | null>; poll: () => void }

/** Supported-URL changes come from the native watcher; never poll/log raw clipboard text in JS. */
export function useClipboard(options: UseClipboardOptions): UseClipboardResponse {
  const content = ref<string | null>(null);
  let unlisten: UnlistenFn | undefined;
  let disposed = false;
  watch(options.doPolling, (enabled) => {
    if (!enabled) content.value = null;
  });
  function poll() {
    if (!options.doPolling.value) return;
    void invoke<string>('capture_clipboard_current').then((text) => {
      if (!disposed && options.doPolling.value) content.value = text;
    }).catch(() => {});
  }
  watch(options.doPolling, (enabled) => {
    if (enabled) poll();
  });
  onMounted(async () => {
    const stop = await listen<string>('capture_clipboard', ({ payload }) => {
      content.value = options.doPolling.value ? payload : null;
    });
    if (disposed) stop();
    else unlisten = stop;
    poll();
  });
  onUnmounted(() => {
    disposed = true;
    unlisten?.();
  });
  return { content, poll };
}
