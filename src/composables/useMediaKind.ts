import { useMediaOptionsStore } from '../stores/media/options';
import { mediaKind } from '../helpers/groupView';
import type { Group } from '../tauri/types/group';

/** What the user gets from a download (sidebar Types filter): its kind, given the options chosen for it. */
export function useMediaKind() {
  const optionsStore = useMediaOptionsStore();
  const kindOf = (group: Group) => mediaKind(group.formats ?? [], optionsStore.getOptions(group.id), group.mediaKind);
  return { kindOf };
}
