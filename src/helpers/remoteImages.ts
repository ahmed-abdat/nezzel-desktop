interface ImageNetworkSettings {
  enableProxy?: boolean | null;
  proxy?: string | null;
}

/** Webview requests bypass the app's proxy. Local previews can still be displayed. */
export function remoteImagesAllowed(network: ImageNetworkSettings, url?: string | null): boolean {
  if (network.enableProxy !== true) return true;
  if (!url) return false;
  try {
    const { protocol, hostname } = new URL(url, window.location.href);
    // Windows serves app and asset files from http(s)://*.localhost: local, not the network.
    if (hostname === 'localhost' || hostname.endsWith('.localhost')) return true;
    return protocol !== 'http:' && protocol !== 'https:';
  } catch {
    return false;
  }
}
