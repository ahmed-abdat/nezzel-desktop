import { watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { useMediaGroupStore } from '../../stores/media/group';
import { MediaState, useMediaStateStore } from '../../stores/media/state';

/** Keeps the tray's "downloading N" line and its titles in step with the queue. */
export function watchTrayProgress() {
  const groups = useMediaGroupStore();
  const states = useMediaStateStore();
  watch(() => Object.values(groups.groups).flatMap(group => Object.values(group.items))
    .filter(item => [MediaState.downloading, MediaState.downloadingList].includes(states.getState(item.id) as MediaState))
    .map(item => item.title ?? ''), (titles) => {
    void invoke('capture_tray_progress', { count: titles.length, titles }).catch(() => {});
  }, { immediate: true });
}
