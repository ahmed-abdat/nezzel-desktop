<template>
  <figure ref="box" class="wc nz-wanim" :class="{ 'is-static': mode === 'static', 'is-paused': mode === 'paused' }" data-testid="welcome-clip">
    <!-- A plain screen: a menu bar, an app with a link. The card lands in its real corner, then the Nezzel window rises. -->
    <div class="wc-stage" :class="narrow && 'is-narrow'" :style="fit" role="img" :aria-label="t('welcome.clip.alt')">
      <div class="wc-bar" aria-hidden="true">
        <i style="width:14px" /><i style="width:34px" /><i style="width:28px" /><i style="width:40px" /><span class="flex-1" /><i style="width:16px" /><i style="width:16px" /><i style="width:46px" />
      </div>

      <div class="wc-app" aria-hidden="true">
        <i style="width:260px" /><i style="width:190px" />
        <div class="wc-copy">
          <span class="wc-pill"><LinkIcon class="wc-pill-ic rtl:-scale-x-100" /><bdi dir="ltr" class="wc-sel">{{ LINK }}</bdi></span>
          <span class="wc-keys" dir="ltr"><span v-for="k in keys" :key="k" class="wc-kbd">{{ k }}</span></span>
        </div>
        <i style="width:230px" /><i style="width:150px" />
      </div>

      <!-- The copied-link card (CaptureCard.vue), at its real 360x160 size. -->
      <section class="wc-card" aria-hidden="true">
        <div class="mb-2 flex h-5 items-center gap-2">
          <svg viewBox="0 0 24 24" fill="currentColor" class="size-4 shrink-0"><path :d="logo" /></svg>
          <p class="min-w-0 flex-1 text-[12px] font-medium text-muted-foreground"><bdi>{{ SITE }}</bdi></p>
          <span :class="buttonVariants({ variant: 'ghost', size: 'icon-xs' })" class="-me-1.5 text-muted-foreground"><X /></span>
        </div>
        <p class="text-start text-[13px] leading-[1.4] font-medium rtl:leading-[1.5]"><bdi dir="auto">{{ TITLE }}</bdi></p>
        <p class="tnum mt-0.5 flex items-center gap-1.5 text-[12px] whitespace-nowrap text-muted-foreground">
          <template v-for="(fact, i) in FACTS" :key="i"><span v-if="i" class="nz-sep" /><bdi dir="ltr" :class="i === 1 && 'font-medium text-foreground'">{{ fact }}</bdi></template>
        </p>
        <div class="mt-auto flex items-center justify-end gap-1.5">
          <span :class="buttonVariants({ variant: 'ghost', size: 'sm' })" class="-ms-2 me-auto px-2 text-muted-foreground">{{ t('capture.options') }}</span>
          <div class="relative flex w-[208px] shrink-0">
            <span :class="buttonVariants({ size: 'sm' })" class="wc-best min-w-0 flex-1 rounded-e-none">
              <span class="wc-lbl wc-lbl-1"><ArrowDownToLine />{{ t('capture.downloadBest') }}</span>
              <span class="wc-lbl wc-lbl-2"><LoaderCircle class="wc-spin" />{{ t('capture.starting') }}</span>
            </span>
            <span :class="buttonVariants({ size: 'icon' })" class="wc-caret rounded-s-none"><ChevronDown /></span>
            <svg class="wc-cursor" viewBox="0 0 18 24"><path d="M1.5 1.5v17.2l4.4-4.1 2.9 6.6 3-1.3-2.9-6.5h6.1z" fill="#111" stroke="#fff" stroke-width="1.4" stroke-linejoin="round" /></svg>
          </div>
        </div>
      </section>

      <!-- The Nezzel window rises with the row in the list (DownloadRow.vue). -->
      <div ref="win" class="wc-win" aria-hidden="true">
        <div class="wc-win-bar">
          <span v-if="os === 'mac'" class="wc-lights" dir="ltr"><i /><i /><i /></span>
          <span class="wc-pill wc-paste"><LinkIcon class="wc-pill-ic rtl:-scale-x-100" /><span>{{ t('downloads.empty.cta') }}</span></span>
        </div>
        <div class="wc-row">
          <span class="wc-thumb"><svg viewBox="0 0 24 24" fill="currentColor" class="size-5"><path :d="logo" /></svg></span>
          <div class="min-w-0 flex-1">
            <p class="truncate leading-5 font-medium"><bdi dir="auto">{{ TITLE }}</bdi></p>
            <div class="wc-meta">
              <div class="wc-run"><span class="wc-track"><i class="wc-fill" /></span><bdi dir="ltr" class="tnum">{{ `${pct}%` }}</bdi></div>
              <div class="wc-done" data-testid="welcome-clip-saved">
                <span class="wc-ok"><Check />{{ t('downloads.row.saved') }}</span><template v-for="fact in FACTS.slice(1)" :key="fact"><span class="nz-sep" /><bdi dir="ltr">{{ fact }}</bdi></template>
              </div>
            </div>
          </div>
          <div class="wc-act">
            <span class="wc-round"><Pause /></span>
            <span :class="buttonVariants({ variant: 'outline' })" class="wc-open">{{ t('downloads.row.open') }}</span>
          </div>
        </div>
      </div>
    </div>

    <!-- Moving pictures need a way to stop them (WCAG 2.2.2); with reduced motion it starts still, on the last frame. -->
    <Button variant="outline" size="icon-xs" class="wc-toggle" :style="toggleAt" :aria-label="mode === 'running' ? t('welcome.clip.pause') : t('welcome.clip.play')" data-testid="welcome-clip-toggle" @click="toggle">
      <component :is="mode === 'running' ? Pause : Play" aria-hidden="true" />
    </Button>
  </figure>
</template>

<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import { ArrowDownToLine, Check, ChevronDown, Link as LinkIcon, LoaderCircle, Pause, Play, X } from '@lucide/vue';
import { Button, buttonVariants } from '../ui/button';
import { siteLogo } from '../../helpers/siteLogo';
import { shortcutKeys } from '../../helpers/shortcut';
import { usePlatform } from '../../composables/usePlatform';
import { desktopOs } from './desktopOs';
import { TRY_LINK } from '@/generated/contract';

/** The sample the "Try it" link opens ("Me at the zoo"); the facts on the card are plausible sample values. */
const LINK = TRY_LINK.replace(/^https:\/\/www\./, '');
const SITE = 'YouTube';
const TITLE = 'Me at the zoo';
/** Length, quality, container, size: sample values for the card and the saved row. */
const FACTS = ['0:19', '360p', 'MP4', '1.2 MB'];
const logo = siteLogo(TRY_LINK) ?? '';

/** One loop in ms; the CSS keyframes below use the same beats as percentages of it. */
const LOOP = 7900;
/** Where each step starts: copy, card, Download. */
const STEP_AT = [0, 1900, 3650];
/** The rest ends: the Saved frame fades into the first beat, and step 1 shows from here. */
const RESET = 7600;
/** The row's progress fills linearly between these two moments. */
const FILL = [4370, 5870] as const;

const emit = defineEmits<{ step: [step: number] }>();
const { t } = useI18n();
const os = computed(() => desktopOs(usePlatform().platform.value));
const keys = computed(() => shortcutKeys('CommandOrControl+C', os.value === 'mac'));

/** static: the last frame, nothing moves (reduced motion, until Play). */
const mode = ref<'running' | 'paused' | 'static'>('static');
const box = ref<HTMLElement>();
const win = ref<HTMLElement>();
const pct = ref(100);

/** The scene keeps a minimum size and scales down to fit a small pane, so the card never overlaps the link. */
const MIN_W = 640;
const MIN_H = 470;
const size = ref({ w: 0, h: 0 });
const fit = computed(() => {
  const { w, h } = size.value;
  if (!w || !h) return undefined;
  const W = Math.max(w, MIN_W);
  const H = Math.max(h, MIN_H);
  const s = Math.min(w / W, h / H, 1);
  return { width: `${W}px`, height: `${H}px`, insetInlineStart: `${(w - W * s) / 2}px`, top: `${(h - H * s) / 2}px`, transform: s < 1 ? `scale(${s})` : undefined };
});

/** The toggle stays in the scene's bottom-end corner when the scene is scaled into a smaller box. */
const toggleAt = computed(() => {
  const { w, h } = size.value;
  if (!w || !h) return undefined;
  const W = Math.max(w, MIN_W);
  const H = Math.max(h, MIN_H);
  const s = Math.min(w / W, h / H, 1);
  return { insetBlockEnd: `${(h - H * s) / 2 + 8}px`, insetInlineEnd: `${(w - W * s) / 2 + 8}px` };
});
/** A narrow screen puts the app's link under the card's corner instead of beside it. */
const narrow = computed(() => size.value.w > 0 && Math.max(size.value.w, MIN_W) < 760);

let timer: ReturnType<typeof setTimeout> | undefined;
let frame = 0;
/** Where the CSS loop is now, read from the window's own animation: no second clock to drift. */
function clock(): number | undefined {
  const anim = win.value?.getAnimations?.()[0];
  const at = Number(anim?.currentTime ?? NaN);
  return Number.isFinite(at) ? at % LOOP : undefined;
}
function stopClock() {
  clearTimeout(timer);
  cancelAnimationFrame(frame);
}
/** Follows the loop: the step on each beat, the percent while the row fills. */
function tick() {
  stopClock();
  if (mode.value !== 'running') return;
  const at = clock();
  if (at === undefined) {
    timer = setTimeout(tick, 50);
    return;
  }
  emit('step', at >= RESET ? 1 : STEP_AT.reduce((step, start, i) => (at >= start ? i + 1 : step), 0));
  pct.value = Math.round(Math.min(1, Math.max(0, (at - FILL[0]) / (FILL[1] - FILL[0]))) * 100);
  if (at >= FILL[0] && at < FILL[1]) {
    frame = requestAnimationFrame(tick);
    return;
  }
  const next = [...STEP_AT, FILL[0], RESET, LOOP].filter(ms => ms > at).sort((a, b) => a - b)[0] ?? LOOP;
  timer = setTimeout(tick, next - at + 10);
}

function play() {
  mode.value = 'running';
  // The class change restarts or resumes the CSS animations on the next style pass.
  requestAnimationFrame(tick);
}
function stop() {
  stopClock();
  mode.value = 'static';
  pct.value = 100;
  emit('step', 0);
}
function toggle() {
  if (mode.value === 'running') {
    stopClock();
    mode.value = 'paused';
  } else {
    play();
  }
}

let motion: MediaQueryList | undefined;
function onMotion() {
  if (motion?.matches) stop();
  else play();
}
let watchSize: ResizeObserver | undefined;
onMounted(() => {
  motion = window.matchMedia('(prefers-reduced-motion: reduce)');
  motion.addEventListener('change', onMotion);
  if (!motion.matches) play();
  if (typeof ResizeObserver !== 'undefined' && box.value) {
    watchSize = new ResizeObserver(([entry]) => {
      size.value = { w: entry.contentRect.width, h: entry.contentRect.height };
    });
    watchSize.observe(box.value);
  }
});
onBeforeUnmount(() => {
  stopClock();
  motion?.removeEventListener('change', onMotion);
  watchSize?.disconnect();
});

defineExpose({ play });
</script>

<style>
/* Screen colours around the app's own tokens (the card and the row use the app tokens directly). */
.nz-wanim {
  --wc-desk: #E2E9E7; --wc-desk-bar: #D3DCD9; --wc-desk-ink: color-mix(in oklab, var(--foreground) 12%, transparent);
  --wc-select: color-mix(in oklab, var(--primary) 22%, transparent);
  --wc-track: color-mix(in oklab, var(--foreground) 15%, transparent);
  --wc-card-shadow: 0 0 0 1px rgb(14 22 20 / .04), 0 10px 28px rgb(14 22 20 / .14);
}
[data-theme="dark"] .nz-wanim {
  --wc-desk: #121A18; --wc-desk-bar: #19221F; --wc-desk-ink: color-mix(in oklab, var(--foreground) 10%, transparent);
  --wc-select: color-mix(in oklab, var(--primary) 28%, transparent);
  --wc-track: color-mix(in oklab, var(--foreground) 25%, transparent);
  --wc-card-shadow: 0 0 0 1px rgb(255 255 255 / .06), 0 10px 28px rgb(0 0 0 / .55);
}
@media (prefers-color-scheme: dark) {
  body:not([data-theme]) .nz-wanim {
    --wc-desk: #121A18; --wc-desk-bar: #19221F; --wc-desk-ink: color-mix(in oklab, var(--foreground) 10%, transparent);
    --wc-select: color-mix(in oklab, var(--primary) 28%, transparent);
    --wc-track: color-mix(in oklab, var(--foreground) 25%, transparent);
    --wc-card-shadow: 0 0 0 1px rgb(255 255 255 / .06), 0 10px 28px rgb(0 0 0 / .55);
  }
}
</style>

<style scoped>
.wc { position: relative; margin: 0; min-height: 0; min-width: 0; --dx: 1; }
.wc:dir(rtl) { --dx: -1; }

/* The base styles are the last frame (reduced motion shows it as is); the loop's keyframes draw every beat before it.
   Beats in ms of a 7.9 s loop: select 450, copy keys 1000, card 1900, pointer 2900, press 3650, card out 4150,
   row 4250 (fill 4370-5870), saved 5900, rest, reset 7600. Transform and opacity only. */
.wc-stage { position: absolute; inset: 0; overflow: hidden; border-radius: 12px; background: var(--wc-desk); transform-origin: 0 0; pointer-events: none; }
.wc-stage:dir(rtl) { transform-origin: 100% 0; }
.wc-bar { height: 26px; background: var(--wc-desk-bar); display: flex; align-items: center; gap: 14px; padding-inline: 14px; direction: ltr; }
.wc-bar i, .wc-app > i { display: block; height: 7px; border-radius: 4px; background: var(--wc-desk-ink); }
.wc-app { position: absolute; top: 120px; inset-inline-start: 44px; display: flex; flex-direction: column; gap: 10px; }
.wc-app > i { height: 8px; }
.is-narrow .wc-app { top: 216px; }

.wc-copy { display: flex; align-items: center; gap: 10px; }
.wc-pill { height: 28px; display: flex; align-items: center; gap: 6px; padding-inline: 10px; border-radius: 999px; background: var(--card); color: var(--foreground); font-size: 12px; white-space: nowrap; overflow: hidden; }
.wc-copy .wc-pill { width: 300px; }
.wc-pill-ic { width: 12px; height: 12px; flex: none; color: var(--muted-foreground); }
.wc-sel { position: relative; isolation: isolate; display: block; min-width: 0; overflow: hidden; text-overflow: ellipsis; }
.wc-sel::before { content: ""; position: absolute; inset: 0; z-index: -1; border-radius: 3px; background: var(--wc-select); transform-origin: 0 50%; }
.wc-sel:dir(rtl)::before { transform-origin: 100% 50%; }
.wc-keys { display: flex; gap: 3px; }
.wc-kbd { min-width: 22px; height: 22px; padding-inline: 5px; display: grid; place-items: center; border-radius: 5px; font-size: 12px; font-weight: 500; background: var(--card); color: var(--foreground); border: 1px solid var(--input); box-shadow: 0 1px 0 var(--input); }

.wc-card { position: absolute; top: 38px; inset-inline-end: 14px; z-index: 2; width: 360px; height: 160px; padding: 12px; display: flex; flex-direction: column;
  border: 1px solid var(--hairline); border-radius: 12px; background: var(--background); color: var(--foreground); box-shadow: var(--wc-card-shadow); font-size: 13px; }
.wc-card:dir(rtl) { font-size: 14px; }
.wc-best { display: grid; }
.wc-lbl { grid-area: 1 / 1; display: inline-flex; align-items: center; justify-content: center; gap: 6px; }
.wc-lbl-2 { opacity: 0; }
.wc-caret { box-shadow: inset 1px 0 0 color-mix(in oklab, var(--primary-foreground) 28%, transparent); }
.wc-caret:dir(rtl) { box-shadow: inset -1px 0 0 color-mix(in oklab, var(--primary-foreground) 28%, transparent); }
/* The system pointer: never mirrored. */
.wc-cursor { position: absolute; top: 14px; inset-inline-start: 58%; width: 18px; height: 24px; opacity: 0; }

.wc-win { position: absolute; bottom: 0; inset-inline: 44px; height: 150px; display: flex; flex-direction: column; overflow: hidden;
  background: var(--background); color: var(--foreground); border: 1px solid var(--hairline); border-bottom: 0; border-radius: 10px 10px 0 0; font-size: 13px; }
.wc-win:dir(rtl) { font-size: 14px; }
.wc-win-bar { position: relative; height: 44px; flex: none; display: flex; align-items: center; justify-content: center; border-bottom: 1px solid var(--hairline); }
.wc-paste { width: 300px; background: color-mix(in oklab, var(--foreground) 5.5%, transparent); color: var(--muted-foreground); }
/* macOS window buttons: top left in every language, like the real title bar. */
.wc-lights { position: absolute; top: 15px; inset-inline-start: 13px; display: flex; gap: 8px; }
.wc-lights:dir(rtl) { inset-inline-start: auto; inset-inline-end: 13px; }
.wc-lights i { width: 12px; height: 12px; border-radius: 50%; background: color-mix(in oklab, var(--foreground) 16%, transparent); }

.wc-row { display: flex; align-items: center; gap: 12px; padding-block: 10px; padding-inline: 32px 16px; border-bottom: 1px solid var(--hairline); text-align: start; }
.wc-thumb { width: 72px; height: 40px; flex: none; display: grid; place-items: center; border-radius: 5px; background: var(--muted); color: var(--muted-foreground);
  box-shadow: inset 0 0 0 1px color-mix(in oklab, var(--foreground) 6%, transparent); }
.wc-meta { margin-top: 4px; display: grid; font-size: 12px; color: var(--muted-foreground); }
.wc-meta > * { grid-area: 1 / 1; display: flex; align-items: center; gap: 10px; min-width: 0; white-space: nowrap; }
.wc-run { opacity: 0; }
.wc-done { gap: 6px; }
.wc-ok { display: inline-flex; align-items: center; gap: 4px; font-weight: 500; color: var(--primary-soft-foreground); }
.wc-ok svg { width: 13px; height: 13px; stroke-width: 2.5; }
.wc-track { flex: 1 1 120px; max-width: 180px; height: 4px; border-radius: 999px; background: var(--wc-track); overflow: hidden; }
.wc-fill { display: block; height: 100%; border-radius: inherit; background: var(--primary); transform-origin: 0 50%; }
.wc-fill:dir(rtl) { transform-origin: 100% 50%; }
.wc-act { flex: none; display: grid; }
.wc-act > * { grid-area: 1 / 1; }
.wc-round { width: 28px; height: 28px; display: grid; place-items: center; justify-self: end; border-radius: 999px; border: 1px solid var(--input); background: var(--card); opacity: 0; }
.wc-round svg { width: 13px; height: 13px; }

.wc-toggle { position: absolute; inset-block-end: 8px; inset-inline-end: 8px; z-index: 3; }

/* ---------- the loop ---------- */
.wc:not(.is-static) .wc-sel::before,
.wc:not(.is-static) .wc-stage :is(.wc-keys, .wc-card, .wc-cursor, .wc-best, .wc-lbl-1, .wc-lbl-2, .wc-win, .wc-row, .wc-fill, .wc-run, .wc-done, .wc-round, .wc-open) {
  animation-duration: 7900ms; animation-iteration-count: infinite; animation-fill-mode: both; animation-timing-function: linear;
}
.wc:not(.is-static) .wc-sel::before { animation-name: wc-sel, wc-reset; }
.wc:not(.is-static) .wc-keys { animation-name: wc-keys; }
.wc:not(.is-static) .wc-card { animation-name: wc-card; }
.wc:not(.is-static) .wc-cursor { animation-name: wc-cursor; }
.wc:not(.is-static) .wc-best { animation-name: wc-press; }
.wc:not(.is-static) .wc-lbl-1 { animation-name: wc-before-press; }
.wc:not(.is-static) .wc-lbl-2 { animation-name: wc-after-press; }
.wc:not(.is-static) .wc-win { animation-name: wc-win, wc-reset; }
.wc:not(.is-static) .wc-row { animation-name: wc-row; }
.wc:not(.is-static) .wc-fill { animation-name: wc-fill; }
.wc:not(.is-static) :is(.wc-run, .wc-round) { animation-name: wc-before-saved; }
.wc:not(.is-static) :is(.wc-done, .wc-open) { animation-name: wc-after-saved; }
.wc:not(.is-static) .wc-spin { animation: wc-spin 1s linear infinite; }
.wc.is-paused *, .wc.is-paused .wc-sel::before { animation-play-state: paused; }

/* 7600 → 7840: what the first beat lacks (selection, keys, window) fades out; the screen and the app stay,
   so the loop goes from Saved straight into beat 1 and never shows an empty stage. */
@keyframes wc-reset { 0%, 96.2% { opacity: 1; animation-timing-function: ease-out; } 99.2%, 100% { opacity: 0; } }
/* 450 → 830: a selection sweeps the link. */
@keyframes wc-sel {
  0%, 5.7% { transform: scaleX(0); animation-timing-function: cubic-bezier(0.23, 1, 0.32, 1); }
  10.5%, 100% { transform: scaleX(1); }
}
/* 1000: the copy keys. */
@keyframes wc-keys {
  0%, 12.66% { opacity: 0; transform: translateY(3px); animation-timing-function: cubic-bezier(0.23, 1, 0.32, 1); }
  15.2%, 96.2% { opacity: 1; transform: none; animation-timing-function: ease-out; }
  99.2%, 100% { opacity: 0; transform: none; }
}
/* 1900: the card lands in its corner; 4150: it hands over to the list. */
@keyframes wc-card {
  0%, 24.05% { opacity: 0; transform: translateY(-10px) scale(.98); animation-timing-function: cubic-bezier(0.23, 1, 0.32, 1); }
  27.3% { opacity: 1; }
  29.4% { transform: none; }
  52.5% { opacity: 1; transform: none; animation-timing-function: ease-out; }
  55.1%, 100% { opacity: 0; transform: translateY(-8px); }
}
/* 2900: the pointer moves onto Download. */
@keyframes wc-cursor {
  0%, 36.7% { opacity: 0; transform: translate(calc(var(--dx) * 70px), 46px); animation-timing-function: cubic-bezier(0.77, 0, 0.175, 1); }
  39.2% { opacity: 1; }
  44.9%, 100% { opacity: 1; transform: none; }
}
/* 3650: pressed. */
@keyframes wc-press {
  0%, 46.2% { transform: none; animation-timing-function: ease-out; }
  47.7%, 100% { transform: scale(.97); }
}
@keyframes wc-before-press { 0%, 46.19% { opacity: 1; } 46.2%, 100% { opacity: 0; } }
@keyframes wc-after-press { 0%, 46.19% { opacity: 0; } 46.2%, 100% { opacity: 1; } }
/* 4150: the Nezzel window rises; 4410 the row shows; 4370 → 5870 it fills. */
@keyframes wc-win {
  0%, 52.5% { transform: translateY(100%); animation-timing-function: cubic-bezier(0.23, 1, 0.32, 1); }
  57.3%, 100% { transform: none; }
}
@keyframes wc-row {
  0%, 55.8% { opacity: 0; transform: translateY(-4px); animation-timing-function: cubic-bezier(0.23, 1, 0.32, 1); }
  58.4%, 100% { opacity: 1; transform: none; }
}
@keyframes wc-fill { 0%, 55.3% { transform: scaleX(0); } 74.3%, 100% { transform: scaleX(1); } }
/* 5900: Saved. */
@keyframes wc-before-saved { 0%, 74.7% { opacity: 1; animation-timing-function: ease-out; } 77.2%, 100% { opacity: 0; } }
@keyframes wc-after-saved { 0%, 74.7% { opacity: 0; animation-timing-function: ease-out; } 77.2%, 100% { opacity: 1; } }
@keyframes wc-spin { to { transform: rotate(360deg); } }
</style>
