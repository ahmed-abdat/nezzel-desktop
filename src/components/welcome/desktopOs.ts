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

/** Linux desktop family from XDG_CURRENT_DESKTOP (backend `clipboard::desktop_from`): whose tray words to use. */
export type LinuxDesktop = 'gnome' | 'kde' | 'other';

/** `welcome.place.*` key: GNOME says top bar, KDE system tray, the rest notification area. */
export function trayPlaceKey(os: DesktopOs, desktop: LinuxDesktop): string {
  if (os !== 'linux') return os;
  return desktop === 'gnome' ? 'linux' : desktop === 'kde' ? 'linuxKde' : 'linuxOther';
}

/** `closeNote.*` hint for Linux: GNOME points at Activities, others at their panel. */
export function linuxHintKey(desktop: LinuxDesktop): string {
  return desktop === 'gnome' ? 'hintLinux' : desktop === 'kde' ? 'hintKde' : 'hintOther';
}

/** Last two folder names, joined the way the prototype shows them: "Downloads/Nezzel". */
export function folderLabel(path: string | null | undefined): string {
  const parts = (path ?? '').split(/[\\/]/).filter(Boolean);
  return parts.length ? parts.slice(-2).join('/') : 'Downloads/Nezzel';
}
