import { afterEach, describe, expect, it, vi } from 'vitest';
import { flushPromises } from '@vue/test-utils';
import { invoke } from '@tauri-apps/api/core';
import { platformFromUserAgent, resetPlatformForTests, usePlatform } from '../../src/composables/usePlatform';
import { Platform } from '../../src/tauri/types/app';
import { shortcutKeys } from '../../src/helpers/shortcut';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));

const MAC_UA = 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko)';
const WIN_UA = 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36 Edg/130.0.0.0';
const LINUX_UA = 'Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15 (KHTML, like Gecko)';

afterEach(() => vi.mocked(invoke).mockReset());

describe('platform detection', () => {
  it.each([
    [MAC_UA, Platform.MacOS],
    [WIN_UA, Platform.Windows],
    [LINUX_UA, Platform.Linux],
    ['', Platform.Unknown],
  ])('reads %s', (ua, platform) => {
    expect(platformFromUserAgent(ua)).toBe(platform);
  });

  // The owner's Mac showed "Ctrl Alt Shift D": the hint used Unknown (PC keycaps) whenever get_platform had not
  // answered or failed. The webview's own user agent now decides first paint.
  it('shows Mac keycaps on a Mac even when get_platform fails', async () => {
    vi.mocked(invoke).mockRejectedValue(new Error('ipc down'));
    resetPlatformForTests(platformFromUserAgent(MAC_UA));
    const { isMac } = usePlatform();
    expect(shortcutKeys('CommandOrControl+Alt+N', isMac.value)).toEqual(['⌥', '⌘', 'N']);
    await flushPromises();
    expect(isMac.value).toBe(true);
  });

  it('the backend answer wins and is asked once per window', async () => {
    vi.mocked(invoke).mockResolvedValue(Platform.Windows);
    resetPlatformForTests(platformFromUserAgent(MAC_UA));
    const first = usePlatform();
    usePlatform();
    await flushPromises();
    expect(invoke).toHaveBeenCalledTimes(1);
    expect(first.isMac.value).toBe(false);
    expect(shortcutKeys('CommandOrControl+Alt+N', first.isMac.value)).toEqual(['Ctrl', 'Alt', 'N']);
  });
});
