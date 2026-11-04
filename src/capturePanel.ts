import { createApp } from 'vue';
import { createPinia } from 'pinia';
import { invoke } from '@tauri-apps/api/core';
import CapturePanel from './components/downloads/CapturePanel.vue';
import { applyDocumentLocale, getDefaultLocale, i18n, type Locale } from './i18n';
import { applyTheme } from './composables/useTheme';
import type { Settings } from './tauri/types/config';

export async function mountCapturePanel() {
  if (typeof __E2E__ !== 'undefined' && __E2E__) {
    const { installCaptureMock } = await import('../tests/utils/mocks/captureHandlers');
    installCaptureMock(new URLSearchParams(location.search).get('window') ?? 'capture-card');
  }
  document.documentElement.classList.add('nz-capture');
  const settings = await invoke<Settings>('config_get');
  const locale = settings.appearance.language === 'system' ? getDefaultLocale() : settings.appearance.language as Locale;
  i18n.global.locale.value = locale;
  applyDocumentLocale(locale);
  if (settings.appearance.theme !== 'system') applyTheme(settings.appearance.theme);
  const app = createApp(CapturePanel).use(createPinia()).use(i18n);
  app.mount('#app');
  return app;
}
