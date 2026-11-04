import { computed, reactive, type Ref } from 'vue';
import { useI18n } from 'vue-i18n';
import { invoke } from '@tauri-apps/api/core';
import { confirm } from '@tauri-apps/plugin-dialog';
import { useMediaStore } from '../stores/media/media';
import { useMediaDiagnosticsStore } from '../stores/media/diagnostics';
import { useMediaOptionsStore } from '../stores/media/options';
import { useMediaDestinationStore } from '../stores/media/destination';
import { useToastStore } from '../stores/toast';
import { revealsInFolder } from '../helpers/fileCategory';
import { hasAudio, optionsForChoice } from '../helpers/quickChoice';
import { useOpener } from './useOpener';
import { useDownloadList } from './useDownloadList';
import { useUiPrefs } from './useUiPrefs';
import type { Group } from '../tauri/types/group';

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

/** What a download row (or the sheet) can do to its group. Engine calls only, no UI. */
export function useDownloadActions(group: Ref<Group>, { reportErrors = true }: { reportErrors?: boolean } = {}) {
  const { t } = useI18n();
  const mediaStore = useMediaStore();
  const optionsStore = useMediaOptionsStore();
  const destinationStore = useMediaDestinationStore();
  const toastStore = useToastStore();
  const { openPath, revealPath, openUrl } = useOpener();
  const { removeRows } = useDownloadList();
  const { defaultChoice } = useUiPrefs();

  function reportError(key: string) {
    if (reportErrors) toastStore.showToast(t(key), { style: 'error' });
  }

  const diagnostics = useMediaDiagnosticsStore();
  const noAudioTrack = computed(() => diagnostics.findLastDiagnosticByGroupId(group.value.id)?.code === 'noAudioTrack');
  const hasVideo = computed(() => group.value.mediaKind === 'video' || group.value.formats?.some(f => f.height || f.videoCodecs.some(c => !['', 'none', 'unknown'].includes((typeof c === 'string' ? c : c.id).toLowerCase()))));
  /** The recovery is a different download (video instead of audio), not a retry: it gets its own icon. */
  const recoveryIsSwitch = computed(() => noAudioTrack.value && hasVideo.value);
  const recoveryLabel = computed(() => t(noAudioTrack.value ? (hasVideo.value ? 'downloads.row.downloadVideoInstead' : 'downloads.row.retryWithBest') : 'downloads.row.retry'));

  async function recover() {
    if (!noAudioTrack.value) return retry();
    optionsStore.setOptions(group.value.id, optionsForChoice('best', group.value.formats ?? [], group.value.mediaKind));
    return hasVideo.value ? download() : retry();
  }

  async function download() {
    let options = optionsStore.getOptions(group.value.id);
    if (!options || (options.trackType === 'audio' && !hasAudio(group.value))) {
      options = optionsForChoice(defaultChoice.value, group.value.formats ?? [], group.value.mediaKind);
      optionsStore.setOptions(group.value.id, options);
    }
    const ok = await mediaStore.downloadGroup(group.value.id, options);
    if (!ok) reportError('feedback.downloadFailed');
    return ok === true;
  }

  async function pause() {
    const ok = await mediaStore.pauseGroup(group.value.id);
    if (!ok) reportError('feedback.pauseFailed');
    return ok === true;
  }

  function remove() {
    removeRows([group.value.id]);
  }

  async function retry() {
    const url = group.value.url;
    if (!url) {
      reportError('media.card.toasts.retryError');
      return false;
    }
    try {
      await mediaStore.retryGroup(group.value.id);
      return true;
    } catch {
      reportError('media.card.toasts.retryError');
      return false;
    }
  }

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
    if (group.value.url) void openUrl(group.value.url);
  }

  /** Native write, so the clipboard watcher knows this copy came from us and doesn't offer it back. */
  async function copyLink() {
    const url = group.value.url;
    if (!url) return;
    try {
      await invoke('capture_write_clipboard', { text: url });
      toastStore.showToast(t('downloads.row.linkCopied'), { style: 'success' });
    } catch {
      toastStore.showToast(t('downloads.row.copyFailed'), { style: 'error' });
    }
  }

  /**
   * Moves the saved file to the OS Trash / Recycle Bin (never a hard delete) and drops the row. No prompt, no toast.
   * A file that is already gone counts as done (the row goes too). Otherwise the engine's refusal code comes back.
   */
  async function trashFile(): Promise<TrashResult> {
    const path = destination();
    const id = group.value.id;
    if (!path) return 'notOurs';
    try {
      await invoke('media_trash_file', { groupId: id, path });
      mediaStore.deleteGroup(id);
      return 'ok';
    } catch (error) {
      const code = trashCode(error);
      if (code === 'missing') mediaStore.deleteGroup(id);
      if (code === 'notOurs') refusedNotOurs.add(id);
      return code;
    }
  }

  /** Asks first, then trashes the file. A refusal says why and offers the folder, so the user can do it by hand. */
  async function deleteFile() {
    const path = destination();
    if (!path) return;
    const bin = /windows/i.test(navigator.userAgent) ? 'recycleBin' : 'trash';
    const name = path.split(/[\\/]/).pop() ?? path;
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

  return { download, pause, remove, retry, recover, recoveryLabel, recoveryIsSwitch, openFile, revealFile, openPage, copyLink, deleteFile, trashFile, canTrash };
}
