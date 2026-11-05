<template>
  <!-- eslint-disable @intlify/vue-i18n/no-raw-text -- fake OS chrome (app name, clock, keycaps), not UI copy -->
  <div
    ref="screen"
    class="wd-screen"
    :class="[`os-${os}`, bottomBar ? 'bar-bottom' : 'bar-top', { 'is-rtl': rtl, sel, 'keys-in': keysIn, 'card-in': cardIn, press, resting }]"
    :data-p="phase"
    data-testid="welcome-demo"
    role="img"
    :aria-label="t('welcome.demoAlt', { folder })"
    @click="resting && play()"
  >
    <!-- OS bars: the user's own desktop. Mirrors in RTL like the real OS would. -->
    <div v-if="os === 'mac'" class="wd-bar wd-mbar" aria-hidden="true">
      <svg class="wd-apple" viewBox="0 0 814 1000"><path :d="APPLE" /></svg>
      <b>Safari</b><span class="wd-menu">{{ t('welcome.demo.file') }}</span><span class="wd-menu">{{ t('welcome.demo.edit') }}</span><span class="wd-menu">{{ t('welcome.demo.view') }}</span>
      <span class="wd-sp" />
      <span class="wd-tray" :class="{ on: cardIn, ping }"><svg viewBox="27 31 59 49"><path :d="GLYPH" /></svg></span>
      <Wifi class="wd-ic" />
      <Battery class="wd-ic" />
      <span class="wd-nowrap">{{ t('welcome.demo.clock') }}</span>
    </div>
    <div v-else-if="os === 'linux' && desktop === 'gnome'" class="wd-bar wd-gbar" aria-hidden="true">
      <span>{{ t('welcome.demo.activities') }}</span>
      <span class="wd-clk">{{ t('welcome.demo.clockGnome') }}</span>
      <span class="wd-sp" />
      <span class="wd-tray" :class="{ on: cardIn, ping }"><svg viewBox="27 31 59 49"><path :d="GLYPH" /></svg></span>
      <Wifi class="wd-ic" />
      <Power class="wd-ic" />
    </div>
    <!-- KDE Plasma and most other Linux desktops: a bottom panel, launcher at the start, tray at the end. -->
    <div v-else-if="os === 'linux'" class="wd-bar wd-taskbar wd-lpanel" aria-hidden="true">
      <span class="wd-launch"><LayoutGrid class="wd-ic" /></span>
      <span class="wd-apps"><i class="br" /><i class="fo" /></span>
      <span class="wd-traywin">
        <span class="wd-tray" :class="{ on: cardIn, ping }"><svg viewBox="27 31 59 49"><path :d="GLYPH" /></svg></span>
        <Wifi class="wd-ic" />
        <Volume1 class="wd-ic" />
        <span class="wd-clock"><span>9:41</span></span>
      </span>
    </div>
    <div v-else class="wd-bar wd-taskbar" aria-hidden="true">
      <span class="wd-apps"><i class="win11"><b><span /><span /><span /><span /></b></i><i class="br" /><i class="fo" /></span>
      <span class="wd-traywin">
        <span class="wd-tray" :class="{ on: cardIn, ping }"><ChevronUp class="wd-ic" /></span>
        <Wifi class="wd-ic" />
        <Volume1 class="wd-ic" />
        <span class="wd-clock"><span>9:41 AM</span><span>11/4/2026</span></span>
      </span>
    </div>

    <div class="wd-browser" aria-hidden="true">
      <div class="wd-btool">
        <span class="wd-dots"><i /><i /><i /></span>
        <span class="wd-addr"><Lock class="wd-ic" /><span ref="urlEl" class="wd-url">{{ SAMPLE_LABEL }}</span></span>
      </div>
      <div class="wd-page">
        <div ref="playerEl" class="wd-player"><svg class="wd-thumb" viewBox="0 0 160 100" preserveAspectRatio="xMidYMid slice"><rect width="160" height="100" fill="#3b2a1e" /><rect y="62" width="160" height="38" fill="#7a5638" /><rect y="62" width="160" height="2" fill="#5e4129" /><ellipse cx="80" cy="70" rx="58" ry="6" fill="#2a1c12" opacity=".5" /><path d="M28 66c0-22 22-36 52-36s52 14 52 36z" fill="#D49A55" /><path d="M28 66c0-22 22-36 52-36s52 14 52 36z" fill="#8A4E1E" opacity=".3" /><path d="M52 44l10 14M72 38l10 16M94 40l10 14" stroke="#F3D7A6" stroke-width="3.5" stroke-linecap="round" /><circle cx="132" cy="22" r="26" fill="#f6e4c4" opacity=".14" /></svg><span class="wd-pl"><Play class="size-3" /></span></div>
        <div class="wd-vtitle" dir="auto">{{ t('welcome.demo.videoTitle') }}</div>
        <div class="wd-vch" dir="auto">{{ t('welcome.demo.channel') }}</div>
        <div class="wd-side"><i /><i /><i /></div>
      </div>
    </div>

    <div class="wd-keys" aria-hidden="true">
      <kbd>{{ os === 'mac' ? '⌘' : 'Ctrl' }}</kbd><kbd>C</kbd><span>{{ t('welcome.demo.copied') }}</span>
    </div>

    <!-- The real copied-link card, in miniature. -->
    <div ref="cardEl" class="wd-card" aria-hidden="true">
      <div class="wd-hd"><img :src="nezzelMark" alt="" class="size-3.5">Nezzel<X class="wd-x" /></div>
      <div class="wd-it">
        <span class="wd-tn"><svg class="wd-thumb" viewBox="0 0 160 100" preserveAspectRatio="xMidYMid slice"><rect width="160" height="100" fill="#3b2a1e" /><rect y="62" width="160" height="38" fill="#7a5638" /><rect y="62" width="160" height="2" fill="#5e4129" /><ellipse cx="80" cy="70" rx="58" ry="6" fill="#2a1c12" opacity=".5" /><path d="M28 66c0-22 22-36 52-36s52 14 52 36z" fill="#D49A55" /><path d="M28 66c0-22 22-36 52-36s52 14 52 36z" fill="#8A4E1E" opacity=".3" /><path d="M52 44l10 14M72 38l10 16M94 40l10 14" stroke="#F3D7A6" stroke-width="3.5" stroke-linecap="round" /><circle cx="132" cy="22" r="26" fill="#f6e4c4" opacity=".14" /></svg></span>
        <span><b dir="auto">{{ t('welcome.demo.videoTitle') }}</b><small>{{ t('welcome.demo.meta') }}</small></span>
      </div>
      <div class="wd-ac">
        <span class="wd-q"><span>{{ t('welcome.demo.quality') }}</span><ChevronDown class="wd-ic" /></span>
        <span ref="saveEl" class="wd-save"><Download class="wd-ic" /><span>{{ t('welcome.demo.save') }}</span></span>
      </div>
      <div class="wd-prog"><i /></div>
      <div class="wd-done">
        <Check class="wd-ic" />
        <i18n-t keypath="welcome.demo.saved" tag="span" scope="global">
          <template #folder><bdi dir="ltr">{{ folder }}</bdi></template>
        </i18n-t>
      </div>
    </div>

    <div class="wd-cursor" :style="{ transform: `translate(${cursor[0]}px, ${cursor[1]}px)`, transition: cursorInstant ? 'none' : undefined }" aria-hidden="true">
      <svg viewBox="0 0 14 20"><path d="M1 1v15.5l4-3.6 2.6 5.8 2.6-1.2-2.6-5.6H13z" fill="#111" stroke="#fff" stroke-width="1.2" stroke-linejoin="round" /></svg>
    </div>
    <button type="button" class="wd-replay" data-testid="welcome-replay" @click.stop="play()">
      <RotateCcw class="wd-ic" aria-hidden="true" />{{ t('welcome.replay') }}
    </button>
  </div>
</template>

<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { localeDirection } from '../../i18n';
import { useI18n } from 'vue-i18n';
import { Battery, Check, ChevronDown, ChevronUp, Download, LayoutGrid, Lock, Play, Power, RotateCcw, Volume1, Wifi, X } from '@lucide/vue';
import nezzelMark from '../../assets/brand/nezzel-mark.svg';
import type { DesktopOs, LinuxDesktop } from './desktopOs';

const props = withDefaults(defineProps<{ os: DesktopOs; desktop?: LinuxDesktop; folder: string }>(), { desktop: 'gnome' });
/** Windows taskbar and non-GNOME Linux panels sit at the bottom; the tray card opens above them. */
const bottomBar = computed(() => props.os === 'windows' || (props.os === 'linux' && props.desktop !== 'gnome'));
const emit = defineEmits<{ step: [step: number] }>();
const { t, locale } = useI18n();
const rtl = computed(() => localeDirection(locale.value) === 'rtl');

const SAMPLE_LABEL = 'youtube.com/watch?v=jNQXAC9IVRw';
const GLYPH = 'M29.53591,32.96685 L45.94475,32.96685 L63.54696,54.74586 L63.54696,32.96685 L78.46409,32.96685 L78.46409,62.20442 L83.53591,62.20442 L68.91713,77.71823 L43.85635,49.37569 L43.85635,75.03315 L29.53591,75.03315 Z';
const APPLE = 'M788 341c-6 4-108 62-108 190 0 148 130 200 134 201-1 3-21 72-69 142-43 62-88 124-157 124s-87-40-166-40c-77 0-104 41-167 41s-107-58-157-128C40 789 0 663 0 543 0 352 125 250 248 250c65 0 120 43 161 43 39 0 100-45 175-45 28 0 130 3 204 93zM551 159c31-37 53-88 53-139 0-7-1-14-2-20-50 2-110 34-146 76-28 32-55 83-55 135 0 8 1 16 2 18 3 1 9 2 14 2 45 0 101-30 134-72z';

const screen = ref<HTMLElement>();
const urlEl = ref<HTMLElement>();
const playerEl = ref<HTMLElement>();
const cardEl = ref<HTMLElement>();
const saveEl = ref<HTMLElement>();

const sel = ref(false);
const keysIn = ref(false);
const cardIn = ref(false);
const press = ref(false);
const resting = ref(false);
const ping = ref(false);
const phase = ref<'' | 'dl' | 'saved'>('');
const cursor = ref<[number, number]>([0, 0]);
const cursorInstant = ref(true);

let timers: ReturnType<typeof setTimeout>[] = [];
const later = (ms: number, f: () => void) => timers.push(setTimeout(f, ms));
const clearTimers = () => {
  timers.forEach(clearTimeout);
  timers = [];
};

function at(el: HTMLElement | undefined, dx = 0.5, dy = 0.5): [number, number] {
  if (!el || !screen.value) return cursor.value;
  const a = screen.value.getBoundingClientRect();
  const r = el.getBoundingClientRect();
  return [r.left - a.left + r.width * dx, r.top - a.top + r.height * dy];
}
function move(p: [number, number], instant = false) {
  cursorInstant.value = instant;
  cursor.value = p;
  if (instant) requestAnimationFrame(() => (cursorInstant.value = false));
}
function setStep(k: number) {
  emit('step', k);
}
function showCard(on: boolean) {
  cardIn.value = on;
  if (on) {
    ping.value = false;
    requestAnimationFrame(() => (ping.value = true));
  }
}
function reset() {
  clearTimers();
  sel.value = keysIn.value = press.value = resting.value = ping.value = false;
  showCard(false);
  phase.value = '';
  setStep(0);
}
/** Final frame: card shown, saved. */
function rest() {
  clearTimers();
  sel.value = true;
  keysIn.value = false;
  showCard(true);
  ping.value = false;
  phase.value = 'saved';
  setStep(3);
  move(at(cardEl.value, 0.78, bottomBar.value ? -0.14 : 1.08), true);
  resting.value = true;
}
/** ~5s cycle, 2 cycles, then it rests on "Saved". */
function cycle(n: number) {
  showCard(false);
  press.value = false;
  resting.value = false;
  setStep(0);
  later(160, () => {
    phase.value = '';
    sel.value = false;
  });
  later(150, () => move(at(urlEl.value, 0.85, 0.6)));
  later(950, () => {
    sel.value = true;
    setStep(1);
  });
  later(1250, () => (keysIn.value = true));
  later(1750, () => {
    showCard(true);
    setStep(2);
  });
  later(2200, () => move(at(saveEl.value, 0.5, 0.6)));
  later(2300, () => (keysIn.value = false));
  later(3000, () => {
    press.value = true;
    setStep(3);
  });
  later(3150, () => {
    press.value = false;
    phase.value = 'dl';
  });
  later(4100, () => (phase.value = 'saved'));
  if (n < 1) later(5000, () => cycle(n + 1));
  else later(4200, () => (resting.value = true));
}

const reduceMotion = typeof matchMedia === 'function' && matchMedia('(prefers-reduced-motion: reduce)').matches;

function play() {
  reset();
  move(at(playerEl.value, 0.5, 0.62), true);
  if (reduceMotion) {
    // No travel: the steps crossfade in place.
    later(600, () => cycle(1));
    return;
  }
  later(600, () => cycle(0));
}

/** "Try it": the card waits on Save, like the real one. */
function showTry() {
  reset();
  move(at(saveEl.value, 0.5, 1.8), true);
  requestAnimationFrame(() => {
    showCard(true);
    setStep(2);
    resting.value = true;
  });
}

defineExpose({ play, showTry, rest });

onMounted(() => {
  void (document.fonts?.ready ?? Promise.resolve()).then(() => nextTick(play));
});
// Positions depend on text widths and the bar layout.
watch(() => [props.os, locale.value], () => nextTick(play));
onBeforeUnmount(clearTimers);
</script>

<style scoped>
.wd-screen {
  --e-out: cubic-bezier(.23, 1, .32, 1);
  --ease-io: cubic-bezier(.65, 0, .35, 1);
  position: relative;
  flex: 1;
  min-height: 300px;
  border-radius: 9px;
  overflow: hidden;
  isolation: isolate;
  background: linear-gradient(160deg, #BFD9D2 0%, #D9E4DC 45%, #E9DCCB 100%);
  box-shadow: inset 0 0 0 1px rgb(14 22 20 / .10);
  font-size: 11px;
  line-height: 1.35;
  color: #1d2321;
}
.os-windows.wd-screen { background: linear-gradient(200deg, #C9DCEA 0%, #D7E3E6 50%, #C7D8D2 100%); }
.os-linux.wd-screen { background: linear-gradient(150deg, #D6CFE0 0%, #DCE2DE 55%, #C3D8D0 100%); }
.resting { cursor: pointer; }
.wd-ic { width: 13px; height: 13px; flex: none; }

.wd-replay {
  position: absolute; z-index: 12; bottom: 10px; inset-inline-end: 10px; height: 28px; padding: 0 10px;
  border-radius: 999px; display: flex; align-items: center; gap: 6px; background: rgb(14 22 20 / .78); color: #fff;
  font-size: 12px; font-weight: 500; opacity: 0; transition: opacity 150ms ease-out;
}
.is-rtl .wd-replay { font-size: 13px; }
.bar-bottom .wd-replay { bottom: auto; top: 10px; }
.wd-screen:hover .wd-replay, .wd-replay:focus-visible { opacity: 1; }

/* OS bars */
.wd-bar { position: absolute; inset-inline: 0; display: flex; align-items: center; gap: 10px; z-index: 3; }
.wd-mbar { top: 0; height: 22px; padding-inline: 10px; background: rgb(246 248 247 / .78); backdrop-filter: blur(12px); }
.wd-mbar b { font-weight: 600; }
.wd-nowrap { white-space: nowrap; }
.wd-screen { container-type: inline-size; }
.wd-sp { margin-inline-start: auto; }
.wd-apple { width: 11px; height: 13px; fill: #1d2321; }
.wd-tray { display: grid; place-items: center; width: 20px; height: 18px; border-radius: 4px; transition: background-color 180ms var(--nz-ease), box-shadow 300ms ease-out; }
.wd-tray svg { width: 13px; height: 11px; fill: currentColor; }
.wd-tray.on { background: rgb(0 0 0 / .14); }
.wd-tray.ping { animation: wd-ping 500ms var(--e-out); }
@keyframes wd-ping { 0% { transform: scale(1); } 40% { transform: scale(1.16); } 100% { transform: scale(1); } }
.wd-taskbar { bottom: 0; height: 36px; padding-inline: 10px; background: rgb(238 242 244 / .88); backdrop-filter: blur(14px); border-top: 1px solid rgb(0 0 0 / .06); justify-content: center; gap: 6px; }
.wd-apps { display: flex; gap: 6px; }
.wd-apps i { width: 22px; height: 22px; border-radius: 5px; display: grid; place-items: center; }
.wd-apps i.win11 b { display: grid; grid-template-columns: 7px 7px; gap: 1.5px; }
.wd-apps i.win11 b span { width: 7px; height: 7px; background: #1F7AE0; border-radius: 1px; }
.wd-apps i.br { background: #fff; box-shadow: inset 0 0 0 1px rgb(0 0 0 / .08); }
.wd-apps i.br::after { content: ""; width: 12px; height: 12px; border-radius: 50%; background: conic-gradient(#2F7DE1 0 33%, #3AA15A 0 66%, #E8B020 0); }
.wd-apps i.fo { background: #F2C14E; }
.wd-traywin { position: absolute; inset-inline-end: 8px; top: 0; bottom: 0; display: flex; align-items: center; gap: 4px; }
.wd-traywin .wd-tray { width: 22px; height: 24px; }
.wd-traywin .wd-tray.on { background: rgb(0 0 0 / .08); }
.wd-clock { display: flex; flex-direction: column; align-items: flex-end; font-size: 11px; line-height: 1.2; padding-inline-start: 4px; direction: ltr; }
.is-rtl .wd-clock { align-items: flex-start; }
.wd-lpanel { justify-content: flex-start; background: rgb(236 239 241 / .9); }
.wd-launch { width: 22px; height: 22px; border-radius: 5px; display: grid; place-items: center; color: #1F7AE0; }
.wd-gbar { top: 0; height: 20px; padding-inline: 10px; background: #16191a; color: #eef1f0; font-size: 10.5px; font-weight: 600; }
.wd-clk { position: absolute; inset-inline: 0; width: max-content; margin-inline: auto; }
.wd-gbar .wd-ic { width: 12px; height: 12px; }
.wd-gbar .wd-tray.on { background: rgb(255 255 255 / .22); }

/* browser */
.wd-browser { position: absolute; inset-inline: 22px; top: 36px; bottom: 16px; background: #fff; border-radius: 8px; overflow: hidden; box-shadow: 0 10px 24px -8px rgb(0 30 25 / .28), 0 0 0 1px rgb(0 0 0 / .08); display: flex; flex-direction: column; z-index: 1; }
.bar-bottom .wd-browser { top: 14px; bottom: 48px; border-radius: 6px; }
.os-linux.bar-top .wd-browser { top: 32px; }
.wd-btool { height: 32px; flex: none; display: flex; align-items: center; gap: 8px; padding-inline: 10px; background: #F4F6F5; border-bottom: 1px solid #E5EAE8; }
.wd-dots { display: flex; gap: 5px; direction: ltr; }
.wd-dots i { width: 8px; height: 8px; border-radius: 50%; background: #D4DAD8; }
/* macOS traffic lights stay at the physical left, also in Arabic. */
.is-rtl.os-mac .wd-dots { order: 9; }
.os-windows .wd-dots, .os-linux .wd-dots { display: none; }
.wd-addr { flex: 1; height: 20px; border-radius: 6px; background: #fff; box-shadow: inset 0 0 0 1px #E1E7E5; display: flex; align-items: center; gap: 6px; padding-inline: 8px; color: #47524f; min-width: 0; }
.wd-addr .wd-ic { width: 10px; height: 10px; color: #8A9693; }
.wd-url { padding: 1px 2px; border-radius: 2px; transition: background-color 120ms ease-out; direction: ltr; unicode-bidi: isolate; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.sel .wd-url { background: #B3D7FF; }
.wd-page { flex: 1; padding: 12px 14px; display: grid; grid-template-columns: 1fr 92px; grid-template-rows: 1fr auto auto; column-gap: 12px; row-gap: 2px; min-height: 0; }
.wd-player { position: relative; border-radius: 6px; overflow: hidden; grid-row: 1; grid-column: 1; min-height: 0; }
.wd-thumb { position: absolute; inset: 0; width: 100%; height: 100%; display: block; filter: saturate(.8); }
.wd-pl { position: absolute; inset: 0; margin: auto; width: 34px; height: 24px; border-radius: 7px; background: rgb(0 0 0 / .55); display: grid; place-items: center; color: #fff; }
.wd-pl svg { fill: currentColor; }
.wd-vtitle { grid-column: 1; font-size: 12px; font-weight: 600; margin-top: 8px; }
.wd-vch { grid-column: 1; font-size: 10.5px; color: #6b7774; }
.wd-side { grid-row: 1 / 4; grid-column: 2; display: flex; flex-direction: column; gap: 8px; }
.wd-side i { display: block; border-radius: 4px; height: 48px; background: linear-gradient(135deg, #d9e2df, #eef1f0); }
.wd-side i:nth-child(2) { background: linear-gradient(135deg, #E7D8C4, #F2EBE0); }
.wd-side i:nth-child(3) { background: linear-gradient(135deg, #CFE0E8, #E9F0F2); }

/* keycaps */
.wd-keys { position: absolute; z-index: 6; inset-inline-start: 96px; top: 66px; opacity: 0; transform: translateY(-4px); display: flex; align-items: center; gap: 5px; padding: 7px 10px 7px 8px; border-radius: 9px; background: rgb(20 26 24 / .88); color: #fff; font-size: 11px; font-weight: 500; transition: opacity 150ms ease-out, transform 200ms var(--e-out); direction: ltr; }
.wd-keys kbd { min-width: 22px; height: 22px; padding: 0 6px; display: grid; place-items: center; border-radius: 5px; background: #fff; color: #111; font: 600 11px/1 Inter, system-ui, sans-serif; box-shadow: 0 2px 0 #9aa6a3; }
.wd-keys span { padding-inline-start: 4px; }
.bar-bottom .wd-keys { top: 44px; }
.os-linux.bar-top .wd-keys { top: 62px; }
.keys-in .wd-keys { opacity: 1; transform: none; }

/* the copied-link card */
.wd-card { position: absolute; z-index: 5; width: 240px; inset-inline-end: 8px; top: 28px; background: var(--card); color: var(--foreground); border-radius: 10px; padding: 10px;
  box-shadow: 0 16px 32px -10px rgb(0 30 25 / .35), 0 0 0 1px rgb(14 22 20 / .08);
  opacity: 0; transform: translateY(-8px) scale(.97); transform-origin: top right; pointer-events: none;
  transition: opacity 150ms ease-out, transform 0s linear 150ms; }
.is-rtl .wd-card { transform-origin: top left; }
.bar-bottom .wd-card { top: auto; bottom: 42px; transform: translateY(8px) scale(.97); transform-origin: bottom right; }
.is-rtl.bar-bottom .wd-card { transform-origin: bottom left; }
.os-linux.bar-top .wd-card { top: 26px; }
.card-in .wd-card { opacity: 1; transform: none; transition: opacity 240ms var(--e-out), transform 240ms var(--e-out); }
.wd-hd { display: flex; align-items: center; gap: 6px; font-size: 10.5px; color: var(--muted-foreground); margin-bottom: 8px; }
.wd-x { width: 11px; height: 11px; margin-inline-start: auto; }
.wd-it { display: flex; gap: 9px; }
.wd-tn { position: relative; width: 64px; height: 40px; border-radius: 5px; overflow: hidden; flex: none; }
.wd-it b { display: block; font-size: 11.5px; font-weight: 600; line-height: 1.3; }
.is-rtl .wd-it b { font-size: 12px; line-height: 1.45; }
.wd-it small { display: block; font-size: 10px; color: var(--muted-foreground); margin-top: 2px; }
.wd-ac { display: flex; gap: 6px; margin-top: 10px; align-items: center; }
.wd-q { flex: 1; height: 26px; border-radius: 6px; box-shadow: inset 0 0 0 1px var(--input); display: flex; align-items: center; gap: 4px; padding-inline: 8px; font-size: 10.5px; transition: opacity 150ms ease-out; }
.wd-q .wd-ic { width: 11px; height: 11px; margin-inline-start: auto; color: var(--muted-foreground); }
.wd-save { height: 26px; padding: 0 12px; border-radius: 6px; background: var(--primary); color: var(--primary-foreground); font-size: 11px; font-weight: 600; display: flex; align-items: center; gap: 5px; transition: background-color 120ms var(--nz-ease), transform 120ms ease-out; }
.wd-save .wd-ic { width: 11px; height: 11px; }
.press .wd-save { transform: scale(.95); background: var(--primary-hover); }
.wd-prog { height: 4px; border-radius: 999px; background: rgb(14 22 20 / .15); margin-top: 9px; overflow: hidden; display: none; }
.wd-prog i { display: block; height: 100%; width: 0; background: var(--primary); border-radius: 999px; }
[data-p=dl] .wd-prog { display: block; }
[data-p=dl] .wd-prog i { width: 100%; transition: width 900ms cubic-bezier(.3, .1, .3, 1); }
[data-p=dl] .wd-q { opacity: .5; }
.wd-done { display: none; align-items: center; gap: 6px; min-height: 26px; margin-top: 10px; font-size: 11px; font-weight: 600; color: var(--primary-soft-foreground); }
.wd-done .wd-ic { padding: 2px; border-radius: 50%; background: var(--primary-soft); stroke-width: 3; }
[data-p=saved] .wd-ac, [data-p=saved] .wd-prog { display: none; }
[data-p=saved] .wd-done { display: flex; }

/* pointer: placed by measured screen coordinates, so physical left/top on purpose */
.wd-cursor { position: absolute; z-index: 9; left: 0; top: 0; width: 14px; height: 20px; pointer-events: none; transition: transform 800ms var(--ease-io); will-change: transform; }
.wd-cursor svg { display: block; width: 14px; height: 20px; filter: drop-shadow(0 1px 1.5px rgb(0 0 0 / .3)); }
.wd-cursor::after { content: ""; position: absolute; left: -7px; top: -7px; width: 14px; height: 14px; border-radius: 50%; background: rgb(0 107 94 / .35); transform: scale(.2); opacity: 0; }
.press .wd-cursor::after { animation: wd-tap 400ms var(--e-out); }
@keyframes wd-tap { 0% { transform: scale(.2); opacity: 1; } 100% { transform: scale(1.8); opacity: 0; } }

/* Narrow demo (750px window): fewer menus, no related-videos column. */
@container (max-width: 400px) {
  .wd-menu, .wd-side { display: none; }
  .wd-page { grid-template-columns: 1fr; }
  .wd-browser { inset-inline: 12px; }
}

/* Reduced motion: no movement, only short opacity crossfades driven by the steps. */
@media (prefers-reduced-motion: reduce) {
  .wd-cursor { display: none; }
  .wd-keys, .wd-card, .bar-bottom .wd-card, .card-in .wd-card { transform: none !important; }
  .card-in .wd-card, .wd-card { transition: opacity 150ms ease-out !important; }
  .wd-keys { transition: opacity 150ms ease-out; }
  [data-p=dl] .wd-prog i { transition: none; }
  .wd-tray.ping { animation: none; }
  .press .wd-save { transform: none; }
}
</style>

<style>
/* Dark theme: the illustration dims with the app (unscoped: the theme lives on <body>). */
body[data-theme=dark] .wd-screen { filter: brightness(.82) saturate(.9); }
@media (prefers-color-scheme: dark) {
  body:not([data-theme=light]) .wd-screen { filter: brightness(.82) saturate(.9); }
}
</style>
