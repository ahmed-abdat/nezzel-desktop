import { ref, watch, type Ref } from 'vue';

/** How a value is stored as text. `parse` returns undefined for anything it does not accept (the fallback is used). */
export interface PersistCodec<T> {
  parse: (raw: string) => T | undefined;
  serialize: (value: T) => string;
}

export const flagCodec: PersistCodec<boolean> = {
  parse: raw => raw === '1',
  serialize: value => (value ? '1' : '0'),
};

export const textCodec: PersistCodec<string> = {
  parse: raw => raw,
  serialize: value => value,
};

export function choiceCodec<T extends string>(choices: readonly T[]): PersistCodec<T> {
  return {
    parse: raw => choices.find(choice => choice === raw),
    serialize: value => value,
  };
}

/**
 * A ref kept in the webview's localStorage. Storage can be missing or full: reads fall back and writes
 * are dropped, the value then lives in memory for the session.
 */
export function persistedRef<T>(key: string, codec: PersistCodec<T>, fallback: T): Ref<T> {
  let stored: string | null = null;
  try {
    stored = globalThis.localStorage?.getItem(key) ?? null;
  } catch { /* storage unavailable: use the fallback */ }
  const value = ref(stored == null ? fallback : (codec.parse(stored) ?? fallback)) as Ref<T>;
  watch(value, (next) => {
    try {
      globalThis.localStorage?.setItem(key, codec.serialize(next));
    } catch { /* storage unavailable: keep in memory */ }
  });
  return value;
}
