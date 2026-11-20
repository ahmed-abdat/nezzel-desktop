import { flushPromises } from '@vue/test-utils';
import { describe, expect, it, vi } from 'vitest';
import { applyLanguage } from '../../src/appearance';
import { i18n } from '../../src/i18n';
import { useSettingsStore } from '../../src/stores/settings';
import { installTauriMock } from '../utils/tauriMock';
import type { Settings } from '../../src/tauri/types/config';

const clone = <T>(value: T): T => JSON.parse(JSON.stringify(value));

/** Optimistic switches: a failed patch rolls back only its own fields, never a concurrent success. */
describe('settings patchNow', () => {
  it.each(['patchNow', 'patch'] as const)('%s applies language before the backend settles and restores it when saving fails', async (method) => {
    const store = useSettingsStore();
    store.settings.appearance.language = 'en';
    await applyLanguage('en');
    let reject!: () => void;
    installTauriMock({ config_set: () => new Promise((_resolve, fail) => (reject = () => fail('io'))) });

    const saved = store[method]({ appearance: { language: 'ar' } }).catch(() => false);
    await vi.dynamicImportSettled();
    await flushPromises();
    try {
      expect(i18n.global.locale.value).toBe('ar');
      expect(document.documentElement.lang).toBe('ar');
      expect(document.documentElement.dir).toBe('rtl');
    } finally {
      reject();
      expect(await saved).toBe(false);
    }
    expect(store.settings.appearance.language).toBe('en');
    expect(i18n.global.locale.value).toBe('en');
    expect(document.documentElement.lang).toBe('en');
    expect(document.documentElement.dir).toBe('ltr');
  });

  it('keeps a concurrent successful patch when an earlier one fails', async () => {
    const store = useSettingsStore();
    store.settings.appearance.theme = 'system';
    store.settings.input.watchClipboard = false;
    const pending: { patch: Partial<Settings>; settle: (ok: boolean) => void }[] = [];
    installTauriMock({
      config_set: (_cmd, args) => new Promise((resolve, reject) => {
        const { patch } = args as { patch: Partial<Settings> };
        pending.push({ patch, settle: ok => (ok ? resolve(mergeEngine(patch)) : reject('io')) });
      }),
    });
    const engine: Settings = clone(store.settings);
    const mergeEngine = (patch: Partial<Settings>) => {
      if (patch.input) Object.assign(engine.input, patch.input);
      if (patch.appearance) Object.assign(engine.appearance, patch.appearance);
      return clone(engine);
    };

    const theme = store.patchNow({ appearance: { theme: 'dark' } });
    const clipboard = store.patchNow({ input: { watchClipboard: true } });
    expect(store.settings.appearance.theme).toBe('dark');
    expect(store.settings.input.watchClipboard).toBe(true);

    pending[1].settle(true);
    await flushPromises();
    pending[0].settle(false);
    expect(await theme).toBe(false);
    expect(await clipboard).toBe(true);
    expect(store.settings.appearance.theme).toBe('system');
    expect(store.settings.input.watchClipboard).toBe(true);
  });

  it('does not undo a newer value of the same field', async () => {
    const store = useSettingsStore();
    store.settings.appearance.theme = 'system';
    const settle: ((ok: boolean) => void)[] = [];
    installTauriMock({
      config_set: () => new Promise((resolve, reject) => settle.push(ok => (ok ? resolve({ ...clone(store.settings), appearance: { ...clone(store.settings.appearance), theme: 'light' } }) : reject('io')))),
    });
    const first = store.patchNow({ appearance: { theme: 'dark' } });
    const second = store.patchNow({ appearance: { theme: 'light' } });
    settle[1](true);
    await flushPromises();
    settle[0](false);
    await Promise.all([first, second]);
    expect(store.settings.appearance.theme).toBe('light');
  });

  it('a consent switch that could not be saved comes back and tells the caller', async () => {
    const store = useSettingsStore();
    store.settings.privacy.sendReports = true;
    let reject!: () => void;
    installTauriMock({ config_set: () => new Promise((_resolve, fail) => (reject = () => fail('io'))) });
    const saved = store.setPrivacy('sendReports', false);
    expect(store.settings.privacy.sendReports).toBe(false);
    reject();
    expect(await saved).toBe(false);
    expect(store.settings.privacy.sendReports).toBe(true);
  });
});
