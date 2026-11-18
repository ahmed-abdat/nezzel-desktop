import { MaybeRef, unref } from 'vue';
import { LINK_RULES, SITES } from '../generated/contract';
import { routingUrl } from './linkNormalization';

/** Same pure rules as Rust; presentation variants are equated only for pending-row identity. */
export function pendingUrlKey(input: string): string {
  const normalized = normalizePastedUrl(input);
  try {
    const url = routingUrl(normalized);
    const onHost = (host: string) => url.hostname === host || url.hostname.endsWith(`.${host}`);
    const site = { key: onHost('instagram.com') ? 'instagram' : onHost('youtube.com') || url.hostname === 'youtu.be' ? 'youtube' : '' };
    const parts = url.pathname.replace(/\/$/, '').split('/');
    if (site.key === 'instagram' && parts.length === 3 && parts[2] && LINK_RULES.instagramPaths.some(path => path === parts[1])) {
      url.hostname = 'instagram.com';
      url.pathname = `/p/${url.pathname.split('/')[2]}`;
      url.hash = '';
    } else if (site?.key === 'youtube') {
      const video = url.hostname === 'youtu.be'
        ? url.pathname.slice(1).split('/')[0]
        : parts.length === 3 && LINK_RULES.youtubePaths.some(path => path === parts[1]) ? url.pathname.split('/')[2] : url.pathname === '/watch' ? url.searchParams.get('v') : null;
      if (video) {
        url.hostname = 'youtube.com';
        url.pathname = '/watch';
        url.searchParams.set('v', video);
      }
    }
    url.searchParams.sort();
    return url.href;
  } catch {
    return normalized;
  }
}

/** Normalize pasted links; keep signed/share query values intact. */
export function normalizePastedUrl(input: string): string {
  const value = input.replace(/^[\s\u061c\u200b\u200e\u200f\u202a-\u202e\u2060\u2066-\u2069\ufeff]+|[\s\u061c\u200b\u200e\u200f\u202a-\u202e\u2060\u2066-\u2069\ufeff]+$/g, '');
  const hasScheme = /^[a-z][a-z0-9+.-]*:/i.test(value);
  const rewritten = !hasScheme;
  try {
    const url = new URL(hasScheme ? value : `https://${value}`);
    if (!/^https?:$/.test(url.protocol)) return value;
    if (!hasScheme) {
      if (url.username || url.password || url.port) return value;
      const knownSite = SITES.some(site => site.hosts.some(host => url.hostname === host || url.hostname.endsWith(`.${host}`)));
      const host = url.hostname.replace(/^www\./, '');
      const path = url.pathname;
      const bareShare = (host === 'github.com' && !path.endsWith('/')
        && /^\/[^/]+\/[^/]+\/(?:(?:blob|raw)\/[^/]+|releases\/download\/[^/]+)\/.+/.test(path))
      || (host === 'raw.githubusercontent.com' && !path.endsWith('/') && /^\/[^/]+\/[^/]+\/[^/]+\/.+/.test(path))
      || (host === 'pixeldrain.com' && /^\/(?:u|api\/file)\/[A-Za-z0-9_-]+(?:\/info)?\/?$/.test(path))
      || (['yadi.sk', 'disk.yandex.ru', 'disk.yandex.com', 'disk.yandex.net'].includes(host) && /^\/[di]\/[A-Za-z0-9_-]+\/?$/.test(path))
      || (host === 'mediafire.com' && (/^\/file\/[A-Za-z0-9_-]+(?:\/|$)/.test(path) || (path === '/' && /^\?[A-Za-z0-9_-]+$/.test(url.search))))
      || (host === 'drive.usercontent.google.com' && path === '/download' && /^[A-Za-z0-9_-]+$/.test(url.searchParams.get('id') ?? ''));
      const knownLink = LINK_RULES.aliases.some(([alias]) => host === alias)
        || LINK_RULES.shorteners.some(short => host === short)
        || LINK_RULES.wrappers.some(([wrapper, , , sub]) => url.hostname === wrapper || (sub && url.hostname.endsWith(`.${wrapper}`)));
      if (!knownSite && !bareShare && !knownLink) return value;
    }
    return rewritten ? url.href : value;
  } catch {
    return value;
  }
}

export function isValidUrl(input: MaybeRef<string | null>): boolean {
  const value = unref(input);
  if (!value) return false;

  try {
    const url = new URL(normalizePastedUrl(value));
    return /^https?:$/.test(url.protocol);
  } catch {
    return false;
  }
}
