import { createApp } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import CloseNote from './components/welcome/CloseNote.vue';
import { i18n } from './i18n';
import { applyAppearance, applyLanguage } from './appearance';
import type { Settings } from './tauri/types/config';
import { LINUX_DESKTOPS } from './generated/contract';

/** The one-time note after the first close, in its own small native window by the tray. */
export async function mountCloseNote() {
  // Same transparent page as the capture panels, so the arrow and rounded corners show.
  document.documentElement.classList.add('nz-capture');
  try {
    const settings = await invoke<Settings>('config_get');
    applyAppearance(settings.appearance);
  } catch {
    applyLanguage('system');
  }
  const q = new URLSearchParams(location.search);
  const app = createApp(CloseNote, {
    os: q.get('os') ?? 'mac',
    desktop: LINUX_DESKTOPS.find(d => d === q.get('desktop')) ?? 'other',
    edge: q.get('edge') === 'bottom' ? 'bottom' : 'top',
    arrowX: Number(q.get('nx')) || 160,
  }).use(i18n);
  app.mount('#app');
  return app;
}
