<template>
  <!-- Developer-only options, collapsed at the very end of Advanced. -->
  <Collapsible class="group mt-8 border-t border-hairline pt-3" :unmount-on-hide="false">
    <CollapsibleTrigger class="inline-flex h-7 cursor-pointer items-center gap-1 rounded-[6px] text-[12px] font-semibold text-foreground/65 outline-none focus-visible:outline-solid focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--nz-focus)]">
      <ChevronRight class="size-3.5 transition-transform duration-150 group-data-[state=open]:rotate-90 rtl:rotate-180 rtl:group-data-[state=open]:rotate-90" aria-hidden="true" />
      {{ t('prefs.adv.dev.title') }}
    </CollapsibleTrigger>
    <CollapsibleContent>
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
    </CollapsibleContent>
  </Collapsible>
</template>

<script setup lang="ts">
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from '../ui/collapsible';
import { Textarea } from '../ui/textarea';
import { computed } from 'vue';
import { useI18n } from 'vue-i18n';
import { ChevronRight } from '@lucide/vue';
import SettingsRow from './SettingsRow.vue';
import { Settings } from '../../tauri/types/config.ts';

const { t } = useI18n();
const settings = defineModel<Settings>({ required: true });
const showFlagWarning = computed(() => settings.value.network.extractorArgs.toLowerCase().includes('--extractor-args'));
</script>
