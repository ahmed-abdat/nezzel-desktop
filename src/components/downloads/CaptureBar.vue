<template>
  <!-- Spotlight-style quick bar. Fills its native window, which follows the content's height. -->
  <section
    ref="panel"
    :aria-label="t('capture.quickBar')"
    class="capture-panel flex flex-col overflow-hidden rounded-xl border border-hairline bg-background text-foreground gap-3 p-4"
    :style="{ fontSize: rtl ? '14px' : '13px' }"
    @keydown="onPickKey"
  >
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
            @input="edit"
            @paste="onPaste" />
          <span v-if="offer.resolving || (offer.url && !offer.feed && !offer.read && !offer.failed)" class="size-3.5 shrink-0 animate-spin rounded-full border-2 border-primary/25 border-t-primary motion-reduce:animate-none" role="status" :aria-label="t('capture.loading')" />
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
            <img v-else-if="offer.remoteImages && details.thumbnail && !thumbFailed" :src="details.thumbnail" alt="" class="size-full object-cover" referrerpolicy="no-referrer" @error="thumbFailed = true" />
            <div v-else class="flex size-full items-center justify-center text-muted-foreground/60">
              <ImageIcon v-if="isPhotos" class="size-5" aria-hidden="true" />
              <Film v-else class="size-5" aria-hidden="true" />
            </div>
            <span v-if="details.duration" class="tnum absolute bottom-1 end-1 rounded-[4px] bg-black/70 px-1 text-[11px] font-medium leading-4 text-white" dir="ltr">{{ formatDuration(details.duration, { pad: true }) }}</span>
          </div>
          <div class="flex min-w-0 flex-1 flex-col gap-0.5">
            <p class="line-clamp-2 text-start text-[13px] font-medium leading-snug"><bdi dir="auto">{{ offer.title || (offer.read ? offer.site : slow ? t('capture.stillReading') : t('capture.loading')) }}</bdi></p>
            <p v-if="slow && !offer.read" class="text-[12px] text-muted-foreground" role="status">{{ t('capture.slowHint') }}</p>
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
              <template v-for="(part, index) in bestParts" :key="index">
                <span v-if="index > 0" class="nz-sep" aria-hidden="true" />
                <i18n-t v-if="part.estimate" keypath="sheet.size" tag="span" scope="global"><template #size><bdi dir="ltr">{{ part.text }}</bdi></template></i18n-t>
                <bdi v-else :dir="part.ltr ? 'ltr' : undefined">{{ part.text }}</bdi>
              </template>
            </p>
          </div>
        </div>
        <p v-else-if="offer.resolving" class="text-[12.5px] text-muted-foreground" role="status">{{ t('capture.readingBrowser') }}</p>
        <p v-else-if="error" class="text-[12.5px] text-destructive" role="alert">{{ t(error === 'action' ? 'capture.actionFailed' : 'capture.failed') }}</p>
        <CaptureFailure v-else-if="offer.failed" :reason="failure" :site="offer.site" />
        <p v-else-if="text && !offer.url" class="text-[12.5px] text-muted-foreground" role="status">{{ t('capture.unsupported') }}</p>
        <p v-else class="text-[12.5px] text-muted-foreground">{{ t('capture.emptyHint') }}</p>
      </div>

      <div class="flex flex-wrap items-center gap-2">
        <p v-if="isMac && offer.resolving" class="min-w-0 flex-1 truncate text-[11.5px] text-muted-foreground" :title="t('capture.automationHint')">{{ t('capture.automationHint') }}</p>
        <span v-else class="inline-flex min-w-max flex-1 items-center gap-1 whitespace-nowrap text-[11.5px] text-muted-foreground">
          <Kbd>{{ escKey }}</Kbd>{{ t('capture.escCloses') }}
        </span>
        <CaptureFailActions v-if="offer.failed && !editing" :reason="failure" :busy="acting" @act="onFail" />
        <template v-else>
          <Button type="button" variant="ghost" size="sm" :disabled="!ready" @click="act('options')">{{ t('capture.options') }}</Button>
          <CapturePicks :picks="picks" :loading="choicesLoading" :choose-items="isPlaylist" :disabled="!ready" :acting="acting" submit @pick="pick" />
        </template>
      </div>
    </form>
  </section>
</template>

<script setup lang="ts">
import Tooltip from '../ui/tooltip/Tooltip.vue';
import { Input, type InputApi } from '../ui/input';
import FileTypeIcon from './FileTypeIcon.vue';
import CapturePicks from './CapturePicks.vue';
import CaptureFailActions from './CaptureFailActions.vue';
import CaptureFailure from './CaptureFailure.vue';
import { capturePicks, pickChoice, type PickKey } from '../../helpers/captureChoices';
import { computed, nextTick, ref, onMounted, onBeforeUnmount } from 'vue';
import { Film, Image as ImageIcon, Link2, X } from '@lucide/vue';
import { Button } from '@/components/ui/button';
import Kbd from '@/components/ui/kbd/Kbd.vue';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { LogicalSize } from '@tauri-apps/api/dpi';
import { useI18n } from 'vue-i18n';
import { usePlatform } from '../../composables/usePlatform';
import { useCaptureOffer } from '../../composables/useCaptureOffer';
import { formatDuration } from '../../helpers/units';
import { localeDirection } from '../../i18n';
import { CAPTURE_BAR_SIZE } from '@/generated/contract';

const { t, locale } = useI18n();
const { isMac } = usePlatform();
const text = ref('');
const input = ref<InputApi>();
const panel = ref<HTMLElement>();
let editTimer: ReturnType<typeof setTimeout> | undefined;
let sizer: ResizeObserver | undefined;

const { offer, error, acting, editing, slow, thumbFailed, faviconFailed, details, failure, fileKind, isPhotos, isPlaylist, choicesLoading, favicon, bestParts, act, onFail, start } = useCaptureOffer({
  onNewOffer(value, wasEditing) {
    if (wasEditing) return;
    text.value = value.url;
    void nextTick(selectPrefill);
  },
  onApplied: fitBar, // the window may have been hidden when the observer last fired
});

const rtl = computed(() => localeDirection(locale.value) === 'rtl');
const ready = computed(() => !!offer.value.url && !offer.value.resolving && !editing.value && !acting.value);
const compactCount = (value: number) => new Intl.NumberFormat(locale.value, { notation: 'compact', maximumFractionDigits: 1 }).format(value);
const escKey = computed(() => (isMac.value ? 'esc' : 'Esc'));

/** A prefilled link is selected, so typing or pasting replaces it instead of appending a second copy. */
function selectPrefill() {
  if (text.value && document.activeElement?.id === 'capture-url') input.value?.select();
}
/** Pasting the link that is already in the field (Ctrl+V after the bar prefilled it) changes nothing. */
function onPaste(event: ClipboardEvent) {
  const pasted = event.clipboardData?.getData('text/plain').trim();
  if (pasted && pasted === text.value.trim()) event.preventDefault();
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
function fitBar() {
  const height = Math.ceil(panel.value?.scrollHeight ?? 0);
  // A hidden webview can report innerWidth 0: fall back to the width Rust built the window with.
  if (height > 0) void getCurrentWindow().setSize(new LogicalSize(window.innerWidth || CAPTURE_BAR_SIZE.width, height)).catch(() => {});
}

/** Offer only known picks; the primary default action stays available during the read. */
const picks = computed(() => isPlaylist.value ? [] : capturePicks(details.value, offer.value.read, offer.value.url));
function pick(key: PickKey) {
  void act('download', pickChoice(key));
}
/** 1/2/3 start a pick (not while typing in the field); the number never changes meaning after the read. */
function onPickKey(event: KeyboardEvent) {
  if (!['1', '2', '3'].includes(event.key) || event.metaKey || event.ctrlKey || event.altKey) return;
  if ((event.target as HTMLElement | null)?.closest('input, textarea')) return;
  const key = (['best', 'audio', 'small'] as const)[Number(event.key) - 1];
  if (key !== 'best' && !picks.value.some(p => p.key === key)) return;
  event.preventDefault();
  pick(key);
}

/** Esc closes from anywhere in the window (input, buttons, or nothing focused), not only inside the panel. */
function onEsc(event: KeyboardEvent) {
  if (event.key !== 'Escape' || event.defaultPrevented) return;
  event.preventDefault();
  void act('dismiss');
}
onMounted(async () => {
  window.addEventListener('keydown', onEsc);
  await start();
  input.value?.focus();
  selectPrefill();
  if (panel.value && typeof ResizeObserver !== 'undefined') {
    sizer = new ResizeObserver(fitBar);
    sizer.observe(panel.value);
  }
});
onBeforeUnmount(() => {
  window.removeEventListener('keydown', onEsc);
  clearTimeout(editTimer);
  sizer?.disconnect();
});
</script>
