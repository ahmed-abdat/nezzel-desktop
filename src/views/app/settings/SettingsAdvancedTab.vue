<template>
  <div>
    <p class="mb-6 text-[12.5px] leading-relaxed text-muted-foreground text-pretty">{{ t('prefs.advanced.note') }}</p>

    <settings-output v-model="settings" />
    <settings-shortcuts v-model="settings" />
    <settings-network v-model="settings" />
    <settings-performance v-model="settings" />
    <settings-sponsor-block v-model="settings" />
    <settings-app v-model="settings" />
    <settings-developer v-model="settings" />

    <div class="mt-10 mb-4">
      <!-- Text-only button: pull it back by its padding so the label lines up with the column. -->
      <Button variant="destructive" class="-ms-2.5" @click="emit('reset')">{{ t('prefs.advanced.reset') }}</Button>
    </div>
  </div>
</template>

<script setup lang="ts">
import { useI18n } from 'vue-i18n';
import { Button } from '@/components/ui/button';
import SettingsApp from '../../../components/settings/SettingsApp.vue';
import SettingsDeveloper from '../../../components/settings/SettingsDeveloper.vue';
import SettingsNetwork from '../../../components/settings/SettingsNetwork.vue';
import SettingsOutput from '../../../components/settings/SettingsOutput.vue';
import SettingsPerformance from '../../../components/settings/SettingsPerformance.vue';
import SettingsShortcuts from '../../../components/settings/SettingsShortcuts.vue';
import SettingsSponsorBlock from '../../../components/settings/SettingsSponsorBlock.vue';
import { Settings } from '../../../tauri/types/config.ts';

const settings = defineModel<Settings>({ required: true });
const emit = defineEmits<{ reset: [] }>();
const { t } = useI18n();
</script>
