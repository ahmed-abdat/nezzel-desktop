<template>
  <config-provider :dir="dir">
    <section
      ref="sheet"
      class="nz-welcome outline-none fixed inset-0 z-[60] flex flex-col bg-background text-foreground"
      :class="dir === 'rtl' && 'is-rtl'"
      :data-keyboard="keyboard || undefined"
      @keydown.capture="onKey"
      @pointerdown.capture="keyboard = false"
      role="dialog"
      aria-modal="true"
      aria-labelledby="welcome-title"
      data-testid="welcome"
      tabindex="-1"
    >
      <!-- macOS overlay title bar: room for the traffic lights, and a place to drag the window. -->
      <div v-if="os === 'mac'" class="h-[38px] flex-none border-b border-hairline bg-sidebar" data-tauri-drag-region />

      <div class="wl-main grid min-h-0 flex-1 grid-cols-[minmax(300px,400px)_minmax(380px,1fr)]">
        <div class="wl-copy-col flex min-w-0 flex-col justify-center overflow-y-auto px-9 py-7">
          <div class="min-w-0">
            <div class="flex items-center gap-2">
              <img :src="nezzelMark" alt="" class="block size-7">
              <span class="text-[15px] font-semibold tracking-tight" dir="ltr">{{ t('shell.brand') }}</span>
            </div>
            <h1 id="welcome-title" class="wl-title">{{ t('welcome.title') }}</h1>
          </div>

          <ol class="wl-steps">
            <li v-for="s in STEPS" :key="s.k" class="wl-step" :class="{ on: step === s.k }">
              <component :is="s.icon" class="wl-step-ic" aria-hidden="true" />
              <span>{{ t(`welcome.steps.${s.key}`) }}</span>
            </li>
          </ol>
        </div>

        <div class="wl-demo flex min-w-0 flex-col border-s border-hairline bg-sidebar px-7 pt-7 pb-4">
          <welcome-clip class="flex-1" @step="step = $event" />

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

      <!-- Like a native setup window: what is on at the start, a way to change it, the default button at the end. -->
      <footer class="wl-foot flex flex-none items-center gap-3 border-t border-hairline px-5">
        <p class="wl-status min-w-0" data-testid="welcome-status">{{ t(`welcome.status.${statusKey}`) }}</p>
        <Popover>
          <PopoverTrigger as-child>
            <Button variant="outline" class="flex-none" data-testid="welcome-change">{{ t('welcome.change') }}</Button>
          </PopoverTrigger>
          <PopoverContent side="top" align="start" :side-offset="8" :collision-padding="12" class="wl-pop w-[360px] p-0" :aria-label="t('prefs.title')" data-testid="welcome-options">
            <div class="wl-row items-center">
              <Languages class="wl-ic" aria-hidden="true" />
              <label for="wl-lang" class="wl-label min-w-0 flex-1">{{ t('prefs.general.language') }}</label>
              <SelectField id="wl-lang" :model-value="language" :options="languageOptions" data-testid="welcome-language" @update:model-value="setLanguage" />
            </div>
            <div class="wl-row">
              <Power class="wl-ic" aria-hidden="true" />
              <span class="min-w-0 flex-1">
                <label id="wl-start-l" for="wl-start" class="wl-label">{{ t('welcome.start.label') }}</label>
                <small id="wl-start-d" class="wl-help">{{ t('welcome.start.help', { place: t(`welcome.place.${trayPlaceKey(os, capturePlatform.desktop)}`) }) }}</small>
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
            <welcome-shortcut :mac="os === 'mac'" class="wl-row-last" />
          </PopoverContent>
        </Popover>
        <span class="flex-1" />
        <Button size="sm" class="wl-cta" data-testid="welcome-start-button" @click="getStarted">{{ t('welcome.getStarted') }}</Button>
      </footer>
    </section>
  </config-provider>
</template>

<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import { ConfigProvider } from 'reka-ui';
import { invoke } from '@tauri-apps/api/core';
import { Check, Copy, Download, Languages, LifeBuoy, Link as LinkIcon, PictureInPicture2, Power } from '@lucide/vue';
import nezzelMark from '../../assets/brand/nezzel-mark.svg';
import { Button } from '../ui/button';
import { Switch } from '../ui/switch';
import { Popover, PopoverContent, PopoverTrigger } from '../ui/popover';
import { SelectField } from '../ui/select';
import WelcomeClip from './WelcomeClip.vue';
import WelcomeShortcut from './WelcomeShortcut.vue';
import { desktopOs, trayPlaceKey } from './desktopOs';
import { getDefaultLocale, localeDirection } from '../../i18n';
import { appLanguageOptions } from '../../helpers/languageOptions';
import { languageOptionsLookup } from '../../helpers/subtitles/languages';
import { usePlatform } from '../../composables/usePlatform';
import { useSettingsStore } from '../../stores/settings';
import { useOnboardingStore } from '../../stores/onboarding';
import { useToastStore } from '../../stores/toast';
import { useCapturePlatform } from '../../composables/useCapturePlatform';
import { TRY_LINK } from '@/generated/contract';
import { MAIN_PANE_ID } from '../../helpers/domIds';

/** The backend's TRY_LINK ("Me at the zoo"); only the browser fallback writes it from here. */
const SAMPLE_LABEL = TRY_LINK.replace(/^https:\/\/www\./, '');
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
const card = computed(() => settings.settings.input.watchClipboard);
const start = ref(true);
const step = ref(0);
const copied = ref(false);
const sheet = ref<HTMLElement>();
/** Keyboard in use: WebKitGTK left :focus-visible unmatched on most welcome stops (#36), so the
 * sheet decides itself and rings every focused control while the keyboard drives. */
const keyboard = ref(false);
function onKey(event: KeyboardEvent) {
  if (!event.metaKey && !event.ctrlKey) keyboard.value = true;
  if (event.key === 'Tab' && !event.metaKey && !event.ctrlKey) trapTab(event);
  // Return presses the default button, like a native dialog, while focus rests on the sheet itself.
  if (event.key === 'Enter' && event.target === sheet.value) getStarted();
}

/** A modal sheet keeps Tab inside. Past the last control WebKitGTK moved focus onto the page
 * itself and then the GTK window: two stops with nothing to outline (#36). */
function trapTab(event: KeyboardEvent) {
  const stops = [...(sheet.value?.querySelectorAll<HTMLElement>('button:not(:disabled), a[href], input:not(:disabled), [tabindex="0"]') ?? [])]
    .filter(el => el.getClientRects().length > 0);
  if (!stops.length) return;
  const at = stops.indexOf(document.activeElement as HTMLElement);
  const next = event.shiftKey
    ? (at <= 0 ? stops.at(-1) : undefined)
    : (at === -1 || at === stops.length - 1 ? stops[0] : undefined);
  if (!next) return;
  event.preventDefault();
  next.focus();
}
let copiedTimer: ReturnType<typeof setTimeout> | undefined;
onBeforeUnmount(() => clearTimeout(copiedTimer));

/** Crash and error reports: on by default, saved at once like the card switch. */
const reports = computed(() => settings.settings.privacy.sendReports);
function setReports(on: boolean) {
  void settings.setPrivacy('sendReports', on);
}

/** One sentence for what is on at the start; it follows the switches behind "Change". */
const statusKey = computed(() => (start.value ? (reports.value ? 'both' : 'startOnly') : (reports.value ? 'reportsOnly' : 'none')));

/** Language: the system's by default; picking one applies at once, the sheet re-reads in it. */
const language = computed(() => settings.settings.appearance?.language ?? 'system');
const languageOptions = computed(() => [
  { value: 'system', label: t('settings.appearance.language.options.system', { language: languageOptionsLookup.get(getDefaultLocale())?.nativeName ?? getDefaultLocale() }) },
  ...appLanguageOptions(language.value).map(option => ({ value: String(option.value), label: option.label })),
]);
function setLanguage(value: string | null | undefined) {
  if (value) void settings.patchNow({ appearance: { language: value } });
}

/** Writes the sample link (never reads the clipboard); with the card on, the real card shows for it. */
async function tryIt() {
  if (!card.value) return;
  try {
    await invoke('capture_try_link');
  } catch {
    useToastStore().showToast(t('welcome.try.failed'), { style: 'error' });
    return;
  }
  copied.value = true;
  clearTimeout(copiedTimer);
  copiedTimer = setTimeout(() => (copied.value = false), 1600);
}

function onLinkCopy() {
  // The user copied the link text themselves: same result as the button.
  setTimeout(() => void tryIt(), 0);
}

function getStarted() {
  void onboarding.complete({ card: card.value, start: start.value });
  // The sheet and its focused button go away now: keyboard and screen-reader focus lands on the main content.
  void nextTick(() => document.getElementById(MAIN_PANE_ID)?.focus({ preventScroll: true }));
}

onMounted(() => {
  // Focus starts in the sheet (screen readers read its title); Tab reaches the switches first.
  void nextTick(() => sheet.value?.focus({ preventScroll: true }));
});
</script>

<style scoped>
/* Own keyboard ring (see `keyboard`): the DESIGN.md focus outline on whatever has focus. */
.nz-welcome[data-keyboard] :deep(:is(button, a, input, [role="switch"], [tabindex="0"]):focus) {
  outline: 2px solid var(--nz-focus);
  outline-offset: 2px;
}

.wl-title { font-size: 22px; line-height: 1.25; font-weight: 600; letter-spacing: -.015em; margin: 18px 0 0; max-width: 20ch; text-wrap: balance; }
.wl-steps { list-style: none; margin: 22px -10px 0; padding: 0; display: flex; flex-direction: column; gap: 2px; }
.wl-step { display: flex; align-items: center; gap: 10px; height: 34px; padding-inline: 10px; border-radius: 6px; font-size: 13px; font-weight: 500; color: var(--foreground);
  transition: background-color 180ms ease-out, color 180ms ease-out; }
.wl-step-ic { width: 16px; height: 16px; flex: none; color: var(--muted-foreground); }
.wl-step.on .wl-step-ic { color: inherit; }
/* Heavier text too: the step in view is not told by colour alone. */
.wl-step.on { background: var(--primary-soft); color: var(--primary-soft-foreground); font-weight: 600; }

.wl-try { display: flex; align-items: center; gap: 10px; margin-top: 14px; font-size: 12.5px; color: var(--muted-foreground); }
.wl-link { flex: 1; min-width: 150px; height: 28px; display: flex; align-items: center; gap: 6px; padding-inline: 10px; border-radius: 999px; background: color-mix(in oklab, var(--foreground) 5.5%, transparent); color: var(--foreground); font-size: 12px; overflow: hidden; white-space: nowrap; user-select: all; cursor: text; }
.wl-copy { flex: none; }

.wl-foot { min-height: 52px; padding-block: 8px; background: var(--background); }
/* A narrow window wraps the sentence to a second line instead of cutting it. */
.wl-status { font-size: 12px; line-height: 1.4; color: var(--muted-foreground); text-wrap: pretty; }
.wl-cta { flex: none; min-width: 108px; white-space: nowrap; }

/* The options behind "Change": flat rows, like a Settings page. */
.wl-row { display: flex; align-items: flex-start; gap: 12px; padding: 11px 14px; border-bottom: 1px solid var(--hairline); }
.wl-ic { width: 16px; height: 16px; flex: none; margin-top: 1px; color: var(--muted-foreground); }
.wl-label { display: block; font-weight: 500; font-size: 13px; cursor: pointer; }
.wl-help { display: block; font-size: 12px; color: var(--muted-foreground); margin-top: 2px; text-wrap: pretty; }
.wl-sw { margin-top: 2px; }
.wl-row.items-center .wl-ic { margin-top: 0; }
.wl-row-last { padding-inline: 14px; border-bottom: 0; }

/* Windows narrower than 1000px (the default is 880): side by side would leave a small clip between two empty
   bands, so the clip takes the full width on top and the words sit under it, title beside the steps. */
@media (max-width: 999px) {
  .wl-main { grid-template-columns: minmax(0, 1fr); grid-template-rows: minmax(0, 1fr) auto; }
  .wl-demo { order: -1; border-inline-start: 0; border-bottom: 1px solid var(--hairline); padding-block: 20px 14px; }
  .wl-copy-col { display: grid; grid-template-columns: minmax(0, 1fr) minmax(0, 1fr); align-items: center; column-gap: 28px; padding-block: 20px; overflow: visible; }
  .wl-steps { margin-top: 0; }
}

/* Narrow panes: the "Try it" label gets its own line. */
.wl-demo { container-type: inline-size; }
@container (max-width: 460px) {
  .wl-try { flex-wrap: wrap; row-gap: 8px; }
  .wl-try > span:first-child { flex-basis: 100%; }
}

/* Arabic: one step larger, taller lines, no tracking. */
.is-rtl .wl-title { font-size: 23px; line-height: 1.5; letter-spacing: 0; }
.is-rtl .wl-step { font-size: 14px; height: 38px; }
.is-rtl .wl-try { font-size: 13.5px; }
.is-rtl .wl-status { font-size: 13px; }
:dir(rtl) .wl-ic { margin-top: 4px; }
:dir(rtl) .wl-label { font-size: 14px; }
:dir(rtl) .wl-help { font-size: 13px; margin-top: 0; }
:dir(rtl) .wl-sw { margin-top: 5px; }

/* Short windows (600px): the left column stays in view without scrolling. */
@media (max-height: 720px) {
  .wl-title { font-size: 20px; margin-top: 14px; }
  .is-rtl .wl-title { font-size: 21px; }
  .wl-steps { margin-top: 16px; }
}

@media (prefers-reduced-motion: reduce) {
  .wl-step { transition: none; }
}
</style>
