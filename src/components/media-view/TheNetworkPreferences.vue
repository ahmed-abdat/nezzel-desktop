<template>
  <div>
    <settings-section :note="t('perDownload.network.note')">
      <network-settings-editor v-model="networkState" id-prefix="override" />
      <settings-row for-id="override-impersonate" :label="t('prefs.adv.net.impersonate')" :help="t('prefs.adv.net.impersonateHint')">
        <select-field id="override-impersonate" v-model="networkState.impersonate" :options="impersonateOptions" />
      </settings-row>
    </settings-section>

    <base-disclosure class="mt-8" :label="t('perDownload.advanced')">
      <settings-row stacked for-id="override-headers" :label="t('perDownload.network.headers')" :help="t('perDownload.network.headersHint')">
        <Textarea
          id="override-headers"
          v-model="headersText"
          dir="ltr"
          rows="3"
          spellcheck="false"
          class="py-1.5 font-mono text-[12px]"
          placeholder="X-Header: value" />
      </settings-row>
      <settings-row stacked for-id="override-extractor-args" :label="t('prefs.adv.dev.extractor')" :help="t('prefs.adv.dev.extractorHint')">
        <Textarea
          id="override-extractor-args"
          v-model="networkState.extractorArgs"
          dir="ltr"
          rows="2"
          spellcheck="false"
          class="py-1.5 font-mono text-[12px]"
          :placeholder="t('settings.network.extractorArgs.placeholder')" />
        <p v-if="showExtractorFlagWarning" class="mt-1.5 text-nz-caption text-amber-foreground" role="alert">
          {{ t('settings.network.extractorArgs.flagWarning') }}
        </p>
      </settings-row>
    </base-disclosure>
  </div>
</template>

<script setup lang="ts">
import { Textarea } from '../ui/textarea';
import { computed, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import NetworkSettingsEditor from '../network/NetworkSettingsEditor.vue';
import { SelectField } from '../ui/select';
import SettingsRow from '../settings/SettingsRow.vue';
import SettingsSection from '../settings/SettingsSection.vue';
import BaseDisclosure from '../base/BaseDisclosure.vue';
import { buildImpersonatePresets } from '../../helpers/network.ts';
import { useMediaOptionsStore } from '../../stores/media/options.ts';
import { DownloadOverrides } from '../../tauri/types/media.ts';
import { defaultNetworkSettings, type NetworkSettings } from '../../tauri/types/config.ts';
import { useSettingsStore } from '../../stores/settings.ts';

const props = defineProps({
  groupId: {
    type: String,
    required: true,
  },
});

const { t } = useI18n();
const optionsStore = useMediaOptionsStore();
const settingsStore = useSettingsStore();

const networkState = ref<NetworkSettings>({
  ...defaultNetworkSettings,
});
const headersText = ref('');

/** "Off" and "Any browser" use the plain Settings copy; the rest are named browsers. */
const impersonateOptions = computed(() => [
  { value: 'none', label: t('prefs.adv.net.impersonateOff') },
  { value: 'any', label: t('prefs.adv.net.impersonateAny') },
  ...buildImpersonatePresets(t).filter(preset => preset.value !== 'none' && preset.value !== 'any'),
]);
const showExtractorFlagWarning = computed(() =>
  networkState.value.extractorArgs.toLowerCase().includes('--extractor-args'));

const parseHeaders = (value: string): string[] =>
  value
    .split('\n')
    .map(line => line.trim())
    .filter(Boolean);

const syncFromStore = () => {
  const current = optionsStore.getOverrides(props.groupId);
  networkState.value = {
    enableProxy: current?.network?.enableProxy ?? false,
    proxy: current?.network?.proxy ?? '',
    impersonate: current?.network?.impersonate ?? 'none',
    extractorArgs: current?.network?.extractorArgs ?? '',
  };
  headersText.value = (current?.auth?.headers ?? []).join('\n');
};

watch(() => props.groupId, syncFromStore, { immediate: true });

watch([networkState, headersText], () => {
  const trimmedProxy = networkState.value.proxy?.trim() ?? '';
  const headers = parseHeaders(headersText.value);
  const globalNetwork = settingsStore.settings.network;
  const extractorArgs = networkState.value.extractorArgs.trim();
  const globalExtractorArgs = settingsStore.settings.network.extractorArgs.trim();

  const networkPayload = {
    enableProxy: networkState.value.enableProxy ?? false,
    proxy: trimmedProxy || undefined,
    impersonate: networkState.value.impersonate !== 'none'
      ? networkState.value.impersonate
      : undefined,
    extractorArgs: extractorArgs || undefined,
  };
  const hasNetwork
    = (globalNetwork.enableProxy ?? false) !== (networkState.value.enableProxy ?? false)
      || (globalNetwork.proxy ?? '') !== trimmedProxy
      || (globalNetwork.impersonate ?? 'none') !== networkState.value.impersonate
      || globalExtractorArgs !== extractorArgs;
  const hasAuth = headers.length > 0;

  const existing = optionsStore.getOverrides(props.groupId);
  const next: DownloadOverrides = {
    ...(existing ?? {}),
  };
  if (hasNetwork) next.network = networkPayload;
  else delete next.network;

  if (hasAuth) next.auth = { ...(existing?.auth ?? {}), headers };
  else if (existing?.auth) {
    const nextAuth = { ...existing.auth };
    delete nextAuth.headers;
    next.auth = Object.keys(nextAuth).length ? nextAuth : undefined;
  } else {
    delete next.auth;
  }

  const hasAny = Object.values(next).some(v => v !== undefined);
  if (hasAny) {
    optionsStore.setOverrides(props.groupId, next);
  } else {
    optionsStore.removeOverrides(props.groupId);
  }
}, { deep: true });
</script>
