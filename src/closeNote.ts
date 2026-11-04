import { createApp } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import CloseNote from './components/welcome/CloseNote.vue';
import { applyDocumentLocale, getDefaultLocale, i18n, type Locale } from './i18n';
import { applyTheme } from './composables/useTheme';
import type { Settings } from './tauri/types/config';

/** The one-time note after the first close, in its own small native window by the tray. */
export async function mountCloseNote() {
  // Same transparent page as the capture panels, so the arrow and rounded corners show.
  document.documentElement.classList.add('nz-capture');
  try {
    const settings = await invoke<Settings>('config_get');
    const locale = settings.appearance.language === 'system' ? getDefaultLocale() : settings.appearance.language as Locale;
    i18n.global.locale.value = locale;
    applyDocumentLocale(locale);
    if (settings.appearance.theme !== 'system') applyTheme(settings.appearance.theme);
  } catch {
    applyDocumentLocale(getDefaultLocale());
  }
  const q = new URLSearchParams(location.search);
  const app = createApp(CloseNote, {
    os: q.get('os') ?? 'mac',
    edge: q.get('edge') === 'bottom' ? 'bottom' : 'top',
    arrowX: Number(q.get('nx')) || 160,
  }).use(i18n);
  app.mount('#app');
  return app;
}
