/** Browsers yt-dlp can read logins (cookies) from. Values are stored lower-case. */
export const COOKIE_BROWSERS = ['Chrome', 'Firefox', 'Safari', 'Edge', 'Brave', 'Opera', 'Vivaldi', 'Chromium', 'Whale'] as const;

/** Display name for an engine browser key ("chrome" → "Chrome"). */
export function browserName(key: string): string {
  return COOKIE_BROWSERS.find(b => b.toLowerCase() === key.toLowerCase()) ?? key.charAt(0).toUpperCase() + key.slice(1);
}

/** Browser plus the human-readable profile name supplied by discovery. */
export function candidateLabel(candidate: { browser: string; profile: string }): string {
  const name = browserName(candidate.browser);
  return candidate.browser.toLowerCase() === 'safari' || !candidate.profile ? name : `${name} (${candidate.profile})`;
}
