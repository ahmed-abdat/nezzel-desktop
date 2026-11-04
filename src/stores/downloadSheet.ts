import { defineStore } from 'pinia';
import { ref, watch } from 'vue';
import { useMediaGroupStore } from './media/group';
import { MediaState, useMediaStateStore } from './media/state';
import { useMediaStore } from './media/media';
import { useMediaOptionsStore } from './media/options';
import { hasRealChoice, optionsForChoice } from '../helpers/quickChoice';

/** Which download the options sheet is showing. One sheet at a time. */
export const useDownloadSheetStore = defineStore('download-sheet', () => {
  const groupId = ref<string | null>(null);
  const open = ref(false);
  /** The user changed something in the sheet; it then stays open whatever the link turns out to be. */
  const touched = ref(false);
  const skippable = ref(false);

  const groupStore = useMediaGroupStore();
  const stateStore = useMediaStateStore();

  function show(id: string) {
    groupId.value = id;
    open.value = true;
    touched.value = false;
    // Only a sheet opened while the link is still being read may skip itself; reopening a ready row never downloads.
    const state = stateStore.getGroupState(id);
    skippable.value = state === MediaState.fetching || state === MediaState.fetchingList;
  }

  function close() {
    open.value = false;
  }

  function touch() {
    touched.value = true;
  }

  // Nothing to choose (photo, single format): once the link is read, download with the best choice.
  watch(
    () => (open.value && groupId.value ? stateStore.getGroupState(groupId.value) : undefined),
    (state) => {
      const id = groupId.value;
      if (state !== MediaState.configure || !id || touched.value || !skippable.value) return;
      const group = groupStore.findGroupById(id);
      if (!group || hasRealChoice(group)) return;
      const options = optionsForChoice('best', group.formats, group.mediaKind);
      useMediaOptionsStore().setOptions(id, options);
      close();
      void useMediaStore().downloadGroup(id, options);
    },
  );

  return { groupId, open, touched, show, close, touch };
});
