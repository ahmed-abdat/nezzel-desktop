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

let wrapper: ReturnType<typeof mount>;
afterEach(() => wrapper?.unmount());

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
