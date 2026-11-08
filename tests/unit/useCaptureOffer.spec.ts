import { mount, flushPromises } from '@vue/test-utils';
import { defineComponent, onMounted } from 'vue';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { useCaptureOffer } from '../../src/composables/useCaptureOffer';
import CaptureBar from '../../src/components/downloads/CaptureBar.vue';
import CaptureCard from '../../src/components/downloads/CaptureCard.vue';
import { emptyDetails } from '../../src/captureTypes';
import { i18n } from '../../src/i18n';
import { installTauriMock } from '../utils/tauriMock';

let wrapper: ReturnType<typeof mount>;
afterEach(() => wrapper?.unmount());

describe('capture primary action', () => {
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
