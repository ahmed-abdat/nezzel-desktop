import { ensureInvokeArgsObject, IPCHandler } from '../tauriMock';
import { StrongholdInitPayload } from '../../../src/stores/stronghold';
import { InvokeArgs } from '@tauri-apps/api/core';

export type StrongholdMockData = {
  fields?: Record<string, string | null>;
  status?: StrongholdInitPayload;
};

// Saved values outlive a reload (sessionStorage), like the real vault, so tests can save, reload and look.
const SAVED_KEY = 'e2e.stronghold.fields';

function readFields(): Record<string, string> {
  const saved = sessionStorage.getItem(SAVED_KEY);
  const fields: Record<string, string | null> = saved ? JSON.parse(saved) : (window.E2E.stronghold?.fields ?? {});
  return Object.fromEntries(Object.entries(fields).filter((entry): entry is [string, string] => entry[1] != null));
}

export const strongholdHandlers: Record<string, IPCHandler> = {
  stronghold_status: (): StrongholdInitPayload => {
    return window.E2E.stronghold?.status ?? {
      unlocked: false,
    };
  },
  stronghold_init: (): StrongholdInitPayload => {
    return window.E2E.stronghold?.status ?? {
      unlocked: true,
    };
  },
  stronghold_keys: (): number[][] => {
    const encoder = new TextEncoder();
    return Object.keys(readFields()).map(key => Array.from(encoder.encode(key)));
  },
  stronghold_get: (): Record<string, number[] | null> => {
    const encoder = new TextEncoder();
    const result: Record<string, number[] | null> = {};
    for (const [key, value] of Object.entries(readFields())) {
      result[key] = Array.from(encoder.encode(value));
    }
    return result;
  },
  // Same contract as stronghold_set.rs: a null entry deletes the key.
  stronghold_set: (_cmd, args: InvokeArgs | undefined): void => {
    const { entries } = ensureInvokeArgsObject<{ entries: Record<string, number[] | null> }>(args, 'stronghold_set');
    const fields = readFields();
    const decoder = new TextDecoder();
    for (const [key, raw] of Object.entries(entries)) {
      if (raw == null) delete fields[key];
      else fields[key] = decoder.decode(new Uint8Array(raw));
    }
    sessionStorage.setItem(SAVED_KEY, JSON.stringify(fields));
  },
};
