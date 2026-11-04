import { defineStore } from 'pinia';
import { ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { UpdaterCheckPayload, UpdaterDownloadProgressPayload } from '../tauri/types/updater';
import { useSettingsStore } from './settings';

export const useUpdaterStore = defineStore('updater', () => {
  const checkResult = ref<UpdaterCheckPayload | undefined>();
  const downloadProgress = ref<UpdaterDownloadProgressPayload>({ received: 0, total: 0 });
  const isUpdating = ref(false);
  const isIgnored = ref(false);
  const isNeedingRestart = ref(false);
  const lastError = ref<string | undefined>();
  const lastOperation = ref<'download' | 'install'>('download');
  const settingsStore = useSettingsStore();

  /** Background check obeys the "update app" switch; a manual check (About) always runs and re-shows the card. */
  async function check(manual = false) {
    if (!manual && !settingsStore.settings.update.updateApp) return;
    checkResult.value = await invoke<UpdaterCheckPayload>('updater_check');
    if (manual && checkResult.value?.available) isIgnored.value = false;
    return checkResult.value;
  }

  async function download() {
    if (isUpdating.value) return;
    lastError.value = undefined;
    lastOperation.value = 'download';
    isIgnored.value = false;
    downloadProgress.value = { received: 0, total: 0 };
    try {
      isUpdating.value = true;
      await invoke('updater_download');
      isNeedingRestart.value = true;
    } catch (e) {
      processUpdaterError(String(e));
    } finally {
      isUpdating.value = false;
    }
  }

  async function install() {
    if (isUpdating.value) return;
    lastError.value = undefined;
    lastOperation.value = 'install';
    isUpdating.value = true;
    try {
      await invoke('updater_install');
      isNeedingRestart.value = false;
      processUpdaterFinished();
    } catch (e) {
      isNeedingRestart.value = true;
      processUpdaterError(String(e));
    } finally { isUpdating.value = false; }
  }

  async function retry() {
    const installing = lastOperation.value === 'install';
    // The native installer consumes its cached bytes even on failure; fetch a fresh copy first.
    await download();
    if (installing && !lastError.value && isNeedingRestart.value) await install();
  }

  function ignore() {
    isIgnored.value = true;
  }

  function processUpdaterDownloadProgress(payload: UpdaterDownloadProgressPayload) {
    Object.assign(downloadProgress.value, payload);
  }

  function processUpdaterFinished() {
    isUpdating.value = false;
    isIgnored.value = true;
    downloadProgress.value = { received: 0, total: 0 };
  }

  function processUpdaterError(payload: string) {
    isUpdating.value = false;
    isIgnored.value = false;
    downloadProgress.value = { received: 0, total: 0 };
    lastError.value = payload;
  }

  return { checkResult, downloadProgress, isUpdating, isIgnored, isNeedingRestart, lastError, lastOperation, retry, ignore, check, download, install, processUpdaterDownloadProgress, processUpdaterFinished, processUpdaterError };
});
