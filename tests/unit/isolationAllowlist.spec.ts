import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { beforeAll, describe, expect, it } from 'vitest';

/**
 * The isolation hook throws on any command it doesn't list, and callers mostly swallow the error,
 * so a missing entry is a button that silently does nothing (e.g. "Delete file…" never asked).
 * Every command the frontend can send must pass the real hook.
 */

const SRC = join(__dirname, '../../src');

function sources(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return sources(path);
    return /\.(ts|vue)$/.test(name) ? [readFileSync(path, 'utf8')] : [];
  });
}

/** Plugin wrappers the app calls, and the IPC command each one sends. */
const DIALOG: [RegExp, string][] = [[/\b(confirm|ask|message)\b/, 'plugin:dialog|message'], [/\bopen\b/, 'plugin:dialog|open']];
const WINDOW: [string, string][] = [['show', 'plugin:window|show'], ['setFocus', 'plugin:window|set_focus'], ['setSize', 'plugin:window|set_size']];

function usedCommands(): string[] {
  const used = new Set<string>();
  for (const file of sources(SRC)) {
    for (const m of file.matchAll(/invoke(?:<[^>]*>)?\(\s*['"]([\w:|-]+)['"]/g)) used.add(m[1]);
    const dialogImports = /import \{([^}]+)\} from '@tauri-apps\/plugin-dialog'/.exec(file)?.[1] ?? '';
    for (const [name, cmd] of DIALOG) if (name.test(dialogImports)) used.add(cmd);
    for (const [method, cmd] of WINDOW) if (file.includes(`getCurrentWindow().${method}(`)) used.add(cmd);
    // Tauri's drag-region script sends these itself, without an invoke() in our code.
    if (file.includes('data-tauri-drag-region')) ['plugin:window|start_dragging', 'plugin:window|internal_toggle_maximize'].forEach(c => used.add(c));
  }
  return [...used].sort();
}

describe('isolation allowlist', () => {
  beforeAll(async () => {
    await import('../../src-isolation/main');
  });

  it('finds the commands the app sends', () => {
    expect(usedCommands()).toEqual(expect.arrayContaining(['media_trash_file', 'plugin:dialog|message', 'plugin:window|show']));
  });

  it.each(usedCommands())('lets %s through', (cmd) => {
    expect(() => window.__TAURI_ISOLATION_HOOK__({ cmd })).not.toThrow();
  });

  it('still blocks unknown commands', () => {
    expect(() => window.__TAURI_ISOLATION_HOOK__({ cmd: 'plugin:fs|remove' })).toThrow();
  });
});
