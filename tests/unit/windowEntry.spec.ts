import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
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

describe('copied-link card', () => {
  it('shows the title from the snapshot when capture_metadata never arrives (Linux, tauri#3654)', async () => {
    vi.useFakeTimers();
    const reading = { id: 4, url: 'https://www.youtube.com/watch?v=card', site: 'youtube.com', title: '', details: { thumbnail: null, duration: null, uploader: '', views: null, bestHeight: null, bestExt: '', size: null }, failed: false, resolving: false, feed: false };
    let snapshot = reading;
    const previous = window.__TAURI_INTERNALS__.invoke;
    window.__TAURI_INTERNALS__.invoke = async (cmd, ...rest) => (cmd === 'capture_snapshot' ? snapshot : previous(cmd, ...rest));
    try {
      mockWindows('capture-card');
      window.history.replaceState(null, '', '/');
      app = await mountWindow('capture-card');
      await vi.advanceTimersByTimeAsync(0);
      expect(document.querySelector('.capture-panel')?.textContent).not.toContain('Card title');
      // The preview finished in Rust; its event is held, so only the next snapshot carries the title.
      snapshot = { ...reading, title: 'Card title' };
      await vi.advanceTimersByTimeAsync(1600);
      expect(document.querySelector('.capture-panel')?.textContent).toContain('Card title');
    } finally {
      window.__TAURI_INTERNALS__.invoke = previous;
      vi.useRealTimers();
    }
  });

  it('stops asking once the offer is gone (dismissed while reading)', async () => {
    vi.useFakeTimers();
    const reading = { id: 5, url: 'https://www.youtube.com/watch?v=gone', site: 'youtube.com', title: '', details: { thumbnail: null, duration: null, uploader: '', views: null, bestHeight: null, bestExt: '', size: null }, failed: false, resolving: false, feed: false };
    let snapshot: typeof reading = reading;
    let asked = 0;
    const previous = window.__TAURI_INTERNALS__.invoke;
    window.__TAURI_INTERNALS__.invoke = async (cmd, ...rest) => {
      if (cmd !== 'capture_snapshot') return previous(cmd, ...rest);
      asked++;
      return snapshot;
    };
    try {
      mockWindows('capture-card');
      window.history.replaceState(null, '', '/');
      app = await mountWindow('capture-card');
      await vi.advanceTimersByTimeAsync(0);
      snapshot = { ...reading, id: 0, url: '', site: '' };
      await vi.advanceTimersByTimeAsync(1600);
      const after = asked;
      await vi.advanceTimersByTimeAsync(10_000);
      expect(asked).toBe(after);
    } finally {
      window.__TAURI_INTERNALS__.invoke = previous;
      vi.useRealTimers();
    }
  });
});
