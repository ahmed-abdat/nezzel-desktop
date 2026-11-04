/** Human site name for a yt-dlp extractor key or a URL host. */
const KNOWN: Record<string, string> = {
  youtube: 'YouTube',
  youtubetab: 'YouTube',
  tiktok: 'TikTok',
  twitter: 'X',
  instagram: 'Instagram',
  facebook: 'Facebook',
  vimeo: 'Vimeo',
  reddit: 'Reddit',
  soundcloud: 'SoundCloud',
  dailymotion: 'Dailymotion',
  twitch: 'Twitch',
};

/**
 * Registrable domains we name by host. Exact match (or a subdomain of one) only: the login card
 * asks for consent with this name, so "youtube.evil.net" must never read as "YouTube".
 */
const KNOWN_HOSTS: Record<string, string> = {
  'youtube.com': 'YouTube',
  'youtu.be': 'YouTube',
  'tiktok.com': 'TikTok',
  'twitter.com': 'X',
  'x.com': 'X',
  'instagram.com': 'Instagram',
  'facebook.com': 'Facebook',
  'fb.watch': 'Facebook',
  'vimeo.com': 'Vimeo',
  'reddit.com': 'Reddit',
  'soundcloud.com': 'SoundCloud',
  'dailymotion.com': 'Dailymotion',
  'twitch.tv': 'Twitch',
};

export function siteName(extractor: string | undefined, url: string | undefined): string | undefined {
  const key = extractor?.split(':')[0]?.toLowerCase();
  if (key && key !== 'generic') return KNOWN[key] ?? key.charAt(0).toUpperCase() + key.slice(1);
  try {
    const host = new URL(url ?? '').hostname.toLowerCase().replace(/^www\./, '');
    // Before the extractor is known (still reading, or a login request), name known sites by host.
    const known = Object.keys(KNOWN_HOSTS).find(domain => host === domain || host.endsWith(`.${domain}`));
    return known ? KNOWN_HOSTS[known] : host;
  } catch {
    return undefined;
  }
}
