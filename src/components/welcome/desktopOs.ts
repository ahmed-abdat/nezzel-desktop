import { Platform } from '../../tauri/types/app';

export type DesktopOs = 'mac' | 'windows' | 'linux';

/** The user's own OS for the welcome demo and the first-close note; the browser guess is for e2e. */
export function desktopOs(platform: Platform, ua: string = typeof navigator === 'undefined' ? '' : `${navigator.platform} ${navigator.userAgent}`): DesktopOs {
  if (platform === Platform.MacOS) return 'mac';
  if (platform === Platform.Windows) return 'windows';
  if (platform === Platform.Linux) return 'linux';
  const s = ua.toLowerCase();
  if (/mac|iphone|ipad/.test(s)) return 'mac';
  if (/win/.test(s)) return 'windows';
  if (/linux|x11|cros/.test(s) && !/android/.test(s)) return 'linux';
  return 'mac';
}

/** Last two folder names, joined the way the prototype shows them: "Downloads/Nezzel". */
export function folderLabel(path: string | null | undefined): string {
  const parts = (path ?? '').split(/[\\/]/).filter(Boolean);
  return parts.length ? parts.slice(-2).join('/') : 'Downloads/Nezzel';
}
