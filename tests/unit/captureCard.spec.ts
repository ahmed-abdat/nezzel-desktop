import { mount } from '@vue/test-utils';
import { computed, nextTick, ref } from 'vue';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import CaptureCard from '../../src/components/downloads/CaptureCard.vue';
import { emptyDetails, type CaptureOffer } from '../../src/captureTypes';
import { i18n } from '../../src/i18n';

const invoke = vi.hoisted(() => vi.fn((_cmd: string, _args?: unknown): Promise<boolean> => Promise.resolve(true)));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
const offer = ref<CaptureOffer>({ id: 1, remoteImages: false, url: 'https://example.com/video', site: 'example.com', title: '', read: false, details: emptyDetails(), failed: false, reason: null, resolving: false, feed: false });
vi.mock('../../src/composables/useCaptureOffer', () => ({
  useCaptureOffer: () => ({
    offer, details: computed(() => offer.value.details), error: ref(false), acting: ref(false), slow: ref(false),
    isPlaylist: computed(() => offer.value.feed || offer.value.details.mediaKind === 'playlist'), choicesLoading: computed(() => !offer.value.read),
    faviconFailed: ref(false), failure: ref(null), favicon: ref(null), facts: ref([]),
    act: vi.fn(), onFail: vi.fn(), start: vi.fn(),
  }),
}));

describe('copied-link card engagement', () => {
  let card: ReturnType<typeof mount>;
  const reports = () => invoke.mock.calls.filter(([cmd, args]) => cmd === 'capture_fit' && args && typeof args === 'object' && 'engaged' in args).map(([, args]) => args);
  beforeEach(() => {
    vi.useFakeTimers();
    invoke.mockClear();
    invoke.mockResolvedValue(true);
    offer.value = { ...offer.value, id: 1, title: 'Read video', read: true, details: { ...emptyDetails(), mediaKind: 'video', bestHeight: 1080, choices: [{ choice: { kind: 'video', height: 1080 }, size: null }, { choice: { kind: 'audio' }, size: null }] } };
    card = mount(CaptureCard, { attachTo: document.body, global: { plugins: [i18n], stubs: { Tooltip: { props: ['content'], template: '<slot />' } } } });
  });
  afterEach(() => {
    card.unmount();
    vi.useRealTimers();
  });

  it.each(['pointerenter', 'pointerdown', 'click'])('holds on %s, including without a hover event, until leave', async (event) => {
    await card.get('section').trigger(event);
    expect(reports().at(-1)).toEqual({ id: 1, engaged: true });
    await vi.advanceTimersByTimeAsync(20_000);
    expect(reports().at(-1)).toEqual({ id: 1, engaged: true });
    await card.get('section').trigger('pointerleave');
    expect(reports().at(-1)).toEqual({ id: 1, engaged: false });
  });

  it('keeps focus within held across child focus changes and pointer leave', async () => {
    const buttons = card.findAll('button');
    await buttons[0]!.trigger('focusin');
    await card.get('section').trigger('pointerleave');
    expect(reports().at(-1)).toEqual({ id: 1, engaged: true });
    await buttons[0]!.trigger('focusout', { relatedTarget: buttons[1]!.element });
    expect(reports().at(-1)).toEqual({ id: 1, engaged: true });
    await buttons[1]!.trigger('focusout', { relatedTarget: null });
    expect(reports().at(-1)).toEqual({ id: 1, engaged: false });
  });

  it('holds the open formats list after pointer leave, then releases when closed', async () => {
    await card.get('[data-testid=capture-formats-toggle]').trigger('click');
    await nextTick();
    (document.activeElement as HTMLElement | null)?.blur();
    await nextTick();
    expect(card.get('[data-testid=capture-formats-toggle]').attributes('aria-expanded')).toBe('true');
    await card.get('section').trigger('pointerleave');
    expect(reports().at(-1)).toEqual({ id: 1, engaged: true });
    await card.get('[data-testid=capture-formats-toggle]').trigger('click');
    await nextTick();
    (document.activeElement as HTMLElement | null)?.blur();
    await nextTick();
    await card.get('section').trigger('pointerleave');
    expect(reports().at(-1)).toEqual({ id: 1, engaged: false });
  });

  it('lets go after 30 s without pointer or key activity when the leave event never arrives', async () => {
    await card.get('section').trigger('pointerdown');
    await vi.advanceTimersByTimeAsync(20_000);
    await card.get('section').trigger('pointermove');
    await vi.advanceTimersByTimeAsync(27_000);
    expect(reports().at(-1)).toEqual({ id: 1, engaged: true });
    await vi.advanceTimersByTimeAsync(6000);
    expect(reports().at(-1)).toEqual({ id: 1, engaged: false });
  });

  it('renews engagement every three seconds and stops renewing after leave', async () => {
    await card.get('section').trigger('pointerdown');
    const before = reports().length;
    await vi.advanceTimersByTimeAsync(6000);
    expect(reports().length).toBe(before + 2);
    await card.get('section').trigger('pointerleave');
    const released = reports().length;
    await vi.advanceTimersByTimeAsync(12_000);
    expect(reports().length).toBe(released);
  });

  it('clears engagement and the open menu when the reused native card is hidden', async () => {
    await card.get('[data-testid=capture-formats-toggle]').trigger('click');
    await nextTick();
    invoke.mockResolvedValue(false);
    await vi.advanceTimersByTimeAsync(3000);
    expect(reports().at(-1)).toEqual({ id: 1, engaged: false });
    expect(card.get('[data-testid=capture-formats-toggle]').attributes('aria-expanded')).toBe('false');
    const hidden = reports().length;
    await vi.advanceTimersByTimeAsync(12_000);
    expect(reports().length).toBe(hidden);
  });

  it('does not retain focus from a format row removed by a replacement offer', async () => {
    await card.get('[data-testid=capture-formats-toggle]').trigger('click');
    await nextTick();
    expect(card.get('[data-testid=capture-formats]').element.contains(document.activeElement)).toBe(true);
    const panel = card.get('section').element;
    const matches = panel.matches.bind(panel);
    vi.spyOn(panel, 'matches').mockImplementation(selector => selector === ':hover' ? false : matches(selector));
    offer.value = { ...offer.value, id: 2 };
    await nextTick();
    await nextTick();
    expect(card.get('section').element.contains(document.activeElement)).toBe(false);
    expect(reports().at(-1)).toEqual({ id: 2, engaged: false });
  });

  it('re-derives real focus for a replacement instead of retaining the previous flags', async () => {
    const button = card.findAll('button')[0]!;
    (button.element as HTMLElement).focus();
    offer.value = { ...offer.value, id: 2 };
    await nextTick();
    expect(reports().at(-1)).toEqual({ id: 2, engaged: true });
    (button.element as HTMLElement).blur();
    await nextTick();
    expect(reports().at(-1)).toEqual({ id: 2, engaged: false });
  });

  it('clears the previous interaction for a replacement offer and releases on unmount', async () => {
    await card.get('section').trigger('pointerdown');
    offer.value = { ...offer.value, id: 2 };
    await nextTick();
    expect(reports().at(-1)).toEqual({ id: 2, engaged: false });
    card.unmount();
    expect(reports().at(-1)).toEqual({ id: 2, engaged: false });
  });
});
