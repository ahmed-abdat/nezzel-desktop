import { describe, expect, it } from 'vitest';
import { flushPromises, mount } from '@vue/test-utils';
import WelcomeShortcut from '../../src/components/welcome/WelcomeShortcut.vue';
import { i18n } from '../../src/i18n';
import { shouldShowWelcome, useOnboardingStore } from '../../src/stores/onboarding';
import { installTauriMock } from '../utils/tauriMock';
import { defaultSettings } from '../../src/tauri/types/config';

/** Welcome sheet: first launch only; existing users and unknown state never see it. */
describe('onboarding', () => {
  it.each([
    ['fresh profile, empty library', { welcomeDone: false, closeNoteDone: false }, 0, true],
    ['already welcomed', { welcomeDone: true, closeNoteDone: false }, 0, false],
    ['existing user with downloads', { welcomeDone: false, closeNoteDone: false }, 3, false],
    ['preferences failed to load', undefined, 0, false],
  ] as const)('%s', (_name, onboarding, libraryCount, expected) => {
    expect(shouldShowWelcome(onboarding, libraryCount)).toBe(expected);
  });

  it('marks an existing user done without showing the sheet', async () => {
    const patches: unknown[] = [];
    installTauriMock({
      library_list: () => [{ id: 'a' }],
      preferences_set: (_cmd, args) => {
        patches.push((args as { patch: unknown }).patch);
        return { paths: {}, recents: { recent: {} }, formats: {} };
      },
    });
    const store = useOnboardingStore();
    await store.decide({ welcomeDone: false, closeNoteDone: false });
    expect(store.showWelcome).toBe(false);
    expect(patches).toEqual([{ onboarding: { welcomeDone: true } }]);
  });

  it('skips the welcome and saves nothing when the library lookup fails', async () => {
    const writes: string[] = [];
    installTauriMock({
      library_list: () => Promise.reject(new Error('io')),
      preferences_set: (cmd) => {
        writes.push(cmd);
        return null;
      },
      config_set: (cmd) => {
        writes.push(cmd);
        return null;
      },
    });
    const store = useOnboardingStore();
    await store.decide({ welcomeDone: false, closeNoteDone: false });
    expect(store.showWelcome).toBe(false);
    expect(writes).toEqual([]);
  });

  it('Get started saves both switches and the done flag', async () => {
    const config: unknown[] = [];
    const prefs: unknown[] = [];
    installTauriMock({
      library_list: () => [],
      config_set: (_cmd, args) => {
        config.push((args as { patch: unknown }).patch);
        return structuredClone(defaultSettings);
      },
      preferences_set: (_cmd, args) => {
        prefs.push((args as { patch: unknown }).patch);
        return null;
      },
    });
    const store = useOnboardingStore();
    await store.decide({ welcomeDone: false, closeNoteDone: false });
    expect(store.showWelcome).toBe(true);

    await store.complete({ card: false, start: true });
    expect(store.showWelcome).toBe(false);
    // Start with my computer launches hidden (tray only).
    expect(config).toEqual([{ input: { watchClipboard: false }, system: { autoStartEnabled: true, autoStartMinimised: true } }]);
    // The welcome carries the reports switch, so no separate "reports are on" notice later.
    expect(store.sawReportsSwitch).toBe(true);
    expect(prefs).toEqual([{ onboarding: { welcomeDone: true, reportsNoticeDone: true } }]);
  });

  it('Wayland: the shortcut row says so and offers the system-shortcut command instead of a recorder', async () => {
    installTauriMock({ capture_shortcut_status: () => ({ failed: true, active: null, reason: 'wayland' }) });
    const wrapper = mount(WelcomeShortcut, { props: { mac: false }, global: { plugins: [i18n] } });
    await flushPromises();
    expect(wrapper.find('[data-testid="welcome-shortcut-wayland"]').text()).toContain('nezzel --capture');
    expect(wrapper.find('[data-testid="welcome-shortcut-skip"]').exists()).toBe(false);
  });
});
