import { ensureInvokeArgsObject, IPCHandler } from '../tauriMock';
import { InvokeArgs } from '@tauri-apps/api/core';
import { defaultSettings } from '../../../src/tauri/types/config.ts';

/** Settings the app loads at start, merged over the defaults (e.g. language, theme). */
export type SettingsMockData = Record<string, unknown>;

export const configHandlers: Record<string, IPCHandler> = {
  // Fixed so keycaps and modifier clicks don't depend on the machine running the tests.
  get_platform: () => window.E2E?.platform ?? 'linux',
  config_get: () => {
    const overrides = window.E2E?.settings;
    return overrides ? deepMerge(structuredClone(defaultSettings) as unknown as Record<string, unknown>, overrides) : defaultSettings;
  },
  config_set: (_cmd, args: InvokeArgs | undefined) => {
    const { patch } = ensureInvokeArgsObject<{ patch: Record<string, unknown> }>(args, 'config_set');
    return deepMerge(structuredClone(defaultSettings) as unknown as Record<string, unknown>, patch);
  },
};

/** Same merge as the backend's config_set: nested objects merge, everything else replaces. */
function deepMerge(target: Record<string, unknown>, patch: Record<string, unknown>): Record<string, unknown> {
  for (const [key, value] of Object.entries(patch)) {
    const current = target[key];
    target[key] = isPlainObject(value) && isPlainObject(current) ? deepMerge(current, value) : value;
  }
  return target;
}

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}
