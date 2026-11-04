import { SelectOption } from './forms.ts';

export function buildImpersonatePresets(t: (key: string) => string): SelectOption[] {
  return [
    { value: 'none', label: t('settings.network.impersonate.options.none') },
    { value: 'any', label: t('settings.network.impersonate.options.any') },
    { value: 'chrome-131:windows-10', label: 'Chrome 131 - Windows 10' },
    { value: 'chrome-131:macos-14', label: 'Chrome 131 - macOS 14' },
    { value: 'chrome-133:macos-15', label: 'Chrome 133 - macOS 15' },
    { value: 'chrome-131:android-14', label: 'Chrome 131 - Android 14' },
    { value: 'chrome-99:android-12', label: 'Chrome 99 - Android 12' },
    { value: 'edge-101:windows-10', label: 'Edge 101 - Windows 10' },
    { value: 'firefox-135:macos-14', label: 'Firefox 135 - macOS 14' },
    { value: 'firefox-133:macos-14', label: 'Firefox 133 - macOS 14' },
    { value: 'safari-18.4:ios-18.4', label: 'Safari 18.4 - iOS 18.4' },
    { value: 'safari-18.0:macos-15', label: 'Safari 18.0 - macOS 15' },
    { value: 'safari-17.0:macos-14', label: 'Safari 17.0 - macOS 14' },
    { value: 'safari-15.5:macos-14', label: 'Safari 15.5 - macOS 14' },
    { value: 'tor-14.5:macos-14', label: 'Tor 14.5 - macOS 14' },
  ];
}

/** scheme://[user:pass@]host[:port][/] with a scheme yt-dlp accepts for --proxy. */
const PROXY_RE = /^(https?|socks4a?|socks5h?):\/\/([^\s/@]+@)?(\[[0-9a-f:.]+\]|[^\s:/@[\]]+)(:\d{1,5})?\/?$/i;

export function isValidProxyUrl(value: string): boolean {
  const match = PROXY_RE.exec(value.trim());
  if (!match) return false;
  const port = match[4] ? Number(match[4].slice(1)) : null;
  return port === null || (port > 0 && port <= 65535);
}
