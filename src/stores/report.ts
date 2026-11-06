import { defineStore } from 'pinia';
import { ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { copyText } from '../tauri/clipboard';
import { i18n } from '../i18n';
import { useToastStore } from './toast';
import { usePreferencesStore } from './preferences';
import { useOnboardingStore } from './onboarding';
import { useSettingsStore } from './settings';
import router from '../router';
import { countUsage } from '../tauri/usage';
import type { SendError } from '../generated/contract';
import { fireAndForget } from '../tauri/fireAndForget';

/** Built and scrubbed in Rust (`diagnostics::report`). */
export interface ProblemReport {
  title: string;
  body: string;
}

/** A failed row's context: error code and stage trail. Rust reduces the link (`link_for_report`). */
export interface ReportDownloadContext {
  errorCode: string | null;
  exitCode: number | null;
  attempts: { stage: string; login: boolean; outcome: string; code: string | null; ms: number }[];
}

export interface ReportedDownload {
  context: ReportDownloadContext;
  url: string | null;
  extractor: string | null;
}

const t = (key: string) => i18n.global.t(key);

export interface CrashConsentInput {
  /** The welcome sheet (with the reports switch) was shown this launch. */
  sawReportsSwitch: boolean;
  /** Saved onboarding preferences; undefined when they did not load. */
  onboarding: { reportsNoticeDone?: boolean } | undefined;
  sendReports: boolean;
}

/**
 * The one consent decision at startup. Installs from before reports existed get a one-time notice first; until
 * the user has seen that reports are on (or with reports off), a previous run's crash is offered, never sent on
 * its own. Rust still checks the switch and the build's DSN before sending.
 */
export function crashConsent(input: CrashConsentInput): { showNotice: boolean; autoSend: boolean } {
  const showNotice = !input.sawReportsSwitch
    && input.onboarding !== undefined
    && !input.onboarding.reportsNoticeDone;
  // Unknown (preferences did not load): the user may never have seen the notice, so offer, don't send.
  const known = input.sawReportsSwitch || input.onboarding !== undefined;
  return { showNotice, autoSend: known && !showNotice && input.sendReports };
}

/** A download failed: Rust sends it when reports are on (site + link shape only). */
export function reportFailedDownload(download: ReportedDownload): void {
  // Never lets reporting break failure handling.
  fireAndForget('diagnostics_download_failed', { download: download.context, url: download.url, extractor: download.extractor });
}

export const useReportStore = defineStore('report', () => {
  const open = ref(false);
  const download = ref<ReportedDownload | null>(null);
  const sending = ref(false);
  const toast = useToastStore();

  function openReport(failed?: ReportedDownload) {
    countUsage('report.opened');
    download.value = failed ?? null;
    sending.value = false;
    open.value = true;
  }

  function close() {
    if (!sending.value) open.value = false;
  }

  function args(note: string, includeLink: boolean) {
    return {
      note: note.trim() || null,
      download: download.value?.context ?? null,
      url: download.value?.url ?? null,
      extractor: download.value?.extractor ?? null,
      includeLink,
    };
  }

  /** Copy the same scrubbed text, for when sending is not possible. */
  async function copyDetails(note: string, includeLink: boolean) {
    try {
      const report = await invoke<ProblemReport>('diagnostics_report', args(note, includeLink));
      await copyText(report.body);
      toast.showToast(t('report.copied'), { style: 'success' });
    } catch {
      toast.showToast(t('report.copyFailed'), { style: 'error' });
    }
  }

  async function send(note: string, includeLink: boolean) {
    sending.value = true;
    try {
      await invoke('diagnostics_send', args(note, includeLink));
      open.value = false;
      toast.showToast(t('report.sent'), { style: 'success' });
    } catch {
      toast.showToast(t('report.sendFailed'), {
        style: 'error',
        duration: 8000,
        action: { label: t('report.copyDetails'), run: () => void copyDetails(note, includeLink) },
      });
    } finally {
      sending.value = false;
    }
  }

  let started = false;
  /**
   * Once per launch, after the welcome sheet. Installs from before reports existed see a one-time
   * notice first, and a crash from that run is offered, not sent on its own.
   */
  async function startupChecks() {
    if (started) return;
    started = true;
    const preferences = usePreferencesStore();
    const { showNotice, autoSend } = crashConsent({
      sawReportsSwitch: useOnboardingStore().sawReportsSwitch,
      onboarding: preferences.preferences?.onboarding,
      sendReports: useSettingsStore().settings.privacy.sendReports,
    });
    if (showNotice) {
      toast.showToast(t('report.notice.text'), {
        style: 'info',
        duration: 0,
        action: { label: t('report.notice.action'), run: () => void router.push({ name: 'settings.advanced' }) },
      });
      void preferences.patch({ onboarding: { reportsNoticeDone: true } }).catch(() => {});
    }
    await checkCrash(autoSend);
  }

  /** The staged crash, on the user's click. Not delivered: it stays staged and the click is offered again. */
  function sendCrash() {
    void invoke('diagnostics_send_crash')
      .then(() => toast.showToast(t('report.sent'), { style: 'success' }))
      .catch((error: SendError | unknown) => toast.showToast(t('report.sendFailed'), {
        style: 'error',
        action: error === 'notDelivered' ? { label: t('report.sendCrash'), run: sendCrash } : undefined,
      }));
  }

  /**
   * Previous run's crash. Rust deletes the file once the crash is sent or staged for a click, so this shows once;
   * null also when it could not be staged (the file stays for the next launch).
   */
  async function checkCrash(auto = true) {
    let notice: { sent: boolean } | null = null;
    try {
      notice = await invoke<{ sent: boolean } | null>('diagnostics_crash_check', { auto });
    } catch {
      return;
    }
    if (!notice) return;
    if (notice.sent) {
      toast.showToast(t('report.crashSent'), { style: 'info', duration: 8000 });
      return;
    }
    toast.showToast(t('report.crashTitle'), {
      style: 'warning',
      duration: 0,
      action: { label: t('report.sendCrash'), run: sendCrash },
    });
  }

  return { open, download, sending, openReport, close, send, checkCrash, startupChecks };
});
