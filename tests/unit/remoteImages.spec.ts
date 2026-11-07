import { describe, expect, it } from 'vitest';
import { remoteImagesAllowed } from '../../src/helpers/remoteImages';

describe('remote image privacy', () => {
  it.each([
    [false, 'https://example.com/image.jpg', true],
    [false, 'http://example.com/image.jpg', true],
    [false, 'blob:local-preview', true],
    [false, 'data:image/png;base64,AA==', true],
    [true, 'https://example.com/image.jpg', false],
    [true, 'http://example.com/image.jpg', false],
    [true, 'blob:local-preview', true],
    [true, 'data:image/png;base64,AA==', true],
    [true, 'asset://localhost/preview.png', true],
    [true, '//example.com/image.jpg', false],
    [true, 'http://asset.localhost/C%3A/preview.png', true],
    [true, 'https://tauri.localhost/icon.png', true],
    [true, 'https://cdn.localhost.example.com/image.jpg', false],
  ])('proxy %s, %s -> %s', (enableProxy, url, allowed) => {
    expect(remoteImagesAllowed({ enableProxy }, url)).toBe(allowed);
  });

  it.each([undefined, null, '', 'socks5://localhost:1080'])('blocks remote images with the switch on regardless of address %s', (proxy) => {
    expect(remoteImagesAllowed({ enableProxy: true, proxy })).toBe(false);
  });

  it.each([undefined, null, false])('allows remote images when the switch is %s', (enableProxy) => {
    expect(remoteImagesAllowed({ enableProxy, proxy: 'socks5://localhost:1080' })).toBe(true);
  });
});
