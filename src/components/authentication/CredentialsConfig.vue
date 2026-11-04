<template>
  <!-- Native disclosure (keeps open state for tests and screen readers), flat like the per-download Advanced. -->
  <Collapsible data-testid="auth-details" class="group mt-8" v-model:open="open" :unmount-on-hide="false">
    <CollapsibleTrigger class="-ms-1 inline-flex h-7 cursor-pointer items-center gap-1 rounded-[6px] px-1 text-[12px] font-semibold text-foreground/65 outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/40">
      <ChevronRight class="size-3.5 transition-transform duration-(--nz-dur-fast) group-data-[state=open]:rotate-90 rtl:rotate-180 rtl:group-data-[state=open]:rotate-90" aria-hidden="true" />
      {{ t('accounts.saved') }}
    </CollapsibleTrigger>
    <CollapsibleContent>
      <p class="mb-2 mt-1 text-nz-caption text-muted-foreground text-pretty">{{ t('prefs.logins.accountsHint') }}</p>
      <credentials-init v-if="!strongholdStore.status.unlocked" />
      <div v-else>
        <settings-section :title="t('auth.credentials.basicAuth.legend')" :note="t('auth.credentials.basicAuth.legendLabel')">
          <settings-row :label="t('auth.credentials.labels.username')">
            <base-secret-input id="basic-auth-username" v-model="strongholdFields.username" :label="t('auth.credentials.labels.username')" />
          </settings-row>
          <settings-row :label="t('auth.credentials.labels.password')">
            <base-secret-input id="basic-auth-password" v-model="strongholdFields.password" password :label="t('auth.credentials.labels.password')" />
          </settings-row>
        </settings-section>
        <settings-section :title="t('auth.credentials.videoPassword.legend')">
          <settings-row :label="t('auth.credentials.labels.password')" :help="t('auth.credentials.videoPassword.legendLabel')">
            <base-secret-input id="video-password" v-model="strongholdFields.videoPassword" password :label="t('auth.credentials.labels.password')" />
          </settings-row>
        </settings-section>
        <settings-section :title="t('auth.credentials.bearerToken.legend')">
          <settings-row :label="t('auth.credentials.labels.token')" :help="t('auth.credentials.bearerToken.legendLabel')">
            <base-secret-input id="bearer-token" v-model="strongholdFields.bearer" password :label="t('auth.credentials.labels.token')" />
          </settings-row>
        </settings-section>
        <settings-section :title="t('auth.credentials.customHeaders.legend')">
          <settings-row stacked for-id="custom-headers" :label="t('auth.credentials.labels.headers')" :help="t('perDownload.network.headersHint')">
            <Textarea
              id="custom-headers"
              :model-value="strongholdFields.headers ?? ''"
              dir="ltr"
              rows="3"
              spellcheck="false"
              class="py-1.5 font-mono text-[12px]"
              :placeholder="t('auth.credentials.customHeaders.placeholder')"
              @input="onHeadersInput" />
          </settings-row>
        </settings-section>
      </div>
    </CollapsibleContent>
  </Collapsible>
</template>

<script setup lang="ts">
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from '../ui/collapsible';
import { Textarea } from '../ui/textarea';
import { ChevronRight } from '@lucide/vue';
import SettingsRow from '../settings/SettingsRow.vue';
import SettingsSection from '../settings/SettingsSection.vue';
import { StrongholdFields, useStrongholdStore } from '../../stores/stronghold';
import CredentialsInit from './CredentialsInit.vue';
import BaseSecretInput from '../base/BaseSecretInput.vue';
import { PropType, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';

const { t } = useI18n();
const strongholdStore = useStrongholdStore();
// Opens itself while saved values exist (keys load after mount), like the old <details :open>.
const open = ref(strongholdStore.hasAvailableKeys());
watch(() => strongholdStore.hasAvailableKeys(), (value) => {
  open.value = value;
});

const strongholdFields = defineModel({
  type: Object as PropType<StrongholdFields>,
  required: true,
});

// Blank means none, like the secret fields: saving it deletes the stored headers.
function onHeadersInput(event: Event) {
  const value = (event.target as HTMLTextAreaElement).value;
  strongholdFields.value.headers = value.trim() === '' ? null : value;
}
</script>
