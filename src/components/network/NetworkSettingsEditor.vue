<template>
  <!-- Proxy rows, same pattern and copy as Settings > Advanced > Network. -->
  <settings-row :for-id="fieldId('enable-proxy', 'enableProxy')" :label="t('prefs.adv.net.proxy')" :help="t('prefs.adv.net.proxyHint')">
    <Switch :id="fieldId('enable-proxy', 'enableProxy')" v-model="proxyEnabled" />
  </settings-row>
  <div v-if="proxyEnabled" class="border-b border-hairline pb-3">
    <label :for="fieldId('proxy', 'proxyUrl')" class="sr-only">{{ t('prefs.adv.net.proxyAddress') }}</label>
    <Input
      :id="fieldId('proxy', 'proxyUrl')"
      v-model="proxy"
      type="text"
      dir="ltr"
      spellcheck="false"
      autocomplete="off"
      placeholder="socks5://127.0.0.1:1080"
      :class="['font-mono text-[12px]', proxyInvalid && 'border-destructive']"
      :aria-invalid="proxyInvalid || undefined"
      :aria-describedby="`${fieldId('proxy', 'proxyUrl')}-hint`" />
    <p :id="`${fieldId('proxy', 'proxyUrl')}-hint`" class="mt-1.5 text-nz-caption" :class="proxyInvalid ? 'text-destructive' : 'text-muted-foreground'">
      {{ proxyInvalid ? t('prefs.adv.net.proxyInvalid') : t('prefs.adv.net.proxyExample') }}
    </p>
  </div>

</template>

<script setup lang="ts">
import { Input } from '../ui/input';
import { computed } from 'vue';
import { useI18n } from 'vue-i18n';
import Switch from '../ui/switch/Switch.vue';
import SettingsRow from '../settings/SettingsRow.vue';
import { isValidProxyUrl } from '../../helpers/network.ts';
import type { NetworkSettings } from '../../tauri/types/config.ts';

const networkState = defineModel<NetworkSettings>({ required: true });

const { idPrefix } = defineProps({
  idPrefix: {
    type: String,
    required: false,
    default: '',
  },
});

const { t } = useI18n();

const fieldId = (name: string, fallback: string) => (idPrefix ? `${idPrefix}-${name}` : fallback);

const proxyEnabled = computed<boolean>({
  get: () => networkState.value.enableProxy === true,
  set: (value) => { networkState.value.enableProxy = value; },
});
const proxy = computed<string>({
  get: () => networkState.value.proxy ?? '',
  set: (value) => { networkState.value.proxy = value; },
});
const proxyInvalid = computed(() => proxy.value.trim() !== '' && !isValidProxyUrl(proxy.value.trim()));
</script>
