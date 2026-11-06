import { invoke } from '@tauri-apps/api/core';

/**
 * Clipboard text for paste actions, or null when nothing can be read. One Rust reader (the watcher's: GTK first
 * on Linux, which works on GNOME Wayland where the plugin's arboard path often doesn't); no JS fallbacks.
 */
export async function readClipboardText(): Promise<string | null> {
  try {
    return (await invoke<string | null>('capture_read_clipboard')) || null;
  } catch {
    return null;
  }
}

/** The supported link on the clipboard, or '' (never throws): the Add field's auto-fill. */
export async function readClipboardLink(): Promise<string> {
  return (await invoke<string>('capture_read_clipboard', { supportedOnly: true }).catch(() => '')) || '';
}

/**
 * Copy text through the backend so the clipboard watcher records the write as Nezzel's own and does not offer
 * it back as a copied link. Rejects when the clipboard is unavailable: callers surface their own error.
 */
export async function copyText(text: string): Promise<void> {
  await invoke('capture_write_clipboard', { text });
}
