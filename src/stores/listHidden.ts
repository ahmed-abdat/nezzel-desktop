import { defineStore } from 'pinia';
import { ref } from 'vue';

/** Rows hidden while an undo toast is open: gone from the list, not yet removed from the engine. */
export const useListHiddenStore = defineStore('list-hidden', () => {
  const ids = ref<Set<string>>(new Set());
  const hide = (groupIds: string[]) => {
    ids.value = new Set([...ids.value, ...groupIds]);
  };
  const show = (groupIds: string[]) => {
    const next = new Set(ids.value);
    for (const id of groupIds) next.delete(id);
    ids.value = next;
  };
  const isHidden = (groupId: string) => ids.value.has(groupId);
  return { ids, hide, show, isHidden };
});
