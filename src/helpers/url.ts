import { MaybeRef, unref } from 'vue';
import { SITES } from '../generated/contract';

/** Normalize pasted links; keep signed/share query values intact. */
export function normalizePastedUrl(input: string): string {
  const value = input.replace(/^[\s\u061c\u200b\u200e\u200f\u202a-\u202e\u2060\u2066-\u2069\ufeff]+|[\s\u061c\u200b\u200e\u200f\u202a-\u202e\u2060\u2066-\u2069\ufeff]+$/g, '');
  const hasScheme = /^[a-z][a-z0-9+.-]*:/i.test(value);
  let rewritten = !hasScheme;
  try {
    const url = new URL(hasScheme ? value : `https://${value}`);
    if (!/^https?:$/.test(url.protocol)) return value;
    if (['fxtwitter.com', 'www.fxtwitter.com', 'vxtwitter.com', 'www.vxtwitter.com'].includes(url.hostname)) {
      url.hostname = 'x.com';
      rewritten = true;
    }
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
      if (!knownSite && !bareShare) return value;
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
