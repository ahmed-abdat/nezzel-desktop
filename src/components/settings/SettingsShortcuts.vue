<template>
  <settings-section :title="t('prefs.adv.keys.title')">
    <h3 class="pt-1 text-[12px] font-medium text-muted-foreground">{{ t('prefs.adv.keys.anywhere') }}</h3>
    <settings-row for-id="keys-global" :label="t('prefs.adv.keys.globalOn')" :help="t('prefs.adv.keys.globalOnHint')">
      <Switch id="keys-global" v-model="settings.input.globalShortcuts" />
    </settings-row>
    <settings-row :label="t('prefs.adv.keys.quick')" :help="t('prefs.adv.keys.defaultIs', { keys: shortcutLabel(DEFAULT_CAPTURE_SHORTCUT, isMac) })">
      <shortcut-recorder
        v-model="settings.input.captureShortcut"
        :mac="isMac"
        :label="t('prefs.adv.keys.quick')"
        :default-value="DEFAULT_CAPTURE_SHORTCUT"
        :disabled="!settings.input.globalShortcuts || capture.reason === 'wayland'"
        :status="captureStatus"
        :validate="validateFor('quick')"
      />
    </settings-row>
    <i18n-t v-if="capture.reason === 'wayland' && settings.input.globalShortcuts" keypath="shortcut.wayland" tag="p" scope="global" class="pb-3 text-[12px] text-muted-foreground" role="status" data-testid="shortcut-wayland">
      <template #command><code dir="ltr" class="font-mono text-[11.5px]">{{ CAPTURE_COMMAND }}</code></template>
    </i18n-t>
    <p v-else-if="capture.startupFailed && capture.seq === 0 && settings.input.globalShortcuts" class="pb-3 text-[12px] text-destructive" role="status">{{ t('shortcut.taken') }}</p>
    <i18n-t v-else-if="fallbackActive" keypath="shortcut.usingFallback" tag="p" scope="global" class="pb-3 text-[12px] text-muted-foreground" role="status">
      <template #wanted><bdi dir="ltr">{{ shortcutLabel(settings.input.captureShortcut, isMac) }}</bdi></template>
      <template #active><bdi dir="ltr">{{ shortcutLabel(capture.active ?? '', isMac) }}</bdi></template>
    </i18n-t>

    <h3 class="pt-4 text-[12px] font-medium text-muted-foreground">{{ t('prefs.adv.keys.inside') }}</h3>
    <settings-row :label="t('prefs.adv.keys.paste')" :help="t('prefs.adv.keys.pasteHint')">
      <shortcut-recorder model-value="CommandOrControl+V" :mac="isMac" :label="t('prefs.adv.keys.paste')" readonly />
    </settings-row>
    <settings-row :label="t('prefs.adv.keys.downloadAll')" :help="t('prefs.adv.keys.defaultIs', { keys: shortcutLabel(IN_APP_SHORTCUT_DEFAULTS.downloadAll, isMac) })">
      <shortcut-recorder
        v-model="inAppShortcuts.downloadAll.value"
        :mac="isMac"
        :label="t('prefs.adv.keys.downloadAll')"
        :default-value="IN_APP_SHORTCUT_DEFAULTS.downloadAll"
        :validate="validateFor('downloadAll')"
      />
    </settings-row>
    <settings-row :label="t('prefs.adv.keys.addAndDownload')" :help="t('prefs.adv.keys.addAndDownloadHint', { keys: shortcutLabel(IN_APP_SHORTCUT_DEFAULTS.addAndDownload, isMac) })">
      <shortcut-recorder
        v-model="inAppShortcuts.addAndDownload.value"
        :mac="isMac"
        :label="t('prefs.adv.keys.addAndDownload')"
        :default-value="IN_APP_SHORTCUT_DEFAULTS.addAndDownload"
        :validate="validateFor('addAndDownload')"
      />
    </settings-row>
    <button
      v-if="!allDefault"
      type="button"
      class="mt-1 inline-flex h-7 items-center rounded-[6px] text-[12.5px] font-medium text-primary outline-none hover:underline focus-visible:ring-2 focus-visible:ring-ring/40"
      @click="resetAll"
    >{{ t('prefs.adv.keys.resetAll') }}</button>
  </settings-section>
</template>

<script setup lang="ts">
import { computed } from 'vue';
import { useI18n } from 'vue-i18n';
import Switch from '@/components/ui/switch/Switch.vue';
import SettingsRow from './SettingsRow.vue';
import SettingsSection from './SettingsSection.vue';
import ShortcutRecorder from './ShortcutRecorder.vue';
import { Settings } from '../../tauri/types/config.ts';
import { usePlatform } from '../../composables/usePlatform.ts';
import { useCaptureShortcut } from '../../composables/useCaptureShortcut';
import { IN_APP_SHORTCUT_DEFAULTS, useUiPrefs } from '../../composables/useUiPrefs';
import { CAPTURE_COMMAND, DEFAULT_CAPTURE_SHORTCUT, isReservedShortcut, sameShortcut, shortcutLabel } from '../../helpers/shortcut';

const { t } = useI18n();
const { isMac } = usePlatform();
const { inAppShortcuts } = useUiPrefs();
const settings = defineModel<Settings>({ required: true });

// Nezzel's own keys that a new shortcut must not take.
type Action = 'quick' | 'downloadAll' | 'addAndDownload';
const BUILT_IN = ['CommandOrControl+V', 'CommandOrControl+B', 'CommandOrControl+\\', 'CommandOrControl+,'];

function current(action: Action): string {
  if (action === 'quick') return settings.value.input.captureShortcut;
  return inAppShortcuts[action].value;
}

function validateFor(action: Action) {
  return (accelerator: string): string | null => {
    if (isReservedShortcut(accelerator, isMac.value)) return isMac.value ? t('shortcut.reservedMac') : t('shortcut.reservedSystem');
    if (BUILT_IN.some(key => sameShortcut(key, accelerator))) return t('shortcut.builtIn');
    const others: Action[] = (['quick', 'downloadAll', 'addAndDownload'] as Action[]).filter(other => other !== action);
    const clash = others.find(other => current(other) && sameShortcut(current(other), accelerator));
    if (clash) return t('shortcut.usedFor', { action: t(`prefs.adv.keys.${clash}`) });
    return null;
  };
}

const allDefault = computed(() => sameShortcut(settings.value.input.captureShortcut, DEFAULT_CAPTURE_SHORTCUT)
  && inAppShortcuts.downloadAll.value === IN_APP_SHORTCUT_DEFAULTS.downloadAll
  && inAppShortcuts.addAndDownload.value === IN_APP_SHORTCUT_DEFAULTS.addAndDownload);

function resetAll() {
  settings.value.input.captureShortcut = DEFAULT_CAPTURE_SHORTCUT;
  inAppShortcuts.downloadAll.value = IN_APP_SHORTCUT_DEFAULTS.downloadAll;
  inAppShortcuts.addAndDownload.value = IN_APP_SHORTCUT_DEFAULTS.addAndDownload;
}

// The OS reports every (re)registration of the Quick download shortcut; the recorder rolls back on failure.
const capture = useCaptureShortcut();
const captureStatus = computed(() => ({ seq: capture.seq, failed: capture.failed }));
// The default was taken, so the backend registered the fallback instead.
const fallbackActive = computed(() => settings.value.input.globalShortcuts && !capture.failed && !!capture.active
  && !sameShortcut(capture.active, settings.value.input.captureShortcut));
</script>
