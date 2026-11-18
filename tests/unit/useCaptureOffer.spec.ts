import { mount, flushPromises } from '@vue/test-utils';
import { defineComponent, onMounted } from 'vue';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { useCaptureOffer } from '../../src/composables/useCaptureOffer';
import CaptureBar from '../../src/components/downloads/CaptureBar.vue';
import CaptureCard from '../../src/components/downloads/CaptureCard.vue';
import { emptyDetails, type CaptureOffer } from '../../src/captureTypes';
import { emit } from '@tauri-apps/api/event';
import { i18n } from '../../src/i18n';
import { installTauriMock } from '../utils/tauriMock';
import { siteLogo } from '../../src/helpers/siteLogo';
import { defaultSettings } from '../../src/tauri/types/config';

let wrapper: ReturnType<typeof mount>;
afterEach(() => wrapper?.unmount());

describe('automatic login after an anonymous preview refusal', () => {
  it.each([['card', CaptureCard, 'en'], ['bar', CaptureBar, 'ar']] as const)('%s offers Download without asking to use the browser login', async (name, component, locale) => {
    const send = vi.fn();
    installTauriMock({
      config_get: () => ({ ...defaultSettings, appearance: { ...defaultSettings.appearance, language: locale }, auth: { ...defaultSettings.auth, cookieBrowser: 'automatic', alwaysUseBrowserLogins: true } }),
      capture_snapshot: () => ({ id: 11, url: 'https://www.instagram.com/p/DEg2j6USJgo/?igsh=test', site: 'www.instagram.com', read: true, failed: true, reason: 'login', details: emptyDetails() }),
      capture_action: send,
    });
    wrapper = mount(component, { global: { plugins: [i18n] } });
    await flushPromises();
    expect(wrapper.find('[data-testid=capture-fail-actions]').exists()).toBe(false);
    expect(wrapper.find('[data-testid=capture-fail]').exists()).toBe(false);
    expect(wrapper.find('[data-testid=capture-automatic-login]').exists()).toBe(true);
    expect(wrapper.get('[data-testid=capture-automatic-login]').text()).toBe(i18n.global.t('capture.automaticLogin'));
    if (name === 'bar') await wrapper.get('form').trigger('submit');
    else await wrapper.get('[data-pick=best]').trigger('click');
    await flushPromises();
    expect(send).toHaveBeenCalledWith('capture_action', expect.objectContaining({ id: 11, action: 'download' }));
  });

  it('rechecks automatic login before dispatch if the owner switched it off while the card was open', async () => {
    let enabled = true;
    const send = vi.fn();
    installTauriMock({
      config_get: () => ({ ...defaultSettings, auth: { ...defaultSettings.auth, cookieBrowser: 'automatic', alwaysUseBrowserLogins: enabled } }),
      capture_snapshot: () => ({ id: 13, url: 'https://instagram.com/p/ABC', read: true, failed: true, reason: 'login', details: emptyDetails() }),
      capture_action: send,
    });
    wrapper = mount(CaptureCard, { global: { plugins: [i18n] } });
    await flushPromises();
    enabled = false;
    await wrapper.get('[data-pick=best]').trigger('click');
    await flushPromises();
    expect(send).not.toHaveBeenCalled();
    expect(wrapper.find('[data-testid=capture-fail-actions]').exists()).toBe(true);
  });

  it('keeps the login prompt when automatic logins are switched off', async () => {
    installTauriMock({
      config_get: () => ({ ...defaultSettings, auth: { ...defaultSettings.auth, cookieBrowser: 'automatic', alwaysUseBrowserLogins: false } }),
      capture_snapshot: () => ({ id: 12, url: 'https://instagram.com/p/ABC', read: true, failed: true, reason: 'login', details: emptyDetails() }),
    });
    wrapper = mount(CaptureCard, { global: { plugins: [i18n] } });
    await flushPromises();
    expect(wrapper.find('[data-testid=capture-fail-actions]').exists()).toBe(true);
    expect(wrapper.find('[data-pick=best]').exists()).toBe(false);
  });
});

describe('instant capture logos', () => {
  it.each([CaptureCard, CaptureBar])('renders a short-link logo before metadata, even with remote images disabled', async (component) => {
    installTauriMock({ capture_snapshot: () => ({ id: 1, url: 'https://vt.tiktok.com/abc', remoteImages: false, read: false, details: emptyDetails() }) });
    wrapper = mount(component, { global: { plugins: [i18n] } });
    await flushPromises();
    expect(wrapper.findAll('svg path').some(path => path.attributes('d') === siteLogo('https://tiktok.com'))).toBe(true);
    expect(wrapper.find('img[src$="favicon.ico"]').exists()).toBe(false);
  });

  it('requests a favicon only for an unknown site with remote images enabled', async () => {
    let capture!: ReturnType<typeof useCaptureOffer>;
    wrapper = mount(defineComponent({ setup() {
      capture = useCaptureOffer();
      return () => null;
    } }), { global: { plugins: [i18n] } });
    capture.offer.value.url = 'https://example.org/clip';
    expect(capture.favicon.value).toBeNull();
    capture.offer.value.remoteImages = true;
    expect(capture.favicon.value).toBe('https://example.org/favicon.ico');
    capture.offer.value.url = 'https://pin.it/abc';
    expect(capture.logo.value).toBeTruthy();
    expect(capture.favicon.value).toBeNull();
  });
});

describe('capture primary action', () => {
  it.each([
    undefined, { kind: 'best' }, { kind: 'small' }, { kind: 'audio' },
    { kind: 'audio', kbps: 128 }, { kind: 'video', height: 720 }, { kind: 'video', height: 1080, fps: 60 },
  ].flatMap(choice => [false, true].map(feed => ({ choice, feed }))) as { choice: import('../../src/generated/contract').CaptureChoice | undefined; feed: boolean }[])('keeps the playlist quality when routing to options (%j)', async ({ choice, feed }) => {
    const send = vi.fn();
    installTauriMock({
      capture_snapshot: () => ({ id: 1, url: 'https://example.com/list', read: true, details: { ...emptyDetails(), mediaKind: feed ? null : 'playlist' }, feed }),
      capture_action: send,
    });
    let capture: ReturnType<typeof useCaptureOffer>;
    wrapper = mount(defineComponent({
      setup() {
        capture = useCaptureOffer();
        onMounted(capture.start);
        return () => null;
      },
    }), { global: { plugins: [i18n] } });
    await flushPromises();
    await capture!.act('download', choice);
    expect(send).toHaveBeenCalledExactlyOnceWith('capture_action', { id: 1, action: 'options', choice: choice ?? null });
  });

  it.each([
    ['playlist', false, 'playlist', 'options'],
    ['feed', true, null, 'options'],
    ['video', false, 'video', 'download'],
  ] as const)('%s uses the expected primary action', async (_name, feed, mediaKind, action) => {
    const send = vi.fn();
    installTauriMock({
      capture_snapshot: () => ({ id: 1, url: 'https://example.com/list', title: 'Read title', read: true, details: { ...emptyDetails(), mediaKind }, feed, failed: false }),
      capture_action: send,
    });
    let capture: ReturnType<typeof useCaptureOffer>;
    wrapper = mount(defineComponent({
      setup() {
        capture = useCaptureOffer();
        onMounted(capture.start);
        return () => null;
      },
    }), { global: { plugins: [i18n] } });
    await flushPromises();
    await capture!.act('download');
    expect(send).toHaveBeenCalledExactlyOnceWith('capture_action', { id: 1, action, choice: null });
  });
});

describe('untitled capture read', () => {
  it.each([['bar', CaptureBar], ['card', CaptureCard]] as const)('%s leaves the reading state when long-title metadata arrives', async (_name, component) => {
    let offer: CaptureOffer = { id: 1, remoteImages: false, url: 'https://example.com/video', site: 'example.com', title: '', read: false, details: emptyDetails(), feed: false, failed: false, resolving: false, reason: null };
    installTauriMock({ capture_snapshot: () => offer });
    wrapper = mount(component, { global: { plugins: [i18n], stubs: { Tooltip: { template: '<slot />' } } } });
    await flushPromises();
    expect(wrapper.find('[data-testid=capture-choices-loading]').exists()).toBe(true);
    offer = { ...offer, read: true, title: 'A very long video title with mixed Arabic — مع نص عربي طويل جدًا في المنتصف (full uncut version, 4K)', details: { ...emptyDetails(), mediaKind: 'video', bestHeight: 1080, bestExt: 'mp4', size: 120_000_000, hasAudio: true } };
    await emit('capture_changed');
    await flushPromises();
    expect(wrapper.text()).toContain(offer.title);
    expect(wrapper.find('[data-testid=capture-choices-loading]').exists()).toBe(false);
    expect(wrapper.find('[role=status]').exists()).toBe(false);
    expect(wrapper.get('[data-testid=capture-best]').text()).toContain('120 MB');
  });
  it.each([['bar', CaptureBar], ['card', CaptureCard]] as const)('%s shows real choices after an untitled read', async (_name, component) => {
    installTauriMock({
      capture_snapshot: () => ({
        id: 1, remoteImages: false, url: 'https://example.com/video', site: 'example.com', title: '', read: true,
        details: { ...emptyDetails(), mediaKind: 'video', bestHeight: 1080, hasAudio: true, smallHeight: 720,
          choices: [{ choice: { kind: 'video', height: 1080 }, size: null }, { choice: { kind: 'audio' }, size: null }] },
        feed: false, failed: false, resolving: false, reason: null,
      }),
    });
    wrapper = mount(component, { global: { plugins: [i18n], stubs: { Tooltip: { props: ['content'], template: '<slot />' } } } });
    await flushPromises();
    expect(wrapper.find('[data-testid=capture-choices-loading]').exists()).toBe(false);
    if (_name === 'bar') {
      expect(wrapper.find('[data-pick=audio]').exists()).toBe(true);
      expect(wrapper.find('[data-pick=small]').exists()).toBe(true);
    } else {
      await wrapper.get('[data-testid=capture-formats-toggle]').trigger('click');
      expect(wrapper.findAll('[role=menuitem]')).toHaveLength(2);
    }
  });
});
