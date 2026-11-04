import { invoke } from '@tauri-apps/api/core';
import { readText } from '@tauri-apps/plugin-clipboard-manager';

/**
 * Clipboard text for paste actions, or null when nothing can be read. The backend command uses the session's
 * readers (GTK first on Linux, which works on GNOME Wayland where the plugin's arboard path often doesn't);
 * the plugin and the browser API are fallbacks (older backends, browser previews).
 */
export async function readClipboardText(): Promise<string | null> {
  // An empty answer falls through too: one reader may see nothing where another sees the link.
  try {
    const text = await invoke<string | null>('capture_read_clipboard');
    if (typeof text === 'string' && text) return text;
  } catch { /* fall through */ }
  try {
    const text = await readText();
    if (typeof text === 'string' && text) return text;
  } catch { /* fall through */ }
  try {
    return await navigator.clipboard?.readText?.() ?? null;
  } catch {
    return null;
  }
}
