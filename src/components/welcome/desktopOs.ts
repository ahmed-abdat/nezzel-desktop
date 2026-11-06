import { Platform } from '../../tauri/types/app';
import { compactFolderPath } from '../../helpers/folderPath';
import type { LinuxDesktop } from '@/generated/contract';

export type { LinuxDesktop };

export type DesktopOs = 'mac' | 'windows' | 'linux';

/** The user's own OS for the welcome demo and the first-close note; the webview's guess (usePlatform) covers e2e. */
export function desktopOs(platform: Platform): DesktopOs {
  if (platform === Platform.Windows) return 'windows';
  if (platform === Platform.Linux) return 'linux';
  return 'mac';
}

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
  return compactFolderPath(path ?? '', '/') || 'Downloads/Nezzel';
}
