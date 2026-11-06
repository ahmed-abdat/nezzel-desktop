<template>
  <!-- Small copied-link card. Fills its native window; the format list grows the window below the card part. -->
  <section
    ref="panel"
    :aria-label="t('capture.copiedLink')"
    class="capture-panel flex flex-col overflow-hidden rounded-xl border border-hairline bg-background text-foreground h-screen"
    :style="{ fontSize: rtl ? '14px' : '13px' }"
  >
    <!-- Card part keeps the closed window height minus the 1px borders whether or not the format list is open; the list grows the window below it. -->
    <div class="flex shrink-0 flex-col p-3" :style="{ height: `${CAPTURE_CARD_SIZE.height - 2}px` }">
    <div class="mb-2 flex h-5 items-center gap-2">
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
      <p class="line-clamp-2 text-start text-[13px] leading-[1.4] font-medium rtl:leading-[1.5]"><bdi dir="auto">{{ offer.title }}</bdi></p>
      <p v-if="facts.length" class="tnum mt-0.5 flex min-w-0 items-center gap-1.5 overflow-hidden text-[12px] whitespace-nowrap text-muted-foreground" data-testid="capture-best">
        <template v-for="(part, index) in facts" :key="index"><span v-if="index > 0" class="nz-sep" aria-hidden="true" /><bdi :class="part.strong && 'font-medium text-foreground'">{{ part.text }}</bdi></template>
      </p>
    </template>
    <CaptureFailure v-else-if="offer.failed" :reason="failure" :site="offer.site" />
    <p v-else class="text-[12.5px] text-muted-foreground" role="status">{{ slow ? t('capture.stillReading') : t('capture.loading') }}</p>
    <div v-if="!offer.feed" class="mt-auto flex items-center justify-end gap-1.5">
      <CaptureFailActions v-if="offer.failed" :reason="failure" :busy="acting" @act="onFail" />
      <template v-else>
        <Button variant="ghost" size="sm" class="-ms-2 me-auto px-2 text-muted-foreground" :disabled="!offer.url || acting" @click="act('options')">{{ t('capture.options') }}</Button>
        <!-- Download best | ▾: one tap stays the fastest path; ▾ opens every format the link offers. -->
        <div class="flex" data-testid="capture-picks">
          <Button
            size="sm"
            :class="rows.length ? 'rounded-e-none' : undefined"
            :disabled="!offer.url || acting"
            :aria-busy="acting || undefined"
            aria-keyshortcuts="1 Enter"
            data-pick="best"
            @click="pick('best')"
          >
            <LoaderCircle v-if="acting" class="animate-spin" aria-hidden="true" /><ArrowDownToLine v-else aria-hidden="true" />
            {{ acting ? t('capture.starting') : rows.length ? t('capture.downloadBest') : t('capture.pick.download') }}
          </Button>
          <Button
            v-if="rows.length"
            ref="caret"
            size="icon"
            class="rounded-s-none shadow-[inset_1px_0_0_color-mix(in_oklab,var(--primary-foreground)_28%,transparent)] rtl:shadow-[inset_-1px_0_0_color-mix(in_oklab,var(--primary-foreground)_28%,transparent)]"
            :class="formatsOpen ? 'bg-primary-hover' : undefined"
            :disabled="!offer.url || acting"
            :aria-label="t('capture.formats.choose')"
            aria-haspopup="menu"
            :aria-expanded="formatsOpen"
            :aria-controls="formatsOpen ? 'capture-formats' : undefined"
            data-testid="capture-formats-toggle"
            @click="toggleFormats"
            @keydown.down.prevent="openFormats(true)"
          >
            <ChevronDown class="transition-transform duration-[var(--nz-dur-fast,120ms)]" :class="formatsOpen && 'rotate-180'" aria-hidden="true" />
          </Button>
        </div>
      </template>
    </div>
    </div>
    <CaptureFormats
      v-if="formatsOpen && rows.length"
      id="capture-formats"
      ref="formatList"
      :rows="rows"
      :best-ext="details.bestExt"
      :disabled="!offer.url || acting"
      @pick="pickRow"
      @close="closeFormats(true)"
    />
  </section>
</template>

<script setup lang="ts">
import Tooltip from '../ui/tooltip/Tooltip.vue';
import CaptureFormats from './CaptureFormats.vue';
import CaptureFailActions from './CaptureFailActions.vue';
import CaptureFailure from './CaptureFailure.vue';
import { cardRows, pickChoice, type PickKey } from '../../helpers/captureChoices';
import { computed, nextTick, ref, watch, onMounted, onBeforeUnmount } from 'vue';
import { ArrowDownToLine, ChevronDown, ClipboardCheck, LoaderCircle, X } from '@lucide/vue';
import { Button } from '@/components/ui/button';
import { invoke } from '@tauri-apps/api/core';
import { useI18n } from 'vue-i18n';
import { useCaptureOffer } from '../../composables/useCaptureOffer';
import { localeDirection } from '../../i18n';
import { CAPTURE_CARD_SIZE, type CaptureChoice } from '@/generated/contract';

const { t, locale } = useI18n();
const panel = ref<HTMLElement>();
const { offer, error, acting, slow, faviconFailed, details, failure, favicon, facts, act, onFail, start } = useCaptureOffer();

const rtl = computed(() => localeDirection(locale.value) === 'rtl');
/** The card's ▾ format menu: rows for this link (generic ceilings before the read), [] = plain Download. */
const rows = computed(() => cardRows(details.value, !!offer.value.title));
const formatsOpen = ref(false);
const formatList = ref<{ focusFirst: () => void } | null>(null);
const caret = ref<{ $el: HTMLElement } | null>(null);
/** Grow (or shrink) the native window to the card plus the open list; Rust keeps it on its screen corner. */
function fitCard() {
  const list = formatsOpen.value ? (panel.value?.querySelector<HTMLElement>('#capture-formats')?.offsetHeight ?? 0) : 0;
  void invoke('capture_fit', { height: CAPTURE_CARD_SIZE.height + list }).catch(() => {});
}
async function openFormats(fromKeyboard = false) {
  if (!rows.value.length || formatsOpen.value) return;
  formatsOpen.value = true;
  await nextTick();
  fitCard();
  if (fromKeyboard) formatList.value?.focusFirst();
}
function closeFormats(refocus = false) {
  if (!formatsOpen.value) return;
  formatsOpen.value = false;
  void nextTick(fitCard);
  if (refocus) caret.value?.$el.focus();
}
function toggleFormats(event: MouseEvent) {
  if (formatsOpen.value) closeFormats();
  // detail 0 = Enter/Space on the button: move focus into the list like a native menu.
  else void openFormats(event.detail === 0);
}
/** Esc closes the open format list first (focus may still be on ▾ after a pointer open), then the card. */
function onEscape() {
  if (formatsOpen.value) closeFormats(true);
  else void act('dismiss');
}
function pick(key: PickKey) {
  void act('download', pickChoice(key));
}
function pickRow(choice: CaptureChoice) {
  void act('download', choice);
}
// A new link (or rows that vanish after the read, e.g. it turned out to be a photo) closes the list.
watch(() => [offer.value.id, rows.value.length] as const, ([id, count], [oldId]) => {
  if (id !== oldId || !count) formatsOpen.value = false;
});

/** Esc closes from anywhere in the window (input, buttons, or nothing focused), not only inside the panel. */
function onEsc(event: KeyboardEvent) {
  if (event.key !== 'Escape' || event.defaultPrevented) return;
  event.preventDefault();
  onEscape();
}
onMounted(async () => {
  window.addEventListener('keydown', onEsc);
  await start();
});
onBeforeUnmount(() => window.removeEventListener('keydown', onEsc));
</script>
