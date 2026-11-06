import { computed, nextTick, ref, watch, onBeforeUnmount } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { useI18n } from 'vue-i18n';
import { emptyDetails, type CaptureOffer } from '../captureTypes';
import { failReason, type FailAction } from '../helpers/captureFailure';
import { fileCategory } from '../helpers/fileCategory';
import { formatDuration } from '../helpers/units';
import { useLocaleNumber } from './useLocaleNumber';
import type { Settings } from '../tauri/types/config';
import { applyAppearance } from '../appearance';
import type { CaptureChoice, CaptureWindowAction } from '@/generated/contract';

const SLOW_MS = 5000;
const SNAPSHOT_MS = 1500;

export interface CaptureOfferHooks {
  /** The offer's id changed (a new link). `wasEditing`: the user had been typing in the bar. */
  onNewOffer?: (value: CaptureOffer, wasEditing: boolean) => void;
  /** After every applied snapshot. */
  onApplied?: () => void;
}

/** The capture windows' shared offer state: Rust's snapshot is the only read; `capture_changed` and the
 * poll below only ask again; one stale-offer guard (`apply`); `act` is the only way out. */
export function useCaptureOffer(hooks: CaptureOfferHooks = {}) {
  const { t } = useI18n();
  const { size: formatSize } = useLocaleNumber();
  const offer = ref<CaptureOffer>({ id: 0, url: '', site: '', title: '', details: emptyDetails(), failed: false, reason: null, resolving: false, feed: false });
  /** 'fetch' = the link could not be read; 'action' = Download/Options could not start. */
  const error = ref<false | 'fetch' | 'action'>(false);
  const acting = ref(false);
  const editing = ref(false);
  const slow = ref(false);
  const thumbFailed = ref(false);
  const faviconFailed = ref(false);
  let unlisten: UnlistenFn | undefined;
  let slowTimer: ReturnType<typeof setTimeout> | undefined;
  let snapshotTimer: ReturnType<typeof setInterval> | undefined;

  const details = computed(() => offer.value.details ?? emptyDetails());
  const failure = computed(() => failReason(offer.value.reason));
  const fileKind = computed(() => details.value.mediaKind === 'file' ? fileCategory(offer.value.title || offer.value.url) : undefined);
  const isPhotos = computed(() => ['image', 'carousel', 'mixed'].includes(details.value.mediaKind ?? ''));
  /** The site's own favicon (the same host the link points to; nothing goes to a third party). */
  const favicon = computed(() => {
    try {
      const url = new URL(offer.value.url);
      return url.protocol === 'https:' ? `${url.origin}/favicon.ico` : null;
    } catch {
      return null;
    }
  });

  /** "Best: 1080p MP4 · ~120 MB": what one click on Download gets you, as separate parts so each stays in
   * order inside Arabic. Computed once; the bar shows the parts, the card's facts line reuses them. */
  const bestParts = computed<string[]>(() => {
    const { bestHeight, bestExt, size, photos } = details.value;
    if (fileKind.value) {
      const category = t(`files.category.${fileKind.value}`);
      return size ? [category, formatSize(size)] : [category];
    }
    if (isPhotos.value && photos) return [t('capture.photos', { n: photos }, photos)];
    const kind = bestHeight ? `${bestHeight}p` : bestExt ? t('capture.audio') : '';
    if (!kind) return [];
    const quality = [kind, bestExt.toUpperCase()].filter(Boolean).join(' ');
    return [t('capture.best', { quality }), size ? t('sheet.size', { size: formatSize(size) }) : ''].filter(Boolean);
  });
  /** The card's one facts line: duration, then what Download best gets ("12:34 · 1080p · MP4 · 120 MB"). */
  const facts = computed<{ text: string; strong?: boolean }[]>(() => {
    const { bestHeight, bestExt, size, photos, duration } = details.value;
    if (fileKind.value || (isPhotos.value && photos)) return bestParts.value.length ? [{ text: bestParts.value.join(' · ') }] : [];
    return [
      duration ? { text: formatDuration(duration, { pad: true }) } : null,
      bestHeight ? { text: t('capture.formats.height', { n: bestHeight }), strong: true } : bestExt ? { text: t('capture.audio'), strong: true } : null,
      bestExt ? { text: bestExt.toUpperCase() } : null,
      size ? { text: formatSize(size) } : null,
    ].filter((part): part is { text: string; strong?: boolean } => !!part);
  });

  async function syncAppearance() {
    const settings = await invoke<Settings>('config_get');
    applyAppearance(settings.appearance);
  }

  let asked = 0;
  async function refresh() {
    const ask = ++asked;
    const value = await invoke<CaptureOffer>('capture_snapshot');
    if (ask === asked) apply(value);
  }
  function apply(value: CaptureOffer) {
    // Typing in the bar replaces this offer: its own late answers must not bring the old link back.
    if (editing.value && value.id === offer.value.id) return;
    if (value.id !== offer.value.id) {
      void syncAppearance().catch(() => {});
      hooks.onNewOffer?.(value, editing.value);
      editing.value = false;
    }
    if (value.url !== offer.value.url) {
      faviconFailed.value = false;
      thumbFailed.value = false;
    }
    offer.value = value;
    error.value = false;
    void nextTick(() => hooks.onApplied?.());
  }

  /** One action at a time: a double Enter must not start the download twice. */
  async function act(action: CaptureWindowAction, choice?: CaptureChoice) {
    // A failed read never offers Download (it would fail again): Enter does nothing there.
    if (action === 'download' && offer.value.failed) return;
    if (action !== 'dismiss' && action !== 'report' && (!offer.value.url || offer.value.feed || offer.value.resolving || editing.value || acting.value)) return;
    if (action !== 'dismiss') acting.value = true;
    try {
      await invoke('capture_action', { id: offer.value.id, action, choice: choice ?? null });
    } catch {
      if (action !== 'dismiss') error.value = 'action';
    } finally {
      acting.value = false;
    }
  }
  /** Failed read: Retry reads the same link again in place; Report opens Report a problem in the main window. */
  async function onFail(action: FailAction) {
    if (action === 'options' || action === 'report') return act(action);
    acting.value = true;
    try {
      await invoke('capture_retry');
    } catch {
      error.value = 'action';
    } finally {
      acting.value = false;
    }
  }

  /** Slow sites: after a few seconds say it's still working, and that Download already works (the list finishes reading). */
  watch(() => [offer.value.id, offer.value.url, !!offer.value.title, offer.value.failed] as const, ([, url, hasTitle, failed]) => {
    clearTimeout(slowTimer);
    slow.value = false;
    if (url && !hasTitle && !failed && !offer.value.feed) slowTimer = setTimeout(() => {
      slow.value = true;
    }, SLOW_MS);
  });
  /** Linux can hold capture_changed for this hidden, always-on-top window until the next input
   * (tauri#3654): while the card still reads, ask again without waiting on the event. Dismissed, reported
   * or expired, Rust has no offer (id 0): the panel takes the empty one and stops asking. */
  const waitingForRead = computed(() => !!offer.value.url && !offer.value.title && !offer.value.failed && !offer.value.feed && !offer.value.resolving);
  watch(waitingForRead, (waiting) => {
    clearInterval(snapshotTimer);
    snapshotTimer = waiting ? setInterval(() => void refresh().catch(() => {}), SNAPSHOT_MS) : undefined;
  });

  /** Listen for `capture_changed`, then take the first snapshot. Call from the window's onMounted. */
  async function start() {
    unlisten = await listen('capture_changed', () => void refresh().catch(() => {}));
    await refresh();
  }
  onBeforeUnmount(() => {
    unlisten?.();
    clearTimeout(slowTimer);
    clearInterval(snapshotTimer);
  });

  return { offer, error, acting, editing, slow, thumbFailed, faviconFailed, details, failure, fileKind, isPhotos, favicon, bestParts, facts, act, onFail, start };
}
