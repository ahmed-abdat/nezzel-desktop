<template>
  <config-provider :dir="dir">
    <section
      ref="sheet"
      class="nz-welcome outline-none fixed inset-0 z-[60] flex flex-col bg-background text-foreground"
      :class="dir === 'rtl' && 'is-rtl'"
      role="dialog"
      aria-modal="true"
      aria-labelledby="welcome-title"
      data-testid="welcome"
      tabindex="-1"
    >
      <!-- macOS overlay title bar: room for the traffic lights, and a place to drag the window. -->
      <div v-if="os === 'mac'" class="h-[38px] flex-none border-b border-hairline bg-sidebar" data-tauri-drag-region />

      <div class="grid min-h-0 flex-1 grid-cols-[minmax(300px,440px)_minmax(380px,1fr)]">
        <div class="wl-copy-col flex min-w-0 flex-col overflow-y-auto px-10 pt-9">
          <img :src="nezzelMark" alt="Nezzel" class="block size-11">
          <h1 id="welcome-title" class="wl-title">{{ t('welcome.title') }}</h1>
          <p class="wl-lead">{{ capturePlatform.backgroundClipboard ? t('welcome.lead') : t('welcome.leadFocus') }}</p>

          <div class="mt-[26px] border-t border-hairline">
            <div class="wl-row">
              <ClipboardCheck class="wl-ic" aria-hidden="true" />
              <span class="min-w-0 flex-1">
                <label id="wl-card-l" for="wl-card" class="wl-label">{{ t('welcome.card.label') }}</label>
                <small id="wl-card-d" class="wl-help">{{ capturePlatform.backgroundClipboard ? t('welcome.card.help') : t('welcome.card.helpFocus') }}</small>
              </span>
              <Switch id="wl-card" class="wl-sw" :model-value="card" aria-labelledby="wl-card-l" aria-describedby="wl-card-d" data-testid="welcome-card" @update:model-value="setCard" />
            </div>
            <div class="wl-row">
              <Power class="wl-ic" aria-hidden="true" />
              <span class="min-w-0 flex-1">
                <label id="wl-start-l" for="wl-start" class="wl-label">{{ t('welcome.start.label') }}</label>
                <small id="wl-start-d" class="wl-help">{{ t('welcome.start.help', { place: t(`welcome.place.${os}`) }) }}</small>
              </span>
              <Switch id="wl-start" v-model="start" class="wl-sw" aria-labelledby="wl-start-l" aria-describedby="wl-start-d" data-testid="welcome-start" />
            </div>
            <div class="wl-row">
              <LifeBuoy class="wl-ic" aria-hidden="true" />
              <span class="min-w-0 flex-1">
                <label id="wl-reports-l" for="wl-reports" class="wl-label">{{ t('report.setting.label') }}</label>
                <small id="wl-reports-d" class="wl-help">{{ t('report.setting.welcomeHelp') }}</small>
              </span>
              <Switch id="wl-reports" class="wl-sw" :model-value="reports" aria-labelledby="wl-reports-l" aria-describedby="wl-reports-d" data-testid="welcome-reports" @update:model-value="setReports" />
            </div>
            <welcome-shortcut :mac="os === 'mac'" />
          </div>

          <p class="wl-priv">
            <ShieldCheck class="wl-priv-ic" aria-hidden="true" />
            <span>{{ t('welcome.privacy') }}</span>
          </p>

          <div class="wl-foot sticky bottom-0 mt-auto flex items-center gap-3.5 bg-background pt-6 pb-8">
            <Button size="lg" class="wl-cta" data-testid="welcome-start-button" @click="getStarted">{{ t('welcome.getStarted') }}</Button>
            <small class="wl-caption">{{ t('welcome.later') }}</small>
          </div>
        </div>

        <div class="wl-demo flex min-w-0 flex-col border-s border-hairline bg-sidebar px-7 pt-7 pb-[22px]">
          <welcome-demo ref="demo" :os="os" :folder="folder" @step="step = $event" />

          <ol class="mt-3.5 flex gap-1.5" aria-hidden="true">
            <li v-for="s in STEPS" :key="s.k" class="wl-step" :class="{ on: step === s.k }">
              <component :is="s.icon" class="size-[15px]" />
              <span class="wl-step-t">{{ t(`welcome.steps.${s.key}`) }}</span>
            </li>
          </ol>

          <div class="wl-try">
            <span id="wl-try-l" class="flex-none">{{ card ? t('welcome.try.label') : t('welcome.try.off') }}</span>
            <span class="wl-link" tabindex="0" aria-labelledby="wl-try-l" @copy="onLinkCopy">
              <LinkIcon class="size-3 flex-none text-muted-foreground rtl:-scale-x-100" aria-hidden="true" />
              <bdi dir="ltr" class="truncate">{{ SAMPLE_LABEL }}</bdi>
            </span>
            <Button variant="outline" class="wl-copy" :class="copied && 'text-primary-soft-foreground'" :disabled="!card" data-testid="welcome-copy" @click="tryIt">
              <component :is="copied ? Check : Copy" aria-hidden="true" />
              <span aria-live="polite">{{ copied ? t('welcome.try.copied') : t('welcome.try.copy') }}</span>
            </Button>
          </div>
        </div>
      </div>
    </section>
  </config-provider>
</template>

<script setup lang="ts">
import { computed, nextTick, onMounted, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import { ConfigProvider } from 'reka-ui';
import { invoke } from '@tauri-apps/api/core';
import { Check, ClipboardCheck, Copy, Download, LifeBuoy, Link as LinkIcon, PictureInPicture2, Power, ShieldCheck } from '@lucide/vue';
import nezzelMark from '../../assets/brand/nezzel-mark.svg';
import { Button } from '../ui/button';
import { Switch } from '../ui/switch';
import WelcomeDemo from './WelcomeDemo.vue';
import WelcomeShortcut from './WelcomeShortcut.vue';
import { desktopOs, folderLabel } from './desktopOs';
import { localeDirection } from '../../i18n';
import { usePlatform } from '../../composables/usePlatform';
import { useSettingsStore } from '../../stores/settings';
import { useOnboardingStore } from '../../stores/onboarding';
import { useToastStore } from '../../stores/toast';
import { useCapturePlatform } from '../../composables/useCapturePlatform';

/** Same link as the backend's TRY_LINK ("Me at the zoo"); only the browser fallback writes it from here. */
const SAMPLE_URL = 'https://www.youtube.com/watch?v=jNQXAC9IVRw';
const SAMPLE_LABEL = 'youtube.com/watch?v=jNQXAC9IVRw';
const STEPS = [
  { k: 1, key: 'copy', icon: Copy },
  { k: 2, key: 'card', icon: PictureInPicture2 },
  { k: 3, key: 'save', icon: Download },
] as const;

const { t, locale } = useI18n();
const settings = useSettingsStore();
const onboarding = useOnboardingStore();
const { platform } = usePlatform();
// GNOME Wayland can't read the clipboard in the background: don't promise a card "with this window closed".
const capturePlatform = useCapturePlatform();

const dir = computed(() => localeDirection(locale.value));
const os = computed(() => desktopOs(platform.value));
const folder = computed(() => folderLabel(settings.settings.output?.downloadDir));
const card = computed(() => settings.settings.input.watchClipboard);
const start = ref(true);
const step = ref(0);
const copied = ref(false);
const demo = ref<{ showTry: () => void }>();
const sheet = ref<HTMLElement>();
let copiedTimer: ReturnType<typeof setTimeout> | undefined;

/** Crash and error reports: on by default, saved at once like the card switch. */
const reports = computed(() => settings.settings.privacy?.sendReports ?? true);
function setReports(on: boolean) {
  void settings.patch({ privacy: { sendReports: on } }).catch(() => {});
}

function setCard(on: boolean) {
  void onboarding.setCard(on);
}

/** Writes the sample link (never reads the clipboard); with the card on, the real card shows for it. */
async function tryIt() {
  if (!card.value) return;
  try {
    await invoke('capture_try_link');
  } catch {
    try {
      await navigator.clipboard.writeText(SAMPLE_URL);
    } catch {
      useToastStore().showToast(t('welcome.try.failed'), { style: 'error' });
      return;
    }
  }
  copied.value = true;
  clearTimeout(copiedTimer);
  copiedTimer = setTimeout(() => (copied.value = false), 1600);
  demo.value?.showTry();
}

function onLinkCopy() {
  // The user copied the link text themselves: same result as the button.
  setTimeout(() => void tryIt(), 0);
}

function getStarted() {
  void onboarding.complete({ card: card.value, start: start.value });
}

onMounted(() => {
  // Focus starts in the sheet (screen readers read its title); Tab reaches the switches first.
  void nextTick(() => sheet.value?.focus({ preventScroll: true }));
});
</script>

<style scoped>
.wl-title { font-size: 24px; line-height: 1.22; font-weight: 600; letter-spacing: -.015em; margin: 20px 0 10px; text-wrap: balance; }
.wl-lead { font-size: 14px; line-height: 1.6; color: var(--muted-foreground); max-width: 46ch; text-wrap: pretty; }
.wl-row { display: flex; align-items: flex-start; gap: 12px; padding: 12px 0; border-bottom: 1px solid var(--hairline); }
.wl-ic { width: 16px; height: 16px; flex: none; margin-top: 1px; color: var(--muted-foreground); }
.wl-label { display: block; font-weight: 500; font-size: 13px; cursor: pointer; }
.wl-help { display: block; font-size: 12px; color: var(--muted-foreground); margin-top: 2px; text-wrap: pretty; }
.wl-sw { margin-top: 2px; }
.wl-priv { display: flex; gap: 8px; align-items: flex-start; margin-top: 14px; font-size: 12px; color: var(--muted-foreground); text-wrap: pretty; }
.wl-priv-ic { width: 14px; height: 14px; flex: none; margin-top: 1px; color: var(--primary-soft-foreground); }
.wl-cta { flex: none; white-space: nowrap; }
.wl-caption { font-size: 12px; color: var(--muted-foreground); }
.wl-step { list-style: none; flex: 1; display: flex; align-items: center; gap: 8px; padding: 8px 10px; border-radius: 7px; color: var(--muted-foreground); font-size: 12.5px; font-weight: 500; min-width: 0;
  transition: background-color 180ms var(--nz-ease), color 180ms var(--nz-ease), box-shadow 180ms var(--nz-ease); }
.wl-step svg { flex: none; }
.wl-step.on { background: var(--card); color: var(--primary-soft-foreground); box-shadow: 0 1px 2px rgb(0 30 25 / .08); }
.wl-try { display: flex; align-items: center; gap: 10px; margin-top: 14px; padding-top: 14px; border-top: 1px solid var(--hairline); font-size: 12.5px; color: var(--muted-foreground); }
.wl-link { flex: 1; min-width: 0; height: 28px; display: flex; align-items: center; gap: 6px; padding-inline: 10px; border-radius: 999px; background: color-mix(in oklab, var(--foreground) 5.5%, transparent); color: var(--foreground); font-size: 12px; overflow: hidden; white-space: nowrap; user-select: all; cursor: text; }
.wl-copy { flex: none; }
.wl-step-t { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.wl-link { min-width: 150px; }
/* Narrow windows (750px): the demo column is ~380px. */
.wl-demo { container-type: inline-size; }
@container (max-width: 460px) {
  .wl-step svg { display: none; }
  .wl-step { justify-content: center; padding-inline: 6px; }
  .wl-step-t { white-space: normal; text-align: center; line-height: 1.25; }
  .wl-try { flex-wrap: wrap; row-gap: 8px; }
  .wl-try > span:first-child { flex-basis: 100%; }
}

/* Arabic: one step larger, taller lines, no tracking. */
.is-rtl .wl-title { font-size: 26px; line-height: 1.5; letter-spacing: 0; }
.is-rtl .wl-lead { font-size: 15px; line-height: 1.8; }
.is-rtl .wl-ic { margin-top: 4px; }
.is-rtl .wl-label { font-size: 14px; }
.is-rtl .wl-help { font-size: 13px; margin-top: 0; }
.is-rtl .wl-sw { margin-top: 5px; }
.is-rtl .wl-priv { font-size: 13px; }
.is-rtl .wl-priv-ic { margin-top: 4px; }
.is-rtl .wl-caption { font-size: 13px; }
.is-rtl .wl-step { font-size: 13.5px; }
.is-rtl .wl-try { font-size: 13.5px; }
.is-rtl :deep(.wl-cta) { font-size: 15px; }

/* Short windows (650px): keep Get started in view. */
@media (max-height: 720px) {
  .wl-copy-col { padding-top: 24px; }
  .wl-title { font-size: 21px; margin: 14px 0 8px; }
  .is-rtl .wl-title { font-size: 23px; }
  .wl-lead { font-size: 13px; }
  .is-rtl .wl-lead { font-size: 14px; line-height: 1.7; }
  .wl-copy-col > div:nth-of-type(1) { margin-top: 16px; }
  .wl-row, :deep(.wl-row) { padding-block: 9px; }
  .wl-foot { padding-top: 14px; padding-bottom: 20px; }
}

@media (prefers-reduced-motion: reduce) {
  .wl-step { transition: background-color 150ms ease-out, color 150ms ease-out; }
}
</style>
