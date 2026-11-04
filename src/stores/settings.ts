import { defineStore } from 'pinia';
import { ref, toRaw } from 'vue';
import { mergeInto, rollbackInto } from '../helpers/mergeInto';
import { useToastStore } from './toast';
import { invoke } from '@tauri-apps/api/core';
import { applyDocumentLocale, getDefaultLocale, i18n, type Locale } from '../i18n';
import { defaultSettings } from '../tauri/types/config.ts';
import { Settings } from '../tauri/types/config.ts';

type DeepPartial<T> = T extends object ? {
  [P in keyof T]?: DeepPartial<T[P]>;
} : T;

export const useSettingsStore = defineStore('settings', () => {
  const settings = ref<Settings>(defaultSettings);

  async function load(): Promise<Settings> {
    const cfg = await invoke<Settings>('config_get');
    applySettings(cfg);
    return cfg;
  }

  async function patch(partial: DeepPartial<Settings>) {
    const newCfg = await invoke<Settings>('config_set', { patch: partial });
    applySettings(newCfg);
  }

  /**
   * For switches and pickers: the UI flips now, the engine catches up. On failure the old value
   * comes back and a toast says so, so the control never lies about what is saved.
   */
  async function patchNow(partial: DeepPartial<Settings>): Promise<boolean> {
    const before = structuredClone(toRaw(settings.value));
    mergeInto(settings.value, partial);
    try {
      await patch(partial);
      return true;
    } catch {
      // Only this patch's fields: a concurrent patch that succeeded keeps its value.
      rollbackInto(settings.value, partial, before);
      useToastStore().showToast(i18n.global.t('feedback.settingFailed'), { style: 'error' });
      return false;
    }
  }

  async function reset() {
    const cfg = await invoke<Settings>('config_reset');
    applySettings(cfg);
    return cfg;
  }

  function hasAuthConfigured() {
    return settings.value.auth.cookieFile !== null || !['none', 'automatic'].includes(settings.value.auth.cookieBrowser);
  }

  function applySettings(cfg: Settings) {
    Object.assign(settings.value, cfg);

    if (cfg.appearance.language) {
      const currentLocale: Locale = cfg.appearance.language === 'system'
        ? getDefaultLocale()
        : cfg.appearance.language as Locale;

      i18n.global.locale.value = currentLocale;
      applyDocumentLocale(currentLocale);
    }
  }

  return {
    settings,
    load,
    patch,
    patchNow,
    reset,
    hasAuthConfigured,
  };
});
