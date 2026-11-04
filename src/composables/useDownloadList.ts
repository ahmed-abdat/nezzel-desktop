import { computed, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import { useDownloadBuckets } from './useDownloadBuckets';
import { useMediaKind } from './useMediaKind';
import { useMediaStore } from '../stores/media/media';
import { useListHiddenStore } from '../stores/listHidden';
import { useListClearedStore } from '../stores/listCleared';
import { useMediaDestinationStore } from '../stores/media/destination';
import { useToastStore } from '../stores/toast';
import { siteName } from '../helpers/siteName';
import { countView, filterRows, finishedIds, kindFilterOf, statusOf, type ListRow, type ListView, type TypeFilter } from '../helpers/listFilter';
import type { Group } from '../tauri/types/group';

export type GroupRow = ListRow & { id: string; group: Group };

/** Search text is shared by the field and the list (one list on screen at a time). */
export const downloadSearchQuery = ref('');
const query = downloadSearchQuery;

const UNDO_MS = 5000;

/** Downloads list as rows the sidebar filters, the search field and "Clear finished" work on. */
export function useDownloadList() {
  const { t } = useI18n();
  const { withState } = useDownloadBuckets();
  const { kindOf } = useMediaKind();
  const mediaStore = useMediaStore();
  const hidden = useListHiddenStore();
  const cleared = useListClearedStore();
  const destinationStore = useMediaDestinationStore();
  const toastStore = useToastStore();

  const pathsOf = (id: string) => destinationStore.getDestinations(id).map(d => d.path);
  const rows = computed<GroupRow[]>(() => withState.value.map(({ group, state }) => ({
    id: group.id,
    group,
    status: statusOf(state),
    cleared: statusOf(state) === 'done' && cleared.isCleared(pathsOf(group.id)),
    kind: kindFilterOf(kindOf(group)),
    title: group.title ?? group.url ?? '',
    site: siteName(group.extractor ?? group.items[Object.keys(group.items)[0]]?.extractor, group.url) ?? '',
  })));

  const counts = (type: TypeFilter) => countView(rows.value, type);
  const shown = (view: ListView) => filterRows(rows.value, view, query.value);

  /** Takes finished rows off the main list. Files and history stay; the rows remain under Completed. */
  function clearFinished(from: GroupRow[] = rows.value) {
    removeRows(finishedIds(from), 'downloads.list.cleared');
  }

  /** Done rows with saved files only leave the main list; returns the ids it took. */
  function clearDone(ids: string[]): string[] {
    const withFiles = ids.filter(id => pathsOf(id).length > 0);
    const paths = withFiles.flatMap(pathsOf);
    if (withFiles.length === 0) return [];
    cleared.clear(paths);
    toastStore.showToast(t('downloads.list.cleared', { n: withFiles.length }, withFiles.length), {
      style: 'info',
      duration: UNDO_MS,
      action: { label: t('downloads.list.undo'), run: () => cleared.restore(paths) },
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
    const ids = unique.filter(id => !done.has(id) && !hidden.isHidden(id));
    if (ids.length === 0) return;
    hidden.hide(ids);
    let undone = false;
    setTimeout(() => {
      if (undone) return;
      for (const id of ids) {
        if (hidden.isHidden(id)) mediaStore.deleteGroup(id);
      }
      hidden.show(ids);
    }, UNDO_MS);
    toastStore.showToast(t(message, { n: ids.length }, ids.length), {
      style: 'info',
      duration: UNDO_MS,
      action: {
        label: t('downloads.list.undo'),
        run: () => {
          undone = true;
          hidden.show(ids);
        },
      },
    });
  }

  return { rows, counts, shown, query, clearFinished, removeRows };
}
