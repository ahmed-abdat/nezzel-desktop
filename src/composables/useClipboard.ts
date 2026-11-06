import { ref, onMounted, onUnmounted, type Ref, watch } from 'vue';
import { readClipboardLink } from '../tauri/clipboard';
import { capturePrefill } from '../tauri/listeners/capture';

export interface UseClipboardOptions { interval?: number; doPolling: Ref<boolean> }
export interface UseClipboardResponse { content: Ref<string | null>; poll: () => void }

/** Supported-URL changes come from the native watcher (a `prefill` capture message); never poll/log raw clipboard text in JS. */
export function useClipboard(options: UseClipboardOptions): UseClipboardResponse {
  const content = ref<string | null>(null);
  let disposed = false;
  watch(options.doPolling, (enabled) => {
    if (!enabled) content.value = null;
  });
  function poll() {
    if (!options.doPolling.value) return;
    void readClipboardLink().then((text) => {
      if (!disposed && options.doPolling.value) content.value = text;
    });
  }
  watch(options.doPolling, (enabled) => {
    if (enabled) poll();
  });
  watch(capturePrefill, (url) => {
    if (url !== null) content.value = options.doPolling.value ? url : null;
  });
  onMounted(poll);
  onUnmounted(() => {
    disposed = true;
  });
  return { content, poll };
}
