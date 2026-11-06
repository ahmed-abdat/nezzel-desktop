import { defineStore } from 'pinia';
import { ref, toRaw } from 'vue';
import { mergeInto, rollbackInto } from '../helpers/mergeInto';
import { useToastStore } from './toast';
import { invoke } from '@tauri-apps/api/core';
import { i18n } from '../i18n';
import { applyLanguage } from '../appearance';
import { defaultSettings } from '../tauri/types/config.ts';
import { Settings, type PrivacySettings } from '../tauri/types/config.ts';
import { COOKIE_BROWSER_AUTOMATIC, COOKIE_BROWSER_NEVER } from '../generated/contract';

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

  /**
   * The consent switches (Settings and the welcome sheet): the switch flips now and the value is saved at once,
   * not after a page's save delay, so "off" holds even if the app closes next. Resolves false when it could not
   * be saved: the switch is back and a toast said so.
   */
  function setPrivacy(key: keyof PrivacySettings, on: boolean): Promise<boolean> {
    return patchNow({ privacy: { [key]: on } });
  }

  async function reset() {
    const cfg = await invoke<Settings>('config_reset');
    applySettings(cfg);
    return cfg;
  }

  function hasAuthConfigured() {
    const browser = settings.value.auth.cookieBrowser;
    return settings.value.auth.cookieFile !== null || (browser !== COOKIE_BROWSER_NEVER && browser !== COOKIE_BROWSER_AUTOMATIC);
  }

  function applySettings(cfg: Settings) {
    Object.assign(settings.value, cfg);

    if (cfg.appearance.language) {
      applyLanguage(cfg.appearance.language);
    }
  }

  return {
    settings,
    load,
    patch,
    patchNow,
    setPrivacy,
    reset,
    hasAuthConfigured,
  };
});
