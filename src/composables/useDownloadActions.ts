import { computed, reactive, type Ref } from 'vue';
import { useI18n } from 'vue-i18n';
import { invoke } from '@tauri-apps/api/core';
import { copyText } from '../tauri/clipboard';
import { confirm } from '@tauri-apps/plugin-dialog';
import { useMediaStore } from '../stores/media/media';
import { useMediaDiagnosticsStore } from '../stores/media/diagnostics';
import { useMediaOptionsStore } from '../stores/media/options';
import { useMediaDestinationStore } from '../stores/media/destination';
import { useToastStore } from '../stores/toast';
import { revealsInFolder } from '../helpers/fileCategory';
import { baseName } from '../helpers/folderPath';
import { hasVideo as groupHasVideo } from '../helpers/groupView';
import { defaultDownloadOptions, optionsForChoice } from '../helpers/quickChoice';
import { useOpener } from './useOpener';
import { countUsage } from '../tauri/usage';
import { useListRemoval } from './useListRemoval';
import { useUiPrefs } from './useUiPrefs';
import { currentPlatform } from './usePlatform';
import { Platform } from '../tauri/types/app';
import type { Group } from '../tauri/types/group';
import { isValidUrl } from '../helpers/url';

export type TrashResult = 'ok' | 'missing' | 'notOurs' | 'changed' | 'trashFailed';

/** Engine refusal → code; anything unknown is a plain failure. */
export function trashCode(error: unknown): Exclude<TrashResult, 'ok'> {
  return error === 'missing' || error === 'notOurs' || error === 'changed' ? error : 'trashFailed';
}

/**
 * Rows whose file the engine refused as not ours (saved before download receipts existed). Their Delete
 * can never work, so the row stops offering it for the rest of the session; Remove from list still does.
 */
const refusedNotOurs = reactive(new Set<string>());
export const isTrashRefused = (groupId: string) => refusedNotOurs.has(groupId);

/** Which name the OS gives its bin, for the confirm and result texts. */
export const trashBinKind = () => (currentPlatform() === Platform.Windows ? 'recycleBin' : 'trash');

/**
 * The one low-level trash operation (row Delete and batch Delete files share it): moves the saved file to
 * the OS Trash / Recycle Bin (never a hard delete) and drops the row. No prompt, no toast. A file that is
 * already gone counts as done (the row goes too). Otherwise the engine's refusal code comes back.
 */
export async function trashGroupFile(groupId: string, path: string | undefined): Promise<TrashResult> {
  if (!path) return 'notOurs';
  const mediaStore = useMediaStore();
  try {
    await invoke('media_trash_file', { groupId, path });
    mediaStore.deleteGroup(groupId);
    return 'ok';
  } catch (error) {
    const code = trashCode(error);
    if (code === 'missing') mediaStore.deleteGroup(groupId);
    if (code === 'notOurs') refusedNotOurs.add(groupId);
    return code;
  }
}

/** Start / pause / retry for any group, passed per call (no shared ref): what a batch bar runs over its rows. */
export function useGroupActions({ reportErrors = true }: { reportErrors?: boolean } = {}) {
  const { t } = useI18n();
  const mediaStore = useMediaStore();
  const optionsStore = useMediaOptionsStore();
  const toastStore = useToastStore();
  const { defaultChoice } = useUiPrefs();

  function reportError(key: string) {
    if (reportErrors) toastStore.showToast(t(key), { style: 'error' });
  }

  async function downloadGroup(group: Group) {
    const options = defaultDownloadOptions(group, optionsStore.getOptions(group.id), defaultChoice.value);
    optionsStore.setOptions(group.id, options);
    const ok = await mediaStore.downloadGroup(group.id, options);
    if (!ok) reportError('feedback.downloadFailed');
    return ok === true;
  }

  async function pauseGroup(group: Group) {
    countUsage('pause');
    const ok = await mediaStore.pauseGroup(group.id);
    if (!ok) reportError('feedback.pauseFailed');
    return ok === true;
  }

  async function retryGroup(group: Group) {
    countUsage('retry');
    if (!group.url) {
      reportError('media.card.toasts.retryError');
      return false;
    }
    try {
      await mediaStore.retryGroup(group.id);
      return true;
    } catch {
      reportError('media.card.toasts.retryError');
      return false;
    }
  }

  return { downloadGroup, pauseGroup, retryGroup };
}

/** What a download row (or the sheet) can do to its group. Engine calls only, no UI. */
export function useDownloadActions(group: Ref<Group>, { reportErrors = true }: { reportErrors?: boolean } = {}) {
  const { t } = useI18n();
  const optionsStore = useMediaOptionsStore();
  const destinationStore = useMediaDestinationStore();
  const toastStore = useToastStore();
  const { openPath, revealPath, openUrl } = useOpener();
  const { removeRows } = useListRemoval();
  const { downloadGroup, pauseGroup, retryGroup } = useGroupActions({ reportErrors });

  const diagnostics = useMediaDiagnosticsStore();
  const noAudioTrack = computed(() => diagnostics.findLastDiagnosticByGroupId(group.value.id)?.code === 'noAudioTrack');
  const hasVideo = computed(() => groupHasVideo(group.value));
  /** The recovery is a different download (video instead of audio), not a retry: it gets its own icon. */
  const recoveryIsSwitch = computed(() => noAudioTrack.value && hasVideo.value);
  const recoveryLabel = computed(() => t(noAudioTrack.value ? (hasVideo.value ? 'downloads.row.downloadVideoInstead' : 'downloads.row.retryWithBest') : 'downloads.row.retry'));

  async function recover() {
    if (!noAudioTrack.value) return retry();
    optionsStore.setOptions(group.value.id, optionsForChoice('best', group.value.formats ?? [], group.value.mediaKind));
    return hasVideo.value ? download() : retry();
  }

  const download = () => downloadGroup(group.value);
  const pause = () => pauseGroup(group.value);

  function remove() {
    removeRows([group.value.id]);
  }

  function cancel() {
    const undo = useMediaStore().cancelDownload(group.value.id);
    if (!undo) return;
    toastStore.showToast(t('downloads.row.cancelled'), {
      style: 'info',
      duration: 5000,
      action: {
        label: t('downloads.list.undo'),
        run: () => void undo().then((ok) => {
          if (!ok && reportErrors) toastStore.showToast(t('feedback.downloadFailed'), { style: 'error' });
        }),
      },
    });
  }

  const retry = () => retryGroup(group.value);

  const destination = () => destinationStore.getPrimaryDestination(group.value.id)?.path;

  function fileMissing() {
    const id = group.value.id;
    toastStore.showToast(t('feedback.fileMissing'), {
      style: 'error',
      action: { label: t('downloads.selection.remove'), run: () => removeRows([id]) },
    });
  }

  async function openFile() {
    const path = destination();
    if (!path) {
      fileMissing();
      return;
    }
    try {
      if (group.value.mediaKind === 'file' && revealsInFolder(path)) await revealPath(path);
      else await openPath(path);
    } catch { fileMissing(); }
  }

  async function revealFile() {
    const path = destination();
    if (!path) {
      fileMissing();
      return;
    }
    try {
      await revealPath(path);
    } catch { fileMissing(); }
  }

  function openPage() {
    // Only web links; the row's URL came from user input or a clipboard watcher.
    const url = group.value.url;
    if (url && /^https?:\/\//i.test(url) && isValidUrl(url)) void openUrl(url);
  }

  /** Native write, so the clipboard watcher knows this copy came from us and doesn't offer it back. */
  async function copyLink() {
    const url = group.value.url;
    if (!url) return;
    try {
      await copyText(url);
      toastStore.showToast(t('downloads.row.linkCopied'), { style: 'success' });
    } catch {
      toastStore.showToast(t('downloads.row.copyFailed'), { style: 'error' });
    }
  }

  const trashFile = () => trashGroupFile(group.value.id, destination());

  /** Asks first, then trashes the file. A refusal says why and offers the folder, so the user can do it by hand. */
  async function deleteFile() {
    const path = destination();
    if (!path) return;
    const bin = trashBinKind();
    const name = baseName(path);
    const ok = await confirm(t(`downloads.row.deleteConfirm.body.${bin}`, { name }), {
      title: t('downloads.row.deleteConfirm.title'),
      kind: 'warning',
      okLabel: t(`downloads.row.deleteConfirm.ok.${bin}`),
      cancelLabel: t('downloads.row.deleteConfirm.cancel'),
    });
    if (!ok) return;
    const result = await trashFile();
    if (result === 'ok') toastStore.showToast(t(`downloads.row.deleted.${bin}`), { style: 'success' });
    else if (result === 'missing') toastStore.showToast(t('downloads.row.deleteMissing'), { style: 'info' });
    else if (result === 'notOurs') {
      // Deleting will never work for this row: offer the way out that does.
      const id = group.value.id;
      toastStore.showToast(t('downloads.row.deleteRefused.notOurs'), {
        style: 'error',
        action: { label: t('downloads.row.removeFromList'), run: () => removeRows([id]) },
      });
    } else {
      toastStore.showToast(t(`downloads.row.deleteRefused.${result}`), {
        style: 'error',
        action: { label: t('downloads.row.show'), run: () => void revealPath(path) },
      });
    }
  }

  const canTrash = computed(() => !isTrashRefused(group.value.id));

  return { download, pause, remove, cancel, retry, recover, recoveryLabel, recoveryIsSwitch, openFile, revealFile, openPage, copyLink, deleteFile, trashFile, canTrash };
}
