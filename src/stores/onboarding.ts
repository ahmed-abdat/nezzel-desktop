import { defineStore } from 'pinia';
import { ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import type { OnboardingPreferences } from '../tauri/types/preferences';
import { usePreferencesStore } from './preferences';
import { useSettingsStore } from './settings';

/**
 * First launch only. Unknown onboarding state (preferences failed to load) never blocks the app,
 * and anyone with downloads in their library has used Nezzel before.
 */
export function shouldShowWelcome(onboarding: OnboardingPreferences | undefined, libraryCount: number): boolean {
  if (!onboarding || onboarding.welcomeDone) return false;
  return libraryCount === 0;
}

export interface WelcomeChoices {
  card: boolean;
  start: boolean;
}

export const useOnboardingStore = defineStore('onboarding', () => {
  const showWelcome = ref(false);
  /** The welcome sheet carries the reports switch: whoever saw it needs no separate notice. */
  const sawReportsSwitch = ref(false);

  /** Called once after preferences load, before the app mounts. */
  async function decide(onboarding: OnboardingPreferences | undefined): Promise<void> {
    if (!onboarding || onboarding.welcomeDone) {
      showWelcome.value = false;
      return;
    }
    let count: number;
    try {
      count = (await invoke<unknown[]>('library_list', { scan: false }) ?? []).length;
    } catch {
      // Can't tell a new user from an existing one: skip the welcome this time, change nothing.
      showWelcome.value = false;
      return;
    }
    showWelcome.value = shouldShowWelcome(onboarding, count);
    if (showWelcome.value) sawReportsSwitch.value = true;
    // An existing user: never ask again.
    if (!showWelcome.value) void usePreferencesStore().patch({ onboarding: { welcomeDone: true } }).catch(() => {});
  }

  /** The card switch takes effect at once so "Try it" shows the real card. */
  function setCard(on: boolean): Promise<boolean> {
    return useSettingsStore().patchNow({ input: { watchClipboard: on } });
  }

  /** "Get started": the sheet goes away now, the choices are saved behind it. */
  async function complete(choices: WelcomeChoices): Promise<void> {
    showWelcome.value = false;
    const settings = useSettingsStore();
    await settings.patchNow({
      input: { watchClipboard: choices.card },
      system: choices.start
        ? { autoStartEnabled: true, autoStartMinimised: true }
        : { autoStartEnabled: false },
    });
    await usePreferencesStore().patch({ onboarding: { welcomeDone: true, reportsNoticeDone: true } }).catch(() => {});
  }

  return { showWelcome, sawReportsSwitch, decide, setCard, complete };
});
