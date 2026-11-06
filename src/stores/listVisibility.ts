import { defineStore } from 'pinia';
import { ref } from 'vue';
import { libraryPathKey } from '../helpers/folderPath';
import { persistedRef, type PersistCodec } from '../helpers/persistedRef';

const KEY = 'nezzel.listCleared';
/** Old entries fall off first; enough for years of downloads at a few bytes each. */
export const MAX_CLEARED = 5000;
const MAX = MAX_CLEARED;

const clearedCodec: PersistCodec<Set<string>> = {
  parse: (raw) => {
    try {
      const parsed: unknown = JSON.parse(raw);
      return Array.isArray(parsed) ? new Set(parsed.filter((p): p is string => typeof p === 'string').slice(-MAX)) : undefined;
    } catch {
      return undefined;
    }
  },
  serialize: value => JSON.stringify([...value]),
};

/**
 * Which rows the main list leaves out, two ways:
 * - hidden: rows hidden while an undo toast is open (gone from the list, not yet removed from the engine);
 *   in memory only.
 * - cleared: finished downloads the user took off the main list ("Remove from list"). Keyed by file path
 *   so the choice survives restarts (history rows get new ids; stored under `nezzel.listCleared`).
 *   The files and the history stay; the rows still show under Completed.
 */
export const useListVisibilityStore = defineStore('list-visibility', () => {
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

  const keys = persistedRef(KEY, clearedCodec, new Set<string>());

  /** Newest last (Set keeps insertion order), so the oldest entries go first past MAX. */
  const clear = (paths: string[]) => {
    const next = new Set(keys.value);
    for (const key of paths.map(libraryPathKey)) {
      next.delete(key);
      next.add(key);
    }
    keys.value = next.size > MAX ? new Set([...next].slice(-MAX)) : next;
  };
  /** Undo, or the file and its history are gone: a new download to the same path must show. */
  const restore = (paths: string[]) => {
    const next = new Set(keys.value);
    for (const path of paths) next.delete(libraryPathKey(path));
    keys.value = next;
  };
  /** A row is cleared when it has files and every one of them was cleared. */
  const isCleared = (paths: string[]) => paths.length > 0 && paths.every(path => keys.value.has(libraryPathKey(path)));

  return { ids, hide, show, isHidden, keys, clear, restore, forget: restore, isCleared };
});
