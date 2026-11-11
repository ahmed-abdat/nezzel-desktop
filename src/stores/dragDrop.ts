import { ref } from 'vue';
import { defineStore } from 'pinia';
import { clearSearch } from '../composables/useDownloadList';
import { useToastStore } from './toast.ts';
import { useMediaStore } from './media/media.ts';
import { i18n } from '../i18n';
import {
  BIG_IMPORT_BYTES,
  getUrlImportReadErrorToast,
  getUrlImportToast,
  getUrlImportUnsupportedToast,
  isSupportedImportFile,
  mergeParsedUrlImports,
  parseUrlFileText,
  parseUrlInputText,
} from '../helpers/urlImport.ts';

type DropPayload = {
  urls: string[];
  files: File[];
};

export const useDragDropStore = defineStore('dragDrop', () => {
  const isOver = ref(false);

  function setOver() {
    isOver.value = true;
  }

  function reset() {
    isOver.value = false;
  }

  async function handleDrop(payload: DropPayload) {
    isOver.value = false;

    const toastStore = useToastStore();
    const mediaStore = useMediaStore();

    const supportedFiles = payload.files.filter(isSupportedImportFile);
    const unsupported = payload.files.length - supportedFiles.length;
    if (supportedFiles.some(file => file.size > BIG_IMPORT_BYTES)) toastStore.showToast(i18n.global.t('common.urlImport.reading'), { style: 'info' });
    // One unreadable file must not throw away the links from the rest of the drop.
    const parsedFiles = [];
    let unreadable = 0;
    for (const file of supportedFiles) {
      try {
        parsedFiles.push(parseUrlFileText(await file.text()));
      } catch {
        unreadable += 1;
      }
    }

    const result = mergeParsedUrlImports([
      parseUrlInputText(payload.urls.join('\n')),
      ...parsedFiles,
    ]);

    let addedIds: string[] = [];
    if (result.urls.length > 0) {
      clearSearch();
      addedIds = await mediaStore.addUrlBatch(result.urls);
    } else if (unreadable || unsupported) {
      const toast = unreadable ? getUrlImportReadErrorToast() : getUrlImportUnsupportedToast();
      toastStore.showToast(toast.message, { style: toast.style });
      return;
    }

    const toast = getUrlImportToast(result);
    toastStore.showToast(toast.message, {
      style: toast.style,
      ...(addedIds.length > 1
        ? {
            duration: 8000,
            action: { label: i18n.global.t('downloads.summary.downloadAll'), run: async () => {
              const results = await Promise.allSettled(addedIds.map(id => mediaStore.downloadPreparedGroup(id)));
              if (results.some(result => result.status === 'rejected')) {
                toastStore.showToast(i18n.global.t('feedback.downloadFailed'), { style: 'error' });
              }
              const playlists = results.filter(result => result.status === 'fulfilled' && result.value === 'playlist').length;
              if (playlists > 0) {
                toastStore.showToast(i18n.global.t('downloads.summary.playlistsNeedSelection', { n: playlists }, playlists), { style: 'info' });
              }
            } },
          }
        : {}),
    });
  }

  return {
    isOver,
    setOver,
    reset,
    handleDrop,
  };
});
