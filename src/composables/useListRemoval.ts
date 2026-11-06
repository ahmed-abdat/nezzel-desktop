import { useI18n } from 'vue-i18n';
import { useDownloadList, type GroupRow } from './useDownloadList';
import { useMediaStore } from '../stores/media/media';
import { useListVisibilityStore } from '../stores/listVisibility';
import { useMediaDestinationStore } from '../stores/media/destination';
import { useToastStore } from '../stores/toast';
import { finishedIds } from '../helpers/listFilter';

const UNDO_MS = 5000;

/**
 * Taking rows off the list: remove / undo / "Remove from list". Depends on the list state (useDownloadList,
 * the visibility store); the list itself knows nothing about removal.
 */
export function useListRemoval() {
  const { t } = useI18n();
  const { rows } = useDownloadList();
  const mediaStore = useMediaStore();
  const visibility = useListVisibilityStore();
  const destinationStore = useMediaDestinationStore();
  const toastStore = useToastStore();

  const pathsOf = (id: string) => destinationStore.getDestinations(id).map(d => d.path);

  /** Takes finished rows off the main list. Files and history stay; the rows remain under Completed. */
  function clearFinished(from: GroupRow[] = rows.value) {
    removeRows(finishedIds(from), 'downloads.list.cleared');
  }

  /** Done rows with saved files only leave the main list; returns the ids it took. */
  function clearDone(ids: string[]): string[] {
    const withFiles = ids.filter(id => pathsOf(id).length > 0);
    const paths = withFiles.flatMap(pathsOf);
    if (withFiles.length === 0) return [];
    visibility.clear(paths);
    toastStore.showToast(t('downloads.list.cleared', { n: withFiles.length }, withFiles.length), {
      style: 'info',
      duration: UNDO_MS,
      action: { label: t('downloads.list.undo'), run: () => visibility.restore(paths) },
    });
    return withFiles;
  }

  /**
   * Finished rows with files only leave the main list (see clearFinished). Other rows, and finished
   * rows removed again from Completed (that forgets their history; files stay), are hidden at once
   * and removed for good when the undo window closes, so their work never stops before the user can undo.
   */
  function removeRows(groupIds: string[], message = 'feedback.removed') {
    const unique = [...new Set(groupIds)];
    const done = new Set(clearDone(finishedIds(rows.value.filter(row => unique.includes(row.id)))));
    const ids = unique.filter(id => !done.has(id) && !visibility.isHidden(id));
    if (ids.length === 0) return;
    visibility.hide(ids);
    let undone = false;
    setTimeout(() => {
      if (undone) return;
      for (const id of ids) {
        if (visibility.isHidden(id)) mediaStore.deleteGroup(id);
      }
      visibility.show(ids);
    }, UNDO_MS);
    toastStore.showToast(t(message, { n: ids.length }, ids.length), {
      style: 'info',
      duration: UNDO_MS,
      action: {
        label: t('downloads.list.undo'),
        run: () => {
          undone = true;
          visibility.show(ids);
        },
      },
    });
  }

  return { clearFinished, removeRows };
}
