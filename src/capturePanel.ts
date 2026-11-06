import { createApp } from 'vue';
import { createPinia } from 'pinia';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import CaptureBar from './components/downloads/CaptureBar.vue';
import CaptureCard from './components/downloads/CaptureCard.vue';
import { i18n } from './i18n';
import { applyAppearance } from './appearance';
import type { Settings } from './tauri/types/config';
import { CAPTURE_WINDOW } from './generated/contract';

export async function mountCapturePanel() {
  if (typeof __E2E__ !== 'undefined' && __E2E__) {
    const { installCaptureMock } = await import('../tests/utils/mocks/captureHandlers');
    installCaptureMock(new URLSearchParams(location.search).get('window') ?? CAPTURE_WINDOW.card);
  }
  document.documentElement.classList.add('nz-capture');
  const settings = await invoke<Settings>('config_get');
  applyAppearance(settings.appearance);
  const app = createApp(getCurrentWindow().label === CAPTURE_WINDOW.bar ? CaptureBar : CaptureCard).use(createPinia()).use(i18n);
  app.mount('#app');
  return app;
}
