<template>
  <div class="wl-row" data-testid="welcome-shortcut">
    <keyboard class="wl-ic" aria-hidden="true" />
    <span class="min-w-0 flex-1">
      <span id="wl-key-l" class="wl-label">{{ t('welcome.shortcut.label') }}</span>
      <small v-if="ready" id="wl-key-d" class="wl-help">
        <i18n-t keypath="welcome.shortcut.help" scope="global">
          <template #keys><span class="wl-keys" dir="ltr"><Kbd v-for="k in keys" :key="k">{{ k }}</Kbd></span></template>
        </i18n-t>
      </small>
      <i18n-t v-else-if="wayland" id="wl-key-d" keypath="shortcut.wayland" tag="small" scope="global" class="wl-help" data-testid="welcome-shortcut-wayland">
        <template #command><code dir="ltr" class="wl-cmd">{{ CAPTURE_COMMAND }}</code></template>
      </i18n-t>
      <small v-else-if="!skipped" id="wl-key-d" class="wl-help text-amber-foreground" role="status">{{ t('welcome.shortcut.taken') }}</small>
      <small v-else id="wl-key-d" class="wl-help">{{ t('welcome.shortcut.later') }}</small>
    </span>
    <span v-if="!ready && !skipped && !wayland" class="flex flex-none items-center gap-1.5">
      <shortcut-recorder
        v-model="shortcut"
        :mac="mac"
        :label="t('welcome.shortcut.label')"
        :default-value="DEFAULT_CAPTURE_SHORTCUT"
        :status="status"
        :validate="validate"
      />
      <Button variant="ghost" data-testid="welcome-shortcut-skip" @click="skipped = true">{{ t('welcome.shortcut.skip') }}</Button>
    </span>
  </div>
</template>

<script setup lang="ts">
import { computed, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import { Keyboard } from '@lucide/vue';
import { Button } from '../ui/button';
import Kbd from '../ui/kbd/Kbd.vue';
import ShortcutRecorder from '../settings/ShortcutRecorder.vue';
import { CAPTURE_COMMAND, DEFAULT_CAPTURE_SHORTCUT, isReservedShortcut, shortcutKeys } from '../../helpers/shortcut';
import { useSettingsStore } from '../../stores/settings';
import { useCaptureShortcut } from '../../composables/useCaptureShortcut';

const props = defineProps<{ mac: boolean }>();
const { t } = useI18n();
const settings = useSettingsStore();

// The OS result: `active` is what the user presses now (a fallback when the default was taken).
const capture = useCaptureShortcut();
const status = computed(() => ({ seq: capture.seq, failed: capture.failed }));
const failed = computed(() => capture.failed);
const active = computed(() => settings.settings.input.globalShortcuts && !!(capture.active ?? settings.settings.input.captureShortcut));

const skipped = ref(false);
// Wayland: no recorder, the combo can't be grabbed whatever it is. A system shortcut can run the command.
const wayland = computed(() => capture.reason === 'wayland');
const ready = computed(() => active.value && !failed.value);
const keys = computed(() => shortcutKeys(capture.active ?? settings.settings.input.captureShortcut, props.mac));
const shortcut = computed({
  get: () => settings.settings.input.captureShortcut,
  set: (value: string) => void settings.patchNow({ input: { captureShortcut: value, globalShortcuts: true } }),
});
function validate(accelerator: string): string | null {
  if (isReservedShortcut(accelerator, props.mac)) return props.mac ? t('shortcut.reservedMac') : t('shortcut.reservedSystem');
  return null;
}
</script>

<style scoped>
.wl-row { display: flex; align-items: flex-start; gap: 12px; padding: 12px 0; border-bottom: 1px solid var(--hairline); }
.wl-ic { width: 16px; height: 16px; flex: none; margin-top: 1px; color: var(--muted-foreground); }
.wl-label { display: block; font-weight: 500; font-size: 13px; }
.wl-help { display: block; font-size: 12px; color: var(--muted-foreground); margin-top: 2px; text-wrap: pretty; }
.wl-keys { display: inline-flex; gap: 3px; vertical-align: 1px; }
.wl-cmd { font-family: var(--font-mono, ui-monospace, monospace); font-size: 11.5px; unicode-bidi: isolate; }
:dir(rtl) .wl-label { font-size: 14px; }
:dir(rtl) .wl-help { font-size: 13px; margin-top: 0; }
:dir(rtl) .wl-ic { margin-top: 4px; }
</style>
