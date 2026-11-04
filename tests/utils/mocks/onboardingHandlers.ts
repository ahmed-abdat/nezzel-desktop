import { IPCHandler } from '../tauriMock';
import { defaultPreferences, type Preferences } from '../../../src/tauri/types/preferences.ts';

/** Saved preferences for a test, e.g. `{ onboarding: { welcomeDone: false } }` for a fresh profile. */
export type PreferencesMockData = Partial<Preferences>;

/** Preferences only answer when a test seeds them; other tests keep the old "nothing saved" (null) path. */
export const onboardingHandlers: Record<string, IPCHandler> = {
  preferences_get: () => (window.E2E?.preferences ? { ...defaultPreferences, ...window.E2E.preferences } : null),
  preferences_set: (_cmd, args) => {
    if (!window.E2E?.preferences) return null;
    const patch = (args as { patch?: Partial<Preferences> } | undefined)?.patch ?? {};
    window.E2E.preferences = { ...window.E2E.preferences, ...patch, onboarding: { ...window.E2E.preferences.onboarding, ...patch.onboarding } as Preferences['onboarding'] };
    return { ...defaultPreferences, ...window.E2E.preferences };
  },
  // Welcome "Try it": the backend writes the clipboard and shows the real card.
  capture_try_link: async () => {
    await navigator.clipboard.writeText('https://www.youtube.com/watch?v=jNQXAC9IVRw');
    return null;
  },
};
