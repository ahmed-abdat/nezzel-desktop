import { i18n } from '../i18n';
import { COOKIE_BROWSERS, type CookieBrowser } from '@/generated/contract';

/** Display names of the browsers yt-dlp can read logins (cookies) from; the list and its order come from Rust. */
const NAMES: Record<CookieBrowser, string> = {
  chrome: 'Chrome', firefox: 'Firefox', safari: 'Safari', edge: 'Edge', brave: 'Brave',
  opera: 'Opera', vivaldi: 'Vivaldi', chromium: 'Chromium', whale: 'Whale',
};

/** Browser choices for Settings: stored lower-case, shown by name. */
export const COOKIE_BROWSER_OPTIONS = COOKIE_BROWSERS.map(key => ({ value: key, label: NAMES[key] }));

/** Display name for an engine browser key ("chrome" → "Chrome"). */
export function browserName(key: string): string {
  const lower = key.toLowerCase();
  return (COOKIE_BROWSERS as readonly string[]).includes(lower) ? NAMES[lower as CookieBrowser] : key.charAt(0).toUpperCase() + key.slice(1);
}

/** Browser plus the human-readable profile name supplied by discovery. */
export function candidateLabel(candidate: { browser: string; profile: string }): string {
  const name = browserName(candidate.browser);
  const profile = candidate.browser.toLowerCase() === 'firefox'
    ? candidate.profile.replace(/ · userContext(Personal|Work|Banking|Shopping)\.label$/, (_, container: string) => ` · ${i18n.global.t(`downloads.login.containers.${container.toLowerCase()}`)}`)
    : candidate.profile;
  return candidate.browser.toLowerCase() === 'safari' || !profile ? name : `${name} (${profile})`;
}
