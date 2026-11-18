import { SITES } from '@/generated/contract';
import { siteForHost } from './siteLogo';

/** Known sites from the engine's site table (`sites.rs`). */
const KNOWN: readonly { name: string; hosts: readonly string[]; extractors: readonly string[] }[] = SITES;

/** Human site name for a yt-dlp extractor key or a URL host. */
export function siteName(extractor: string | undefined, url: string | undefined): string | undefined {
  const key = extractor?.split(':')[0]?.toLowerCase();
  const knownExtractor = key === 'youtubeclip' ? 'youtube' : key;
  const extractedSite = KNOWN.find(site => site.extractors.includes(knownExtractor ?? ''));
  if (extractedSite) return extractedSite.name;
  try {
    const host = new URL(url ?? '').hostname.toLowerCase().replace(/^www\./, '');
    // Before the extractor is known (still reading, or a login request), name known sites by host.
    // Exact host or a subdomain only: the login card asks for consent with this name, so
    // "youtube.evil.net" must never read as "YouTube".
    const known = siteForHost(host);
    if (known) return known.name;
    return key && key !== 'generic' ? key.charAt(0).toUpperCase() + key.slice(1) : host;
  } catch {
    return key && key !== 'generic' ? key.charAt(0).toUpperCase() + key.slice(1) : undefined;
  }
}

/** Host only (no scheme, path or query, no leading www), so a signed download link never shows in the UI. */
export function hostOf(url: string | undefined): string {
  try {
    return new URL(url ?? '').hostname.replace(/^www\./, '');
  } catch {
    return '';
  }
}
