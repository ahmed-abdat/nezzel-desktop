/**
 * Global shortcut accelerators (Tauri global-shortcut syntax, e.g. "CommandOrControl+Shift+D") to and from
 * keyboard events and display keycaps. The raw accelerator string is never shown to the user.
 */
import { DEFAULT_CAPTURE_SHORTCUT, FALLBACK_CAPTURE_SHORTCUT } from '@/generated/contract';
import type { ShortcutAction } from '../tauri/types/shortcuts';

/** Quick download: ⌥⌘N on macOS, Ctrl+Alt+N elsewhere; FALLBACK is what the backend registers when the default
 *  is taken (or types AltGr+N on Windows). Both come from config_models.rs. */
export { DEFAULT_CAPTURE_SHORTCUT, FALLBACK_CAPTURE_SHORTCUT };
/** What a Wayland system shortcut runs to open Quick download (handled by the single-instance hand-off). */
export const CAPTURE_COMMAND = '/usr/bin/nezzel --capture';

type Mod = 'CommandOrControl' | 'Control' | 'Alt' | 'Shift' | 'Super';
const MOD_ORDER: Mod[] = ['CommandOrControl', 'Control', 'Alt', 'Shift', 'Super'];

const MOD_ALIASES: Record<string, Mod> = {
  commandorcontrol: 'CommandOrControl',
  cmdorctrl: 'CommandOrControl',
  cmdorcontrol: 'CommandOrControl',
  commandorctrl: 'CommandOrControl',
  control: 'Control',
  ctrl: 'Control',
  alt: 'Alt',
  option: 'Alt',
  shift: 'Shift',
  super: 'Super',
  command: 'Super',
  cmd: 'Super',
  meta: 'Super',
};

const NAMED_KEYS: Record<string, string> = {
  Space: 'Space', Enter: 'Enter', Tab: 'Tab', Escape: 'Escape', Backspace: 'Backspace', Delete: 'Delete',
  Insert: 'Insert', Home: 'Home', End: 'End', PageUp: 'PageUp', PageDown: 'PageDown',
  ArrowUp: 'Up', ArrowDown: 'Down', ArrowLeft: 'Left', ArrowRight: 'Right',
  Minus: '-', Equal: '=', BracketLeft: '[', BracketRight: ']', Backslash: '\\', Semicolon: ';', Quote: '\'',
  Comma: ',', Period: '.', Slash: '/', Backquote: '`',
};

export interface ParsedShortcut {
  mods: Mod[];
  key: string;
}

/** Splits an accelerator into canonical modifiers (fixed order) and its key. Null when it has no key. */
export function parseAccelerator(accelerator: string): ParsedShortcut | null {
  const tokens = accelerator.split('+').map(token => token.trim()).filter(Boolean);
  if (tokens.length === 0) return null;
  const mods = new Set<Mod>();
  let key: string | null = null;
  for (const token of tokens) {
    const mod = MOD_ALIASES[token.toLowerCase()];
    if (mod) mods.add(mod);
    else key = token.length === 1 ? token.toUpperCase() : token.replace(/^Key([A-Z])$/, '$1').replace(/^Digit(\d)$/, '$1');
  }
  if (!key) return null;
  return { mods: MOD_ORDER.filter(mod => mods.has(mod)), key };
}

const MAC_GLYPHS: Record<Mod, string> = { CommandOrControl: '⌘', Control: '⌃', Alt: '⌥', Shift: '⇧', Super: '⌘' };
const PC_NAMES: Record<Mod, string> = { CommandOrControl: 'Ctrl', Control: 'Ctrl', Alt: 'Alt', Shift: 'Shift', Super: 'Win' };
const MAC_KEY_GLYPHS: Record<string, string> = { Enter: '↩', Backspace: '⌫', Delete: '⌦', Escape: '⎋', Tab: '⇥', Up: '↑', Down: '↓', Left: '←', Right: '→', Space: 'Space' };
// macOS order is ⌃⌥⇧⌘; Windows/Linux is Ctrl Alt Shift Win.
const MAC_ORDER: Mod[] = ['Control', 'Alt', 'Shift', 'CommandOrControl', 'Super'];

/** Keycaps as the OS prints them: ["⌘","⇧","D"] on macOS, ["Ctrl","Shift","D"] elsewhere. */
export function shortcutKeys(accelerator: string, mac: boolean): string[] {
  const parsed = parseAccelerator(accelerator);
  if (!parsed) return [];
  const mods = mac ? MAC_ORDER.filter(mod => parsed.mods.includes(mod)) : parsed.mods;
  const caps = [...new Set(mods.map(mod => (mac ? MAC_GLYPHS[mod] : PC_NAMES[mod])))];
  const key = mac ? (MAC_KEY_GLYPHS[parsed.key] ?? parsed.key) : parsed.key;
  return [...caps, key];
}

/** Modifier keycaps held right now, in the OS order, for live feedback while recording. */
export function heldKeys(event: KeyLike, mac: boolean): string[] {
  const mods: Mod[] = [];
  if (mac ? event.metaKey : event.ctrlKey) mods.push('CommandOrControl');
  if (mac && event.ctrlKey) mods.push('Control');
  if (event.altKey) mods.push('Alt');
  if (event.shiftKey) mods.push('Shift');
  if (!mac && event.metaKey) mods.push('Super');
  const order = mac ? MAC_ORDER : MOD_ORDER;
  return [...new Set(order.filter(mod => mods.includes(mod)).map(mod => (mac ? MAC_GLYPHS[mod] : PC_NAMES[mod])))];
}

/** One-line label for help text: "⌘⇧D" on macOS, "Ctrl+Shift+D" elsewhere. */
export function shortcutLabel(accelerator: string, mac: boolean): string {
  return shortcutKeys(accelerator, mac).join(mac ? '' : '+');
}

export interface KeyLike {
  key: string;
  code: string;
  metaKey: boolean;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
}

export type RecordResult
  = | { kind: 'pending' } // only modifiers so far
    | { kind: 'cancel' }
    | { kind: 'clear' }
    | { kind: 'needsModifier' }
    | { kind: 'ok'; accelerator: string };

function keyFromCode(code: string): string | null {
  if (/^Key[A-Z]$/.test(code)) return code.slice(3);
  if (/^Digit\d$/.test(code)) return code.slice(5);
  if (/^F([1-9]|1\d|2[0-4])$/.test(code)) return code;
  return NAMED_KEYS[code] ?? null;
}

/**
 * Turns a keydown during recording into an outcome. Esc cancels, Backspace/Delete alone clears.
 * A shortcut needs ⌘/Ctrl or Alt plus a key; the primary modifier is stored as CommandOrControl so one
 * setting works on every OS.
 */
export function recordKey(event: KeyLike, mac: boolean): RecordResult {
  const hasPrimary = mac ? event.metaKey : event.ctrlKey;
  const extraCtrl = mac && event.ctrlKey;
  const anyMod = event.metaKey || event.ctrlKey || event.altKey || event.shiftKey;
  if (['Meta', 'Control', 'Alt', 'Shift', 'OS', 'AltGraph'].includes(event.key)) return { kind: 'pending' };
  if (event.key === 'Escape' && !anyMod) return { kind: 'cancel' };
  if ((event.key === 'Backspace' || event.key === 'Delete') && !anyMod) return { kind: 'clear' };
  const key = keyFromCode(event.code);
  if (!key) return { kind: 'needsModifier' };
  if (!hasPrimary && !extraCtrl && !event.altKey && !(!mac && event.metaKey)) return { kind: 'needsModifier' };
  const parts: string[] = [];
  if (hasPrimary) parts.push('CommandOrControl');
  if (extraCtrl) parts.push('Control');
  if (event.altKey) parts.push('Alt');
  if (event.shiftKey) parts.push('Shift');
  if (!mac && event.metaKey) parts.push('Super');
  parts.push(key);
  return { kind: 'ok', accelerator: parts.join('+') };
}

/** Same shortcut regardless of alias spelling or modifier order. */
export function sameShortcut(a: string, b: string): boolean {
  const pa = parseAccelerator(a);
  const pb = parseAccelerator(b);
  if (!pa || !pb) return !pa && !pb;
  return pa.key === pb.key && pa.mods.join('+') === pb.mods.join('+');
}

// Shortcuts the OS or every app already owns, in canonical accelerator form (see parseAccelerator).
const RESERVED_MAC = [
  'CommandOrControl+A', 'CommandOrControl+C', 'CommandOrControl+V', 'CommandOrControl+X', 'CommandOrControl+Z', 'CommandOrControl+S', 'CommandOrControl+F', 'CommandOrControl+N', 'CommandOrControl+T', 'CommandOrControl+W', 'CommandOrControl+P', 'CommandOrControl+O', 'CommandOrControl+Q', 'CommandOrControl+H', 'CommandOrControl+M', 'CommandOrControl+Tab', 'CommandOrControl+Space', 'CommandOrControl+,', 'CommandOrControl+Shift+Z', 'CommandOrControl+Shift+3', 'CommandOrControl+Shift+4', 'CommandOrControl+Shift+5', 'CommandOrControl+Alt+Escape', 'CommandOrControl+Alt+H', 'CommandOrControl+Alt+M', 'CommandOrControl+Alt+D', 'CommandOrControl+Control+Q', 'CommandOrControl+Control+F', 'CommandOrControl+Control+Space', 'CommandOrControl+Control+D', 'Control+Space', 'Control+Alt+Space', 'CommandOrControl+Shift+D', 'CommandOrControl+Alt+Shift+V', 'CommandOrControl+`',
];
const RESERVED_PC = [
  'CommandOrControl+A', 'CommandOrControl+C', 'CommandOrControl+V', 'CommandOrControl+X', 'CommandOrControl+Z', 'CommandOrControl+S', 'CommandOrControl+F', 'CommandOrControl+N', 'CommandOrControl+T', 'CommandOrControl+W', 'CommandOrControl+P', 'CommandOrControl+O', 'CommandOrControl+Y', 'CommandOrControl+Shift+Z', 'CommandOrControl+Escape', 'CommandOrControl+Shift+Escape', 'CommandOrControl+Alt+Delete', 'CommandOrControl+Tab', 'CommandOrControl+Shift+Tab', 'Alt+F4', 'Alt+Tab', 'Alt+Shift+Tab', 'Alt+Space', 'Alt+Escape', 'CommandOrControl+Shift+D', 'CommandOrControl+Alt+D', 'CommandOrControl+,',
];

/** True when the OS (or every app, like copy/paste) already uses this shortcut. Windows-key combos are all OS. */
export function isReservedShortcut(accelerator: string, mac: boolean): boolean {
  const parsed = parseAccelerator(accelerator);
  if (!parsed) return false;
  if (!mac && parsed.mods.includes('Super')) return true;
  return (mac ? RESERVED_MAC : RESERVED_PC).some(reserved => sameShortcut(reserved, accelerator));
}

/** The accelerator a keydown event produces, or null when it is not a full shortcut. */
export function eventAccelerator(event: KeyLike, mac: boolean): string | null {
  const result = recordKey(event, mac);
  return result.kind === 'ok' ? result.accelerator : null;
}

/** Nezzel's own in-app keys (only while the window is focused). ⌘ on macOS, Ctrl elsewhere. */
export const BUILT_IN_KEYS = {
  paste: 'CommandOrControl+V',
  sidebar: 'CommandOrControl+B',
  sidebarAlt: 'CommandOrControl+\\',
  settings: 'CommandOrControl+,',
} as const;
export type BuiltInKey = keyof typeof BUILT_IN_KEYS;

/** Queue shortcuts a user can rebind (Settings > Keyboard shortcuts), only while Nezzel is focused. Empty = off. */
export const IN_APP_SHORTCUT_DEFAULTS = {
  downloadAll: 'CommandOrControl+Enter',
  addAndDownload: 'CommandOrControl+Shift+Enter',
} as const;
/** The queue action each rebindable in-app key runs. */
export const IN_APP_ACTIONS: Record<keyof typeof IN_APP_SHORTCUT_DEFAULTS, ShortcutAction> = {
  downloadAll: 'download_all',
  addAndDownload: 'media_add_and_download',
};

/** Which built-in key a keydown is (⌘ or Ctrl on any platform, as before), or null. */
export function builtInKeyOf(event: KeyLike): BuiltInKey | null {
  if (!(event.metaKey || event.ctrlKey)) return null;
  const plain = !event.shiftKey && !event.altKey;
  const key = event.key.toLowerCase();
  if (plain && (key === '\\' || key === 'b')) return 'sidebar';
  if (key === ',') return 'settings';
  if (key === 'v') return 'paste';
  return null;
}

/** `aria-keyshortcuts` value for a built-in key on either platform: "Meta+B Control+B". */
export function ariaKeyShortcuts(accelerator: string): string {
  const parsed = parseAccelerator(accelerator);
  return parsed ? `Meta+${parsed.key} Control+${parsed.key}` : '';
}
