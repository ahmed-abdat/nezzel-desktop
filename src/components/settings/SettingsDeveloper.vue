<template>
  <!-- Developer-only options, collapsed at the very end of Advanced. -->
  <BaseDisclosure class="mt-8 border-t border-hairline pt-3" :label="t('prefs.adv.dev.title')">
    <settings-row stacked for-id="extractorArgs" :label="t('prefs.adv.dev.extractor')">
      <template #help>
        {{ t('prefs.adv.dev.extractorHint') }}
        <a href="https://github.com/yt-dlp/yt-dlp#extractor-arguments" target="_blank" rel="noopener" class="text-primary hover:underline">{{ t('prefs.adv.dev.whatsThis') }}</a>
      </template>
      <div class="w-full">
        <Textarea
          id="extractorArgs"
          v-model="settings.network.extractorArgs"
          dir="ltr"
          rows="3"
          spellcheck="false"
          class="py-1.5 font-mono text-[12px]"
          :placeholder="t('settings.network.extractorArgs.placeholder')" />
        <p v-if="showFlagWarning" class="mt-1.5 text-[12px] text-amber-foreground" role="alert">{{ t('settings.network.extractorArgs.flagWarning') }}</p>
      </div>
    </settings-row>
  </BaseDisclosure>
</template>

<script setup lang="ts">
import BaseDisclosure from '../base/BaseDisclosure.vue';
import { Textarea } from '../ui/textarea';
import { computed } from 'vue';
import { useI18n } from 'vue-i18n';
import SettingsRow from './SettingsRow.vue';
import { Settings } from '../../tauri/types/config.ts';

const { t } = useI18n();
const settings = defineModel<Settings>({ required: true });
const showFlagWarning = computed(() => settings.value.network.extractorArgs.toLowerCase().includes('--extractor-args'));
</script>
