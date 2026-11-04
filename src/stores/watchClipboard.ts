import { defineStore } from 'pinia';
import { computed } from 'vue';
import { useSettingsStore } from './settings';

/** The Rust watcher owns capture policy; this store only reflects its persistent switch. */
export const useWatchClipboardStore = defineStore('watch-clipboard', () => {
  const settings = useSettingsStore();
  const enabled = computed(() => settings.settings.input.watchClipboard);
  const isActive = enabled;
  function enable() {
    void settings.patchNow({ input: { watchClipboard: true } });
  }
  function disable() {
    void settings.patchNow({ input: { watchClipboard: false } });
  }
  function toggle() {
    if (enabled.value) disable();
    else enable();
  }
  return { enabled, isActive, enable, disable, toggle };
});
