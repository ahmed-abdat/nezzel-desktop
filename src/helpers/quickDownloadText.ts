import { Platform } from '../tauri/types/app';

/**
 * Quick download does different things per OS, and its texts must say what really happens:
 * macOS (AppleScript, Automation permission) and Windows (UI Automation, no permission) read the
 * active browser tab; Linux (and an unknown OS) only uses the copied link.
 */
export function quickDownloadText(platform: Platform) {
  const readsBrowser = platform === Platform.MacOS || platform === Platform.Windows;
  return {
    barEmpty: readsBrowser ? 'capture.emptyHint' : 'capture.emptyHintClipboard',
    welcomeHelp: readsBrowser ? 'welcome.shortcut.helpBrowser' : 'welcome.shortcut.help',
    homeHint: readsBrowser ? 'downloads.empty.hintQuick' : 'downloads.empty.hintQuickClipboard',
    /** The Automation hint and "Open Automation settings" exist only on macOS. */
    automation: platform === Platform.MacOS,
  };
}
