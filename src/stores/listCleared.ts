import { defineStore } from 'pinia';
import { ref, watch } from 'vue';
import { libraryPathKey } from '../tauri/types/library';

const KEY = 'nezzel.listCleared';
/** Old entries fall off first; enough for years of downloads at a few bytes each. */
export const MAX_CLEARED = 5000;
const MAX = MAX_CLEARED;

function load(): string[] {
  try {
    const parsed: unknown = JSON.parse(globalThis.localStorage?.getItem(KEY) ?? '[]');
    return Array.isArray(parsed) ? parsed.filter((p): p is string => typeof p === 'string').slice(-MAX) : [];
  } catch {
    return [];
  }
}

/**
 * Finished downloads the user took off the main list ("Remove from list"). Keyed by file path so
 * the choice survives restarts (history rows get new ids). The files and the history stay; the
 * rows still show under Completed.
 */
export const useListClearedStore = defineStore('list-cleared', () => {
  const keys = ref<Set<string>>(new Set(load()));
  watch(keys, (value) => {
    try {
      globalThis.localStorage?.setItem(KEY, JSON.stringify([...value]));
    } catch { /* storage unavailable: keep in memory */ }
  });

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
  return { keys, clear, restore, forget: restore, isCleared };
});
