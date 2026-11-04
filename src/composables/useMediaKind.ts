import { computed } from 'vue';
import { useDownloadBuckets } from './useDownloadBuckets';
import { useMediaOptionsStore } from '../stores/media/options';
import { mediaKind } from '../helpers/mediaKind';
import type { Group } from '../tauri/types/group';

/** Downloads split by what the user gets (sidebar Types filter). */
export function useMediaKind() {
  const { active, finished } = useDownloadBuckets();
  const optionsStore = useMediaOptionsStore();
  const all = computed<Group[]>(() => [...active.value, ...finished.value]);
  const kindOf = (group: Group) => mediaKind(group.formats ?? [], optionsStore.getOptions(group.id), group.mediaKind);
  const video = computed(() => all.value.filter(group => kindOf(group) === 'video'));
  const audio = computed(() => all.value.filter(group => kindOf(group) === 'audio'));
  return { video, audio, kindOf };
}
