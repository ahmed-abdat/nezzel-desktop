<script setup lang="ts">
import { useI18n } from 'vue-i18n';
import { candidateLabel } from '../../helpers/cookieBrowsers';
import type { LoginCandidate } from '../../tauri/types/login';

defineProps<{ candidate?: LoginCandidate; host: string }>();
const { t } = useI18n();
</script>

<template>
  <template v-if="candidate">
    <i18n-t v-if="candidate.signedIn" keypath="downloads.login.profileStatus" tag="span" scope="global">
      <template #profile><bdi dir="ltr">{{ candidateLabel(candidate) }}</bdi></template>
      <template #status><i18n-t keypath="downloads.login.signedIn" tag="span" scope="global"><template #host><bdi dir="ltr">{{ host }}</bdi></template></i18n-t></template>
    </i18n-t>
    <bdi v-else dir="ltr">{{ candidateLabel(candidate) }}</bdi>
  </template>
  <span v-else>{{ t('downloads.login.useFrom') }}</span>
</template>
