<template>
  <settings-section :title="t('settings.network.legend')">
    <settings-link-row :to="{ name: 'settings.logins' }" :label="t('prefs.adv.net.logins')" :help="t('prefs.adv.net.loginsHint')" />
    <settings-row for-id="enableProxy" :label="t('prefs.adv.net.proxy')" :help="t('prefs.adv.net.proxyHint')">
      <Switch id="enableProxy" v-model="proxyEnabled" />
    </settings-row>
    <div v-if="proxyEnabled" class="border-b border-hairline pb-3 last:border-b-0">
      <label for="proxyUrl" class="sr-only">{{ t('prefs.adv.net.proxyAddress') }}</label>
      <Input
        id="proxyUrl"
        v-model.trim="proxy"
        type="text"
        dir="ltr"
        spellcheck="false"
        autocomplete="off"
        :class="['font-mono text-[12px]', proxyInvalid && 'border-destructive']"
        placeholder="socks5://127.0.0.1:1080"
        :aria-invalid="proxyInvalid || undefined"
        aria-describedby="proxyUrlHint" />
      <p id="proxyUrlHint" class="mt-1.5 text-[12px]" :class="proxyInvalid ? 'text-destructive' : 'text-muted-foreground'">
        {{ proxyInvalid ? t('prefs.adv.net.proxyInvalid') : t('prefs.adv.net.proxyExample') }}
      </p>
    </div>

    <template #more>
      <settings-row for-id="impersonate" :label="t('prefs.adv.net.impersonate')" :help="t('prefs.adv.net.impersonateHint')">
        <SelectField id="impersonate" v-model="network.impersonate" :options="impersonateOptions" />
      </settings-row>
    </template>
  </settings-section>
</template>

<script setup lang="ts">
import { Input } from '../ui/input';
import { computed } from 'vue';
import { useI18n } from 'vue-i18n';
import { SelectField } from '@/components/ui/select';
import Switch from '@/components/ui/switch/Switch.vue';
import SettingsLinkRow from './SettingsLinkRow.vue';
import SettingsRow from './SettingsRow.vue';
import SettingsSection from './SettingsSection.vue';
import { buildImpersonatePresets, isValidProxyUrl } from '../../helpers/network.ts';
import { Settings } from '../../tauri/types/config.ts';

const { t } = useI18n();
const settings = defineModel<Settings>({ required: true });

const network = computed(() => settings.value.network);
const impersonateOptions = computed(() => [
  { value: 'none', label: t('prefs.adv.net.impersonateOff') },
  { value: 'any', label: t('prefs.adv.net.impersonateAny') },
  ...buildImpersonatePresets(t).filter(preset => preset.value !== 'none' && preset.value !== 'any').map(preset => ({ value: String(preset.value), label: preset.label })),
]);
const proxyEnabled = computed<boolean>({
  get: () => network.value.enableProxy === true,
  set: (value) => { settings.value.network.enableProxy = value; },
});
const proxy = computed<string>({
  get: () => network.value.proxy ?? '',
  set: (value) => { settings.value.network.proxy = value; },
});
const proxyInvalid = computed(() => proxy.value.length > 0 && !isValidProxyUrl(proxy.value));
</script>
