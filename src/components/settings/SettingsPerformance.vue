<template>
  <settings-section :title="t('settings.performance.legend')">
    <settings-row for-id="maxConcurrency" :label="t('prefs.adv.perf.parallel')" :help="t('prefs.adv.perf.parallelHint')">
      <NumberField id="maxConcurrency" v-model="maxConcurrency" :min="1" :max="32" />
    </settings-row>
    <settings-row for-id="concurrentFragments" :label="t('prefs.adv.perf.pieces')" :help="t('prefs.adv.perf.piecesHint')">
      <NumberField id="concurrentFragments" v-model="concurrentFragments" :min="1" :max="16" />
    </settings-row>

    <template #more>
      <settings-row for-id="splitPlaylistThreshold" :label="t('prefs.adv.perf.split')" :help="t('prefs.adv.perf.splitHint')">
        <SelectField id="splitPlaylistThreshold" v-model="settings.performance.splitPlaylistThreshold" :options="splitOptions" />
      </settings-row>
      <settings-row for-id="autoLoadSize" :label="t('prefs.adv.perf.size')" :help="t('prefs.adv.perf.sizeHint')">
        <Switch id="autoLoadSize" v-model="settings.performance.autoLoadSize" />
      </settings-row>
      <settings-row for-id="aria2Enabled" :label="t('prefs.adv.perf.aria2')" :help="aria2Available ? t('prefs.adv.perf.aria2Hint') : t('settings.performance.aria2Missing')">
        <Switch id="aria2Enabled" v-model="settings.performance.aria2Enabled" :disabled="!aria2Available" />
      </settings-row>
    </template>
  </settings-section>
</template>

<script setup lang="ts">
import { computed, onMounted, ref } from 'vue';
import { engineAvailability } from '../../tauri/listeners/transport';
import { useI18n } from 'vue-i18n';
import { SelectField } from '@/components/ui/select';
import NumberField from '@/components/ui/number-field/NumberField.vue';
import Switch from '@/components/ui/switch/Switch.vue';
import SettingsRow from './SettingsRow.vue';
import SettingsSection from './SettingsSection.vue';
import { Settings } from '../../tauri/types/config.ts';

const { t } = useI18n();
const settings = defineModel<Settings>({ required: true });

const aria2Available = ref(false);
onMounted(async () => {
  try {
    aria2Available.value = (await engineAvailability()).aria2Available;
  } catch { /* Disabled until availability is known. */ }
});

const splitOptions = computed(() => [
  { value: 1, label: t('settings.performance.splitPlaylistThreshold.options.never') },
  ...[50, 100, 150, 200].map(number => ({ value: number, label: t('settings.performance.splitPlaylistThreshold.options.lessThan', { number }) })),
]);

// The stepper can emit NaN while the field is cleared; never write that into the settings.
const concurrentFragments = computed<number>({
  get: () => settings.value.performance.concurrentFragments,
  set: (value) => {
    if (Number.isFinite(value)) settings.value.performance.concurrentFragments = value;
  },
});

const maxConcurrency = computed<number>({
  get: () => settings.value.performance.maxConcurrency,
  set: (value) => {
    if (Number.isFinite(value)) settings.value.performance.maxConcurrency = value;
  },
});
</script>
