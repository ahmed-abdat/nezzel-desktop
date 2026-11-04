import { computed } from 'vue';
import { useMediaGroupStore } from '../stores/media/group';
import { MediaState, useMediaStateStore } from '../stores/media/state';
import { useListHiddenStore } from '../stores/listHidden';
import type { Group } from '../tauri/types/group';

/** Splits the queue into what still needs the user or the engine, and what is finished. */
export function useDownloadBuckets() {
  const groupStore = useMediaGroupStore();
  const stateStore = useMediaStateStore();
  const hidden = useListHiddenStore();

  const withState = computed(() => groupStore.orderedGroups.filter((group: Group) => !hidden.isHidden(group.id)).map((group: Group) => ({
    group,
    state: stateStore.getGroupState(group.id) ?? MediaState.fetching,
  })));

  const active = computed(() => withState.value.filter(entry => entry.state !== MediaState.done).map(entry => entry.group));
  const finished = computed(() => withState.value.filter(entry => entry.state === MediaState.done).map(entry => entry.group));
  const failed = computed(() => withState.value.filter(entry => entry.state === MediaState.error).map(entry => entry.group));
  const ready = computed(() => withState.value.filter(entry => entry.state === MediaState.configure).map(entry => entry.group));
  /** Transferring right now: the one number the sidebar badge and the status bar both show. */
  const downloading = computed(() => withState.value
    .filter(entry => entry.state === MediaState.downloading || entry.state === MediaState.downloadingList)
    .map(entry => entry.group));

  return { active, finished, failed, ready, downloading, withState };
}
