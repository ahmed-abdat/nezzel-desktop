import { defineStore } from 'pinia';
import { computed, ref } from 'vue';
import type { Group, PlaylistParent } from '../../tauri/types/group';
import { useMediaGroupStore } from './group';
import { MediaState, useMediaStateStore } from './state';
import { notifyGroup } from '../../tauri/notifications';
import { NotificationKind } from '../../tauri/types/app';
import { i18n } from '../../i18n';
import { useLoginStore } from '../login';

const TERMINAL = new Set<MediaState | undefined>([MediaState.done, MediaState.error, MediaState.cancelled]);

/**
 * Playlists shown as one row over their downloads. Each download stays a normal group (the engine
 * pauses and cancels per group); children point here with `parentId`. Parent meta lives until its
 * last child leaves the list.
 */
export const usePlaylistStore = defineStore('media-playlist', () => {
  const playlists = ref<Record<string, PlaylistParent>>({});
  const groupStore = useMediaGroupStore();
  const stateStore = useMediaStateStore();
  /** Parents whose "finished" notice went out; cleared when one of their downloads runs again. */
  const notified = new Set<string>();

  /** Children per parent, in playlist order. One pass over the list, shared by every row. */
  const childrenByParent = computed(() => {
    const map = new Map<string, Group[]>();
    for (const group of groupStore.orderedGroups) {
      if (!group.parentId || !playlists.value[group.parentId]) continue;
      const list = map.get(group.parentId);
      if (list) list.push(group);
      else map.set(group.parentId, [group]);
    }
    for (const list of map.values()) list.sort((a, b) => (a.parentIndex ?? 0) - (b.parentIndex ?? 0));
    return map;
  });

  const childrenOf = (parentId: string): Group[] => childrenByParent.value.get(parentId) ?? [];
  const findPlaylist = (id: string): PlaylistParent | undefined => playlists.value[id];

  function register(parent: PlaylistParent) {
    playlists.value[parent.id] = parent;
  }

  /**
   * Drops parent meta once no download points at it. Deferred one microtask, so a cancel that
   * swaps a child for its re-keyed snapshot in the same tick does not lose the parent.
   */
  function prune(parentId: string | undefined) {
    if (!parentId) return;
    queueMicrotask(() => {
      if (Object.values(groupStore.groups).some(group => group.parentId === parentId)) return;
      delete playlists.value[parentId];
      notified.delete(parentId);
      useLoginStore().forgetGroup(parentId);
    });
  }

  function rearm(parentId: string | undefined) {
    if (parentId) notified.delete(parentId);
  }

  /** One notice per playlist when its last download stops: all saved, or how many failed. Never per item. */
  function settle(parentId: string | undefined) {
    if (!parentId) return;
    const parent = playlists.value[parentId];
    if (!parent) return;
    const children = Object.values(groupStore.groups).filter(group => group.parentId === parentId);
    const states = children.map(child => stateStore.getGroupState(child.id));
    if (states.some(state => !TERMINAL.has(state))) {
      notified.delete(parentId);
      return;
    }
    if (notified.has(parentId)) return;
    notified.add(parentId);
    const done = states.filter(state => state === MediaState.done).length;
    const failed = states.filter(state => state === MediaState.error).length;
    const asGroup = { title: parent.title, url: parent.url ?? '' } as Group;
    if (failed > 0) {
      const message = i18n.global.t('downloads.playlist.notifyFailed', { n: failed, total: children.length }, failed);
      void notifyGroup(NotificationKind.DownloadFailed, asGroup, { message });
    } else if (done > 0) {
      void notifyGroup(NotificationKind.PlaylistFinished, asGroup, {}, done);
    }
  }

  return { playlists, childrenByParent, childrenOf, findPlaylist, register, prune, rearm, settle };
});
