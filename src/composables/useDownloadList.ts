import { computed, ref } from 'vue';
import { useDownloadBuckets } from './useDownloadBuckets';
import { useLoginStore } from '../stores/login';
import { useMediaKind } from './useMediaKind';
import { useListVisibilityStore } from '../stores/listVisibility';
import { useMediaDestinationStore } from '../stores/media/destination';
import { groupSite, kindFilterOf } from '../helpers/groupView';
import { needsAttention, countView, filterRows, statusOf, type ListRow, type ListView, type TypeFilter } from '../helpers/listFilter';
import type { Group } from '../tauri/types/group';

export type GroupRow = ListRow & { id: string; group: Group; needsAttention: boolean };

/** Search text is shared by the field and the list (one list on screen at a time). */
export const downloadSearchQuery = ref('');
const query = downloadSearchQuery;

/** Empties the search field, e.g. when a new link arrives so the new row is not filtered out. */
export const clearSearch = () => {
  downloadSearchQuery.value = '';
};

/** Downloads list as rows the sidebar filters and the search field work on. Removing rows lives in useListRemoval. */
export function useDownloadList() {
  const { withState } = useDownloadBuckets();
  const { kindOf } = useMediaKind();
  const loginStore = useLoginStore();
  const cleared = useListVisibilityStore();
  const destinationStore = useMediaDestinationStore();

  const pathsOf = (id: string) => destinationStore.getDestinations(id).map(d => d.path);
  const rows = computed<GroupRow[]>(() => withState.value.map(({ group, state }) => ({
    id: group.id,
    group,
    status: statusOf(state),
    needsAttention: needsAttention(state, loginStore.cardFor(group.id)?.kind === 'needed'),
    cleared: statusOf(state) === 'done' && cleared.isCleared(pathsOf(group.id)),
    kind: kindFilterOf(kindOf(group)),
    title: group.title ?? group.url ?? '',
    site: groupSite(group) ?? '',
  })));

  const counts = (type: TypeFilter) => countView(rows.value, type);
  const shown = (view: ListView) => filterRows(rows.value, view, query.value);

  return { rows, counts, shown, query };
}
