import { computed, ref } from 'vue';
import { pruneIds, rangeIds, selectAllIds, toggleId } from '../helpers/selection';

/** One list on screen at a time, so selection is shared by the rows, the bar and the keyboard handlers. */
const selected = ref<Set<string>>(new Set());
const anchor = ref<string | null>(null);
/** Visible row ids in display order; HomeView keeps this current. */
const order = ref<string[]>([]);

export function useDownloadSelection() {
  const count = computed(() => selected.value.size);
  const total = computed(() => order.value.length);
  const isSelected = (id: string) => selected.value.has(id);

  function toggle(id: string) {
    selected.value = toggleId(selected.value, id);
    anchor.value = id;
  }
  function extend(id: string) {
    selected.value = rangeIds(order.value, anchor.value, id);
    anchor.value ??= id;
  }
  function selectAll() {
    selected.value = selectAllIds(order.value);
  }
  function clear() {
    selected.value = new Set();
    anchor.value = null;
  }
  /** Called whenever the visible rows change. */
  function setOrder(ids: string[]) {
    order.value = ids;
    const next = pruneIds(selected.value, ids);
    if (next.size !== selected.value.size) selected.value = next;
    if (anchor.value && !ids.includes(anchor.value)) anchor.value = null;
  }

  return { selected, count, total, isSelected, toggle, extend, selectAll, clear, setOrder };
}
