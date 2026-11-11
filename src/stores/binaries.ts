import { defineStore } from 'pinia';
import {
  BinaryDownloadCompletePayload,
  BinaryDownloadProgressPayload,
  BinaryDownloadStartPayload,
  BinaryProgress,
  BinaryCheckPayload, BinaryDownloadErrorPayload,
} from '../tauri/types/binaries';
import { computed, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { useToastStore } from './toast';
import { i18n } from '../i18n';
import { useSettingsStore } from './settings';

export const useBinariesStore = defineStore('binaries', () => {
  const tools = ref<Record<string, BinaryProgress>>({});
  const settingsStore = useSettingsStore();
  const toastStore = useToastStore();
  const repairFailed = ref(false);
  const lastError = ref('');
  const needsRepair = computed(() => repairFailed.value || Object.values(tools.value).some(tool => tool.error || tool.percent < 100));
  let updateStale = false;
  let updateNoticeShown = false;
  let updateToastId: number | undefined;

  function refreshUpdateNotice() {
    const visible = updateStale && navigator.onLine && settingsStore.settings.update.updateBinaries;
    if (!visible) {
      if (updateToastId !== undefined) toastStore.removeToast(updateToastId);
      updateToastId = undefined;
      if (!updateStale || !navigator.onLine) updateNoticeShown = false;
      return;
    }
    if (updateNoticeShown) return;
    updateNoticeShown = true;
    updateToastId = toastStore.showToast(i18n.global.t('toolsUpdate.stale'), {
      style: 'subtle', duration: 0,
      action: { label: i18n.global.t('toolsUpdate.tryNow'), run: () => {
        updateToastId = undefined;
        void invoke('binaries_update_now').catch(() => {
          toastStore.showToast(i18n.global.t('toolsUpdate.failed'), { style: 'subtle' });
        });
      } },
    });
  }

  function processUpdateStale(stale: boolean) {
    updateStale = stale;
    refreshUpdateNotice();
  }

  watch(() => settingsStore.settings.update.updateBinaries, refreshUpdateNotice);

  async function check(): Promise<string[]> {
    if (!settingsStore.settings.update.updateBinaries) return [];
    try {
      const result = await invoke<BinaryCheckPayload>('binaries_check');
      processUpdateStale(await invoke<boolean>('binaries_update_status'));
      tools.value = {};
      for (const tool of result.tools) {
        tools.value[tool] = { total: 0, percent: 0, received: 0 };
      }
      repairFailed.value = false;
      lastError.value = '';
      return result.tools;
    } catch (error) {
      repairFailed.value = true;
      lastError.value = String(error);
      throw error;
    }
  }

  async function ensure(toEnsure?: string[]) {
    try {
      if (!navigator.onLine) throw new Error(i18n.global.t('toolsRepair.offline'));
      await invoke<void>('binaries_ensure', { tools: toEnsure ?? Object.keys(tools.value) });
      repairFailed.value = false;
      lastError.value = '';
    } catch (error) {
      repairFailed.value = true;
      lastError.value = error instanceof Error ? error.message : String(error);
      throw error;
    }
  }

  function processBinaryDownloadStart(payload: BinaryDownloadStartPayload) {
    const tool = tools.value[payload.tool] ??= { total: 0, percent: 0, received: 0 };
    tool.version = payload.version;
  }

  function processBinaryDownloadProgress(payload: BinaryDownloadProgressPayload) {
    const tool = tools.value[payload.tool];
    if (!tool) return;
    tool.total = payload.total;
    tool.received = payload.received;
    tool.percent = Math.round(tool.received / tool.total * 100);
  }

  function processBinaryDownloadComplete(payload: BinaryDownloadCompletePayload) {
    const tool = tools.value[payload.tool];
    if (!tool) return;
    tool.received = tool.total;
    tool.percent = 100;
    delete tool.error;
  }

  function processBinaryDownloadError(payload: BinaryDownloadErrorPayload) {
    const tool = tools.value[payload.tool];
    if (!tool) return;
    tool.error = `[${payload.stage}] ${payload.error}`;
  }

  return { processUpdateStale, refreshUpdateNotice, tools, needsRepair, lastError, check, ensure, processBinaryDownloadStart, processBinaryDownloadProgress, processBinaryDownloadError, processBinaryDownloadComplete };
});
