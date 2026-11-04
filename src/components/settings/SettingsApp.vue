<template>
  <settings-section :title="t('prefs.adv.app.title')">
    <settings-row for-id="notification-behavior" :label="t('prefs.adv.app.notify')" :help="t('prefs.adv.app.notifyHint')">
      <SelectField id="notification-behavior" v-model="settings.notifications.notificationBehavior" :options="behaviorOptions" />
    </settings-row>
    <settings-row for-id="autoStart" :label="t('settings.system.autoStart.label')" :help="t('settings.system.autoStart.hint')">
      <Switch id="autoStart" v-model="settings.system.autoStartEnabled" />
    </settings-row>
    <settings-row for-id="updateApp" :label="t('prefs.adv.app.updateApp')" :help="t('prefs.adv.app.updateAppHint')">
      <Switch id="updateApp" v-model="settings.update.updateApp" />
    </settings-row>
    <settings-row for-id="updateBinaries" :label="t('prefs.adv.app.updateTools')" :help="t('prefs.adv.app.updateToolsHint')">
      <Switch id="updateBinaries" v-model="settings.update.updateBinaries" />
    </settings-row>
    <settings-row for-id="sendReports" :label="t('report.setting.label')" :help="t('report.setting.help')">
      <Switch id="sendReports" :model-value="settings.privacy.sendReports" data-testid="settings-send-reports" @update:model-value="setSendReports" />
    </settings-row>

    <template #more>
      <settings-row
        for-id="autoStartMinimised"
        :label="isMac ? t('settings.system.autoStartMinimised.label.mac') : t('settings.system.autoStartMinimised.label.windows')"
        :help="minimisedHint"
      >
        <Switch id="autoStartMinimised" v-model="settings.system.autoStartMinimised" :disabled="!settings.system.autoStartEnabled || (!isMac && !settings.system.trayEnabled)" />
      </settings-row>
      <settings-row for-id="minimiseToTray" :label="t('prefs.adv.app.closeToTray')" :help="t('prefs.adv.app.closeToTrayHint')">
        <Switch id="minimiseToTray" v-model="minimiseToTray" :disabled="settings.system.trayEnabled === false" />
      </settings-row>
      <fieldset class="border-b border-hairline py-3 last:border-b-0" :disabled="!canShowNotifications">
        <legend class="float-start mb-1 w-full text-[13px]" :class="!canShowNotifications && 'opacity-50'">{{ t('prefs.adv.app.notifyWhich') }}</legend>
        <div class="grid grid-cols-1 gap-x-8 sm:grid-cols-2" :class="!canShowNotifications && 'opacity-50'">
          <label v-for="(label, kind) in notificationKindLabels" :key="kind" class="flex min-h-7 cursor-pointer items-center gap-2.5 text-[13px]">
            <Checkbox :model-value="!isDisabled(kind as NotificationKind)" :disabled="!canShowNotifications" @update:model-value="setDisabled(kind as NotificationKind, !$event)" />
            <span>{{ label }}</span>
          </label>
        </div>
      </fieldset>
    </template>
  </settings-section>
</template>

<script setup lang="ts">
import { computed, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import Checkbox from '@/components/ui/checkbox/Checkbox.vue';
import { SelectField } from '@/components/ui/select';
import Switch from '@/components/ui/switch/Switch.vue';
import SettingsRow from './SettingsRow.vue';
import SettingsSection from './SettingsSection.vue';
import { CloseBehavior, NotificationBehavior, Settings } from '../../tauri/types/config.ts';
import { NotificationKind } from '../../tauri/types/app';
import { usePlatform } from '../../composables/usePlatform.ts';
import { useSettingsStore } from '../../stores/settings';

const i18n = useI18n();
const t = i18n.t;
const { isWindows, isMac } = usePlatform();
const settings = defineModel<Settings>({ required: true });
const settingsStore = useSettingsStore();

/** Consent: saved at once, not after the page's save delay, so "off" holds even if the app closes now. */
function setSendReports(on: boolean) {
  settings.value.privacy.sendReports = on;
  void settingsStore.patchNow({ privacy: { sendReports: on } });
}

const behaviorOptions = computed(() => [
  { value: NotificationBehavior.Always, label: t('settings.notifications.behavior.options.always') },
  { value: NotificationBehavior.OnBackground, label: t('settings.notifications.behavior.options.onBackground') },
  { value: NotificationBehavior.Never, label: t('settings.notifications.behavior.options.never') },
]);
const canShowNotifications = computed(() => settings.value.notifications.notificationBehavior !== NotificationBehavior.Never);
const notificationKindLabels = computed(() => {
  const kinds: Record<NotificationKind, string> = i18n.tm('settings.notifications.disabled.kinds');
  return kinds;
});
function isDisabled(kind: NotificationKind): boolean {
  return settings.value.notifications.disabledNotifications.includes(kind);
}
function setDisabled(kind: NotificationKind, disabled: boolean) {
  const list = settings.value.notifications.disabledNotifications;
  const index = list.indexOf(kind);
  if (disabled && index === -1) list.push(kind);
  if (!disabled && index !== -1) list.splice(index, 1);
}

// "Keep running in the background" (the tray) lives in General; closing to it depends on it being on.
const minimiseToTray = computed<boolean>({
  get: () => settings.value.system.closeBehavior === CloseBehavior.Hide && settings.value.system.trayEnabled === true,
  set: (value) => { settings.value.system.closeBehavior = value ? CloseBehavior.Hide : CloseBehavior.Exit; },
});
const minimisedHint = computed(() => {
  if (isMac.value) return t('settings.system.autoStartMinimised.hint.mac');
  if (isWindows.value) return t('settings.system.autoStartMinimised.hint.windows');
  return t('settings.system.autoStartMinimised.hint.generic');
});

watch(() => settings.value.system.trayEnabled, (now, before) => {
  if (before && !now) minimiseToTray.value = false;
  if (!isMac.value) settings.value.system.autoStartMinimised = false;
});
watch(() => settings.value.system.autoStartEnabled, (now, before) => {
  if (before && !now) settings.value.system.autoStartMinimised = false;
});
</script>
