import { describe, expect, it, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';

const invoke = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: unknown[]) => invoke(...args) }));

import { useStrongholdStore } from '../../src/stores/stronghold';

describe('stronghold store', () => {
  it('a new store starts locked with no error, whatever an earlier store did (#50)', async () => {
    setActivePinia(createPinia());
    invoke.mockResolvedValueOnce({ unlocked: false, initError: 'boom' });
    await expect(useStrongholdStore().initialize()).rejects.toThrow('boom');

    setActivePinia(createPinia());
    expect(useStrongholdStore().status).toEqual({ unlocked: false });
  });
});
