<template>
  <!-- Spotlight-style quick bar (capture-bar) or the small copied-link card (capture-card). Fills its native window. -->
  <section
    ref="panel"
    :aria-label="t(isBar ? 'capture.quickBar' : 'capture.copiedLink')"
    class="capture-panel flex flex-col overflow-hidden rounded-xl border border-hairline bg-background text-foreground"
    :class="isBar ? 'gap-3 p-4' : 'h-screen gap-2 p-3.5'"
    :style="{ fontSize: rtl ? '14px' : '13px' }"
    @keydown.esc.prevent="act('dismiss')"
  >
    <template v-if="isBar">
      <form class="flex flex-col gap-3" @submit.prevent="act('download')">
        <label for="capture-url" class="sr-only">{{ t('capture.url') }}</label>
        <div class="flex items-center gap-2">
          <div class="flex h-11 min-w-0 flex-1 items-center gap-2.5 rounded-lg bg-foreground/[0.05] px-3 ring-primary/40 focus-within:bg-card focus-within:ring-2">
            <img v-if="favicon && !faviconFailed" :src="favicon" alt="" class="size-4 shrink-0 rounded-sm" @error="faviconFailed = true" />
            <Link2 v-else class="size-4 shrink-0 text-muted-foreground rtl:-scale-x-100" aria-hidden="true" />
            <Input variant="bare"
              id="capture-url"
              ref="input"
              v-model="text"
              dir="ltr"
              type="url"
              autocomplete="off"
              spellcheck="false"
              :placeholder="t('capture.placeholder')"
              class="h-full min-w-0 flex-1 bg-transparent text-[15px] outline-none placeholder:text-muted-foreground/70"
              @input="edit" />
            <span v-if="offer.resolving || (offer.url && !offer.feed && !offer.title && !offer.failed)" class="size-3.5 shrink-0 animate-spin rounded-full border-2 border-primary/25 border-t-primary motion-reduce:animate-none" role="status" :aria-label="t('capture.loading')" />
          </div>
          <tooltip :content="t('capture.dismiss')">
            <Button type="button" variant="ghost" size="icon" class="size-9 shrink-0 text-muted-foreground" :aria-label="t('capture.dismiss')" data-testid="capture-close" @click="act('dismiss')">
              <X aria-hidden="true" />
            </Button>
          </tooltip>
        </div>

        <div class="min-h-[63px]">
          <p v-if="offer.feed" class="text-[12.5px] text-foreground/80 text-pretty" role="status" data-testid="capture-feed">{{ t('capture.feedPage') }}</p>
          <div v-else-if="offer.url && !offer.failed && !error" class="flex gap-3" data-testid="capture-preview">
            <div class="relative h-[63px] w-28 shrink-0 overflow-hidden rounded-md bg-muted">
              <FileTypeIcon v-if="fileKind" :category="fileKind" class="size-full p-4 text-muted-foreground" />
              <img v-else-if="details.thumbnail && !thumbFailed" :src="details.thumbnail" alt="" class="size-full object-cover" referrerpolicy="no-referrer" @error="thumbFailed = true" />
              <div v-else class="flex size-full items-center justify-center text-muted-foreground/60">
                <ImageIcon v-if="isPhotos" class="size-5" aria-hidden="true" />
                <Film v-else class="size-5" aria-hidden="true" />
              </div>
              <span v-if="details.duration" class="tnum absolute bottom-1 end-1 rounded-[4px] bg-black/70 px-1 text-[11px] font-medium leading-4 text-white" dir="ltr">{{ formatDuration(details.duration) }}</span>
            </div>
            <div class="flex min-w-0 flex-1 flex-col gap-0.5">
              <p class="line-clamp-2 text-start text-[13px] font-medium leading-snug"><bdi dir="auto">{{ offer.title || (slow ? t('capture.stillReading') : t('capture.loading')) }}</bdi></p>
              <p v-if="slow && !offer.title" class="text-[12px] text-muted-foreground" role="status">{{ t('capture.slowHint') }}</p>
              <p v-if="details.uploader || offer.site" class="flex min-w-0 items-center gap-1 text-[12px] text-muted-foreground">
                <bdi v-if="details.uploader" class="truncate">{{ details.uploader }}</bdi>
                <span v-if="details.uploader && offer.site" class="nz-sep" aria-hidden="true" />
                <bdi v-if="offer.site" class="shrink-0">{{ offer.site }}</bdi>
                <template v-if="details.views != null">
                  <span class="nz-sep" aria-hidden="true" />
                  <span class="tnum shrink-0">{{ t('capture.views', { count: compactCount(details.views) }) }}</span>
                </template>
              </p>
              <p v-if="bestParts.length" class="tnum flex items-center gap-1 text-[12px] text-foreground/75" data-testid="capture-best">
                <template v-for="(part, index) in bestParts" :key="index"><span v-if="index > 0" class="nz-sep" aria-hidden="true" /><bdi>{{ part }}</bdi></template>
              </p>
            </div>
          </div>
          <p v-else-if="offer.resolving" class="text-[12.5px] text-muted-foreground" role="status">{{ t('capture.readingBrowser') }}</p>
          <p v-else-if="error" class="text-[12.5px] text-destructive" role="alert">{{ t(error === 'action' ? 'capture.actionFailed' : 'capture.failed') }}</p>
          <p v-else-if="offer.failed" class="text-[12.5px] text-destructive" role="alert">{{ t('capture.failed') }}</p>
          <p v-else-if="text && !offer.url" class="text-[12.5px] text-muted-foreground" role="status">{{ t('capture.unsupported') }}</p>
          <p v-else class="text-[12.5px] text-muted-foreground">{{ t('capture.emptyHint') }}</p>
        </div>

        <div class="flex items-center gap-2">
          <p v-if="isMac && offer.resolving" class="min-w-0 flex-1 truncate text-[11.5px] text-muted-foreground" :title="t('capture.automationHint')">{{ t('capture.automationHint') }}</p>
          <span v-else class="inline-flex flex-1 items-center gap-1 text-[11.5px] text-muted-foreground">
            <Kbd>{{ escKey }}</Kbd>{{ t('capture.escCloses') }}
          </span>
          <Button type="button" variant="outline" size="sm" :disabled="!ready" @click="act('options')">{{ t('capture.options') }}</Button>
          <Button type="submit" size="sm" :disabled="!ready" :aria-busy="acting || undefined">
            <LoaderCircle v-if="acting" class="animate-spin" aria-hidden="true" /><ArrowDownToLine v-else aria-hidden="true" />{{ acting ? t('capture.starting') : t('capture.downloadBest') }}
          </Button>
        </div>
      </form>
    </template>

    <template v-else>
      <div class="flex items-center gap-2">
        <img v-if="favicon && !faviconFailed" :src="favicon" alt="" class="size-4 shrink-0 rounded-sm" @error="faviconFailed = true" />
        <ClipboardCheck v-else class="size-4 shrink-0 text-primary" aria-hidden="true" />
        <p class="min-w-0 flex-1 truncate text-[12px] font-medium text-muted-foreground"><bdi>{{ offer.site || t('capture.copiedLink') }}</bdi></p>
        <tooltip :content="t('capture.dismiss')">
          <Button variant="ghost" size="icon-xs" class="-me-1.5 text-muted-foreground" :aria-label="t('capture.dismiss')" @click="act('dismiss')">
            <X aria-hidden="true" />
          </Button>
        </tooltip>
      </div>
      <p v-if="offer.feed" class="text-[12.5px] text-foreground/80 text-pretty" role="status" data-testid="capture-feed">{{ t('capture.feedPage') }}</p>
      <p v-else-if="error" class="text-[12.5px] text-destructive" role="alert">{{ t(error === 'action' ? 'capture.actionFailed' : 'capture.failed') }}</p>
      <template v-else-if="offer.title">
        <p class="line-clamp-2 text-start text-[13px] font-medium"><bdi dir="auto">{{ offer.title }}</bdi></p>
        <p v-if="bestParts.length" class="tnum flex items-center gap-1 text-[12px] text-muted-foreground" data-testid="capture-best">
          <template v-for="(part, index) in bestParts" :key="index"><span v-if="index > 0" class="nz-sep" aria-hidden="true" /><bdi>{{ part }}</bdi></template>
        </p>
      </template>
      <p v-else-if="offer.failed" class="text-[12.5px] text-destructive" role="alert">{{ t('capture.failed') }}</p>
      <p v-else class="text-[12.5px] text-muted-foreground" role="status">{{ slow ? t('capture.stillReading') : t('capture.loading') }}</p>
      <div v-if="!offer.feed" class="mt-auto flex items-center justify-end gap-2">
        <Button variant="outline" size="sm" :disabled="!offer.url || acting" @click="act('options')">{{ t('capture.options') }}</Button>
        <Button size="sm" :disabled="!offer.url || acting" :aria-busy="acting || undefined" @click="act('download')">
          <LoaderCircle v-if="acting" class="animate-spin" aria-hidden="true" /><ArrowDownToLine v-else aria-hidden="true" />{{ acting ? t('capture.starting') : t('capture.downloadBest') }}
        </Button>
      </div>
    </template>
  </section>
</template>

<script setup lang="ts">
import Tooltip from '../ui/tooltip/Tooltip.vue';
import { Input, type InputApi } from '../ui/input';
import FileTypeIcon from './FileTypeIcon.vue';
import { fileCategory } from '../../helpers/fileCategory';
import { computed, nextTick, ref, watch, onMounted, onBeforeUnmount } from 'vue';
import { ArrowDownToLine, ClipboardCheck, Film, Image as ImageIcon, Link2, LoaderCircle, X } from '@lucide/vue';
import { Button } from '@/components/ui/button';
import Kbd from '@/components/ui/kbd/Kbd.vue';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { LogicalSize } from '@tauri-apps/api/dpi';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { useI18n } from 'vue-i18n';
import { usePlatform } from '../../composables/usePlatform';
import { emptyDetails, type CaptureOffer } from '../../captureTypes';
import { formatDuration } from '../../composables/useDuration';
import { useLocaleNumber } from '../../composables/useLocaleNumber';
import type { Settings } from '../../tauri/types/config';
import { applyDocumentLocale, getDefaultLocale, localeDirection, type Locale } from '../../i18n';
import { applyTheme } from '../../composables/useTheme';

const { t, locale } = useI18n();
const { isMac } = usePlatform();
const isBar = getCurrentWindow().label === 'capture-bar';
const offer = ref<CaptureOffer>({ id: 0, url: '', site: '', title: '', details: emptyDetails(), failed: false, resolving: false, feed: false });
const { size: formatSize } = useLocaleNumber();
const thumbFailed = ref(false);
const text = ref('');
/** 'fetch' = the link could not be read; 'action' = Download/Options could not start. */
const error = ref<false | 'fetch' | 'action'>(false);
const acting = ref(false);
const editing = ref(false);
const input = ref<InputApi>();
const panel = ref<HTMLElement>();
let unlisten: UnlistenFn | undefined;
let stopMetadata: UnlistenFn | undefined;
let editTimer: ReturnType<typeof setTimeout> | undefined;
let sizer: ResizeObserver | undefined;

const rtl = computed(() => localeDirection(locale.value) === 'rtl');
const ready = computed(() => !!offer.value.url && !offer.value.feed && !offer.value.resolving && !editing.value && !acting.value);
const details = computed(() => offer.value.details ?? emptyDetails());
const compactCount = (value: number) => new Intl.NumberFormat(locale.value, { notation: 'compact', maximumFractionDigits: 1 }).format(value);
/** "Best: 1080p MP4 · ~120 MB": what one click on Download gets you. */
const fileKind = computed(() => details.value.mediaKind === 'file' ? fileCategory(offer.value.title || offer.value.url) : undefined);
const isPhotos = computed(() => ['image', 'carousel', 'mixed'].includes(details.value.mediaKind ?? ''));
const best = computed(() => {
  const { bestHeight, bestExt, size, photos } = details.value;
  if (fileKind.value) {
    const category = t(`files.category.${fileKind.value}`);
    return size ? t('files.withSize', { category, size: formatSize(size) }) : category;
  }
  if (isPhotos.value && photos) return t('capture.photos', { n: photos }, photos);
  const kind = bestHeight ? `${bestHeight}p` : bestExt ? t('capture.audio') : '';
  if (!kind) return '';
  const quality = [kind, bestExt.toUpperCase()].filter(Boolean).join(' ');
  return [t('capture.best', { quality }), size ? t('sheet.size', { size: formatSize(size) }) : ''].filter(Boolean).join(' · ');
});
/** Each part is isolated, so "1080p MP4" and "~120 MB" keep their order inside Arabic. */
const bestParts = computed(() => (best.value ? best.value.split(' · ') : []));
const escKey = computed(() => (isMac.value ? 'esc' : 'Esc'));
/** The site's own favicon (the same host the link points to; nothing goes to a third party). */
const faviconFailed = ref(false);
const favicon = computed(() => {
  try {
    const url = new URL(offer.value.url);
    return url.protocol === 'https:' ? `${url.origin}/favicon.ico` : null;
  } catch {
    return null;
  }
});

async function syncAppearance() {
  const settings = await invoke<Settings>('config_get');
  const language = settings.appearance.language === 'system' ? getDefaultLocale() : settings.appearance.language as Locale;
  locale.value = language;
  applyDocumentLocale(language);
  if (settings.appearance.theme !== 'system') applyTheme(settings.appearance.theme);
  else document.body.removeAttribute('data-theme');
}

function update(value: CaptureOffer) {
  if (value.id < offer.value.id || (editing.value && value.id === offer.value.id)) return;
  if (value.id !== offer.value.id) {
    void syncAppearance().catch(() => {});
    if (!editing.value) text.value = value.url;
    editing.value = false;
  }
  if (value.url !== offer.value.url) {
    faviconFailed.value = false;
    thumbFailed.value = false;
  }
  offer.value = value;
  error.value = false;
  void nextTick(fitBar); // the window may have been hidden when the observer last fired
}
function edit() {
  clearTimeout(editTimer);
  editing.value = true;
  error.value = false;
  // Invalidate the previous URL immediately; Enter must never download a stale link.
  offer.value.url = '';
  editTimer = setTimeout(() => {
    void invoke('capture_edit', { text: text.value }).catch(() => {
      error.value = 'fetch';
    });
  }, 300);
}
/** The quick bar's native window follows its content (Spotlight-style), so no empty slab shows under the actions. */
const BAR_WIDTH = 620; // matches the native window built in capture/mod.rs; a hidden webview can report innerWidth 0
function fitBar() {
  if (!isBar) return;
  const height = Math.ceil(panel.value?.scrollHeight ?? 0);
  if (height > 0) void getCurrentWindow().setSize(new LogicalSize(window.innerWidth || BAR_WIDTH, height)).catch(() => {});
}
/** One action at a time: a double Enter must not start the download twice. */
async function act(action: 'download' | 'options' | 'dismiss') {
  if (action !== 'dismiss' && (!offer.value.url || offer.value.feed || offer.value.resolving || editing.value || acting.value)) return;
  if (action !== 'dismiss') acting.value = true;
  try {
    await invoke('capture_action', { id: offer.value.id, action });
  } catch {
    if (action !== 'dismiss') error.value = 'action';
  } finally {
    acting.value = false;
  }
}

/** Slow sites: after a few seconds say it's still working, and that Download already works (the list finishes reading). */
const SLOW_MS = 5000;
const slow = ref(false);
let slowTimer: ReturnType<typeof setTimeout> | undefined;
watch(() => [offer.value.id, offer.value.url, !!offer.value.title, offer.value.failed] as const, ([, url, hasTitle, failed]) => {
  clearTimeout(slowTimer);
  slow.value = false;
  if (url && !hasTitle && !failed && !offer.value.feed) slowTimer = setTimeout(() => {
    slow.value = true;
  }, SLOW_MS);
});
onMounted(async () => {
  unlisten = await listen<CaptureOffer>('capture_update', event => update(event.payload));
  stopMetadata = await listen<Pick<CaptureOffer, 'id' | 'title' | 'details' | 'failed'>>('capture_metadata', ({ payload }) => {
    if (payload.id !== offer.value.id) return;
    Object.assign(offer.value, payload);
    void nextTick(fitBar);
  });
  update(await invoke<CaptureOffer>('capture_snapshot'));
  input.value?.focus();
  if (isBar && panel.value && typeof ResizeObserver !== 'undefined') {
    sizer = new ResizeObserver(fitBar);
    sizer.observe(panel.value);
  }
});
onBeforeUnmount(() => {
  unlisten?.();
  stopMetadata?.();
  clearTimeout(editTimer);
  clearTimeout(slowTimer);
  sizer?.disconnect();
});
</script>
