import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { mockWindows } from '@tauri-apps/api/mocks';
import type { App } from 'vue';
import { flushPromises } from '@vue/test-utils';
import { defaultSettings } from '../../src/tauri/types/config';
import { mountWindow } from '../../src/windowEntry';

let app: App | undefined;
const forbidden: string[] = [];

beforeEach(() => {
  document.body.innerHTML = '<div id="app"></div>';
  forbidden.length = 0;
  // Any main-window init command or permission query is an isolation failure.
  window.__TAURI_INTERNALS__.invoke = async (cmd) => {
    const allowed: Record<string, unknown> = {
      config_get: structuredClone(defaultSettings),
      get_platform: 'macos',
      capture_snapshot: { id: 1, url: '', site: '', title: '', details: { thumbnail: null, duration: null, uploader: '', views: null, bestHeight: null, bestExt: '', size: null }, failed: false, resolving: false },
    };
    if (cmd in allowed) return allowed[cmd];
    if (cmd === 'plugin:event|listen') return 1;
    if (cmd === 'plugin:event|unlisten') return;
    forbidden.push(cmd);
    throw new Error(`Main-only IPC attempted by a capture panel: ${cmd}`);
  };
});

afterEach(() => {
  app?.unmount();
  app = undefined;
  document.body.innerHTML = '';
});

describe('native window entry isolation', () => {
  it.each(['capture-card', 'capture-bar'])('mounts %s without main-only IPC when its URL has no capture query', async (label) => {
    mockWindows(label);
    window.history.replaceState(null, '', '/');
    app = await mountWindow(label);
    await flushPromises();
    expect(document.querySelector('.capture-panel')).not.toBeNull();
    expect(forbidden).toEqual([]);
    expect(document.querySelector('#capture-url') !== null).toBe(label === 'capture-bar');
  });
  it('leaves an unknown native window inert', async () => {
    mockWindows('unrecognized');
    await mountWindow('unrecognized');
    expect(document.getElementById('app')?.childElementCount).toBe(0);
    expect(forbidden).toEqual([]);
  });
});
