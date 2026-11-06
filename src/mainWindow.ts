import { createApp } from 'vue';
import { createPinia } from 'pinia';
import App from './App.vue';
import router from './router';
import tauriListeners from './plugins/tauriListeners';
import { useSettingsStore } from './stores/settings';
import { applyAppearance } from './appearance';
import { invoke } from '@tauri-apps/api/core';
import { installErrorLog } from './tauri/errorLog';
import { i18n } from './i18n';
import { startWindowWatcher } from './tauri/window.ts';
import { usePreferencesStore } from './stores/preferences.ts';
import { useMediaStore } from './stores/media/media.ts';
import { startCaptureDelivery } from './tauri/listeners/capture';
import { watchTrayProgress } from './tauri/listeners/trayProgress';
import { useOnboardingStore } from './stores/onboarding';

export async function mountMainWindow() {
  const pinia = createPinia();
  const app = createApp(App);

  if (import.meta.env.DEV) {
    (globalThis as typeof globalThis & {
      __OVD_DEBUG__?: {
        pinia: typeof pinia;
        getStore: (id: string) => unknown;
      };
    }).__OVD_DEBUG__ = {
      pinia,
      getStore: (id: string) => (pinia as typeof pinia & { _s: Map<string, unknown> })._s.get(id),
    };
  }

  // Frontend errors go to Rust (diagnostics_log), where the scrubber and the reports switch apply.
  installErrorLog(app);

  if (__E2E__) (await import('../tests/utils/mocks/mainWindowMocks')).installMainWindowMocks();

  app.use(i18n);
  app.use(router);
  app.use(pinia);
  app.use(tauriListeners);

  if (import.meta.hot) {
    import.meta.hot.dispose(() => {
      const mediaStore = useMediaStore();
      mediaStore.deleteAllGroups(false);
    });
  }

  const settingsStore = useSettingsStore();
  const preferencesStore = usePreferencesStore();

  initStores().catch((e) => {
    console.error(e);
  }).finally(() => {
    if (!__E2E__) {
      startWindowWatcher();
      watchTrayProgress();
      void startCaptureDelivery();
      shareMotionPreference();
    }
    app.mount('#app');
    // app_ready shows the window: only once there is something to paint. Shown with an empty page,
    // WebKitGTK (Fedora KDE Wayland VM, #36) kept the window grey until the first click.
    void firstFrame().then(() => invoke('app_ready'));
    if (!__E2E__) void useMediaStore().loadLibrary().catch(() => console.warn('Unable to load local download history'));
  });

  /** The first route rendered and laid out. A hidden window may never run animation frames, so a
   * frame is awaited only while the page is visible, and never longer than 300 ms. */
  async function firstFrame(): Promise<void> {
    await Promise.race([router.isReady(), wait(300)]).catch(() => {});
    void document.body.offsetHeight; // style and layout now, so the first paint has the page
    if (document.visibilityState !== 'visible') return;
    await Promise.race([new Promise(requestAnimationFrame), wait(300)]);
  }

  function wait(ms: number): Promise<void> {
    return new Promise(resolve => setTimeout(resolve, ms));
  }

  /** The first-close shrink is native: Rust needs the user's reduced-motion choice. */
  function shareMotionPreference(): void {
    const query = matchMedia('(prefers-reduced-motion: reduce)');
    const send = () => void invoke('window_motion_pref', { reduce: query.matches }).catch(() => {});
    send();
    query.addEventListener('change', send);
  }

  async function initStores(): Promise<void> {
    try {
      const prefs = await preferencesStore.load();
      await useOnboardingStore().decide(prefs?.onboarding);
    } catch (e) {
      console.error(`Unable to load preferences: ${e}`);
    }

    try {
      const settings = await settingsStore.load();
      applyAppearance(settings.appearance);
    } catch (e) {
      console.error(`Unable to load settings: ${e}`);
    }
  }
}
