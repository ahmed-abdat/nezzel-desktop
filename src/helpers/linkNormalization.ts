import { LINK_RULES } from '../generated/contract';

const hostIs = (host: string, domain: string) => host === domain || host.endsWith(`.${domain}`);

/** Pure counterpart of links::normalize::pure; rules are generated from Rust. */
export function routingUrl(input: string): URL {
  let url = new URL(input);
  const seen = new Set([url.href]);
  const wrapperFor = (value: URL) => LINK_RULES.wrappers.find(([host, path, , sub]) =>
    value.pathname === path && (value.hostname === host || (sub && hostIs(value.hostname, host))));
  const validate = (value: URL) => {
    if (!['http:', 'https:'].includes(value.protocol) || value.username || value.password) throw new Error('Unsafe URL');
  };
  validate(url);
  for (let depth = 0; depth < LINK_RULES.maxWrappers; depth++) {
    const rule = wrapperFor(url);
    if (!rule) break;
    const [, , keys, , relative] = rule;
    const raw = url.search.slice(1);
    const target = keys.length
      ? keys.map(key => url.searchParams.get(key)).find(Boolean)
      : /^https?:\/\//.test(raw) ? raw : decodeURIComponent(raw);
    if (!target) throw new Error('Malformed wrapper');
    url = relative && target.startsWith('/') && !target.startsWith('//')
      ? new URL(target, 'https://www.youtube.com')
      : new URL(target);
    validate(url);
    if (seen.has(url.href)) throw new Error('Wrapper loop');
    seen.add(url.href);
  }
  if (wrapperFor(url)) throw new Error('Wrapper limit');
  const alias = LINK_RULES.aliases.find(([host]) => host === url.hostname.replace(/^www\./, ''));
  if (alias) url.hostname = alias[1];
  const xPost = /\/status\/\d+(?:\/|$)/.test(url.pathname);
  const pairs = url.search.slice(1).split('&');
  const retained = pairs.filter((pair) => {
    const key = new URLSearchParams(pair).keys().next().value?.toLowerCase() ?? '';
    return !(key.startsWith('utm_') || LINK_RULES.trackers.some(tracker => tracker === key)
      || LINK_RULES.scopedTrackers.some(([hosts, keys, postOnly]) =>
        hosts.some(host => hostIs(url.hostname, host)) && keys.some(k => k === key) && (!postOnly || xPost)));
  });
  if (retained.length !== pairs.length) url.search = retained.join('&');
  return url;
}
