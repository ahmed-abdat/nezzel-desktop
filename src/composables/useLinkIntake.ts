import { computed, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import { useRouter } from 'vue-router';
import { useMediaStore } from '../stores/media/media';
import { useSettingsStore } from '../stores/settings';
import { useWatchClipboardStore } from '../stores/watchClipboard';
import { useToastStore } from '../stores/toast';
import { useClipboard } from './useClipboard';
import { isValidUrl } from '../helpers/url';
import {
  getUrlImportReadErrorToast,
  getUrlImportToast,
  getUrlImportUnsupportedToast,
  BIG_IMPORT_BYTES,
  isSupportedImportFile,
  parseUrlFileText,
  parseUrlInputText,
} from '../helpers/urlImport';
import { isInputFiltersActive } from '../helpers/inputFilters';
import { useDownloadSheetStore } from '../stores/downloadSheet';
import { clearSearch } from './useDownloadList';
import { useUiPrefs } from './useUiPrefs';
import { countUsage } from '../tauri/usage';

/**
 * Everything that brings a link into Nezzel: the link field, pasting, a file of links and the
 * copied-link watcher. One link from the field opens the options sheet (unless the user chose to
 * skip it); several links, or Shift, download straight away with the default choice.
 */
export function useLinkIntake() {
  const { t } = useI18n();
  const router = useRouter();
  const mediaStore = useMediaStore();
  const toastStore = useToastStore();
  const settingsStore = useSettingsStore();
  const watchClipboardStore = useWatchClipboardStore();
  const sheetStore = useDownloadSheetStore();
  const { skipOptionsSheet } = useUiPrefs();

  const doPolling = computed(() => settingsStore.settings.input.autoFillClipboard);
  const hasActiveInputFilters = computed(() => isInputFiltersActive(settingsStore.settings.inputFilters));
  const { content: clipboardContent } = useClipboard({ doPolling });

  const url = ref('');
  const clipboardHasValidUrl = computed(() => isValidUrl(clipboardContent));

  const placeholder = computed(() => {
    if (clipboardHasValidUrl.value && clipboardContent.value) return clipboardContent.value;
    return t('downloads.field.placeholder');
  });

  const canSubmit = computed(() => url.value.trim().length > 0 || clipboardHasValidUrl.value);

  async function processParsedUrls(
    result: { urls: string[]; skipped: number },
    immediateDownload = false,
    fromInput = false,
  ) {
    if (result.urls.length > 0) {
      if (fromInput) countUsage('paste');
      clearSearch();
      if (immediateDownload) {
        await mediaStore.addUrlBatchAndDownload(result.urls, false, true);
      } else {
        const ids = await mediaStore.addUrlBatch(result.urls);
        if (fromInput && ids.length === 1 && ids[0]) {
          sheetStore.show(ids[0]);
        }
      }
    }

    if (!fromInput || result.urls.length > 1) {
      const toast = getUrlImportToast(result);
      toastStore.showToast(toast.message, { style: toast.style });
    }
  }

  function submit(immediateDownload = false) {
    const hasDraft = url.value.trim().length > 0;
    const text = hasDraft ? url.value : clipboardContent.value;
    if (!text) return;
    if (!hasDraft && mediaStore.hasRecentCapture(text)) return;
    const parsed = parseUrlInputText(text);
    if (parsed.urls.length === 0) {
      toastStore.showToast(t('feedback.invalidLink'), { style: 'warning' });
      return;
    }
    void processParsedUrls(parsed, immediateDownload || skipOptionsSheet.value, true).catch(() => {
      toastStore.showToast(t('feedback.addFailed'), { style: 'error' });
    });
    void router.push({ name: 'home' });
    url.value = '';
  }

  async function importFile(file: File | undefined, immediateDownload = false) {
    if (!file) return;
    if (!isSupportedImportFile(file)) {
      const toast = getUrlImportUnsupportedToast();
      toastStore.showToast(toast.message, { style: toast.style });
      return;
    }
    if (file.size > BIG_IMPORT_BYTES) toastStore.showToast(t('common.urlImport.reading'), { style: 'info' });
    try {
      const text = await file.text();
      await processParsedUrls(parseUrlFileText(text), immediateDownload);
    } catch {
      const toast = getUrlImportReadErrorToast();
      toastStore.showToast(toast.message, { style: toast.style });
    }
  }

  return {
    url,
    placeholder,
    canSubmit,
    submit,
    importFile,
    hasActiveInputFilters,
    watchClipboard: watchClipboardStore,
  };
}
