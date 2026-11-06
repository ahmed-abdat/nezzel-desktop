import { computed } from 'vue';
import { useMediaGroupStore } from '../stores/media/group';
import { MediaState, useMediaStateStore } from '../stores/media/state';
import { useListVisibilityStore } from '../stores/listVisibility';
import { phaseOf, type Phase } from '../helpers/groupView';
import type { Group } from '../tauri/types/group';

/** Splits the queue into what still needs the user or the engine, and what is finished. */
export function useDownloadBuckets() {
  const groupStore = useMediaGroupStore();
  const stateStore = useMediaStateStore();
  const hidden = useListVisibilityStore();

  const withState = computed(() => groupStore.orderedGroups.filter((group: Group) => !hidden.isHidden(group.id)).map((group: Group) => ({
    group,
    state: stateStore.getGroupState(group.id) ?? MediaState.fetching,
  })));

  const inPhase = (phase: Phase) => computed(() => withState.value.filter(entry => phaseOf(entry.state) === phase).map(entry => entry.group));

  const active = computed(() => withState.value.filter(entry => phaseOf(entry.state) !== 'done').map(entry => entry.group));
  const finished = inPhase('done');
  const failed = inPhase('error');
  const ready = inPhase('ready');
  /** Reading the link (single or list): the status bar says "Reading…". */
  const reading = inPhase('reading');
  /** Transferring right now: the one number the sidebar badge and the status bar both show. */
  const downloading = inPhase('downloading');

  return { active, finished, failed, ready, reading, downloading, withState };
}
