import { invoke, type InvokeArgs } from '@tauri-apps/api/core';

/** Best-effort IPC (usage counts, failure reports, error log): never throws, never rejects, never blocks the
 *  caller. A mocked or missing backend may throw or not even return a promise. */
export function fireAndForget(cmd: string, args?: InvokeArgs): void {
  try {
    void Promise.resolve(invoke(cmd, args)).catch(() => {});
  } catch {
    // Best effort.
  }
}
