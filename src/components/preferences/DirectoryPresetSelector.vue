<template>
  <!-- Preset as a row (example path as help); the raw pattern only appears for "Custom". -->
  <settings-row :for-id="`${idPrefix}-preset`" :label="presetLabel" :help="exampleHelp">
    <select-field :id="`${idPrefix}-preset`" v-model="internalPreset" :options="presetOptions" />
  </settings-row>
  <settings-row v-if="isCustom" stacked :for-id="`${idPrefix}-format`" :label="formatLabel">
    <Input
      :id="`${idPrefix}-format`"
      v-model="internalTemplate"
      type="text"
      dir="ltr"
      spellcheck="false"
      autocomplete="off"
      class="font-mono text-[12px]" />
  </settings-row>
</template>

<script setup lang="ts">
import { Input } from '../ui/input';
import { computed } from 'vue';
import { useI18n } from 'vue-i18n';
import { SelectField } from '../ui/select';
import SettingsRow from '../settings/SettingsRow.vue';
import { DirectoryPreset } from '../../tauri/types/config.ts';
import { usePreferencesStore } from '../../stores/preferences';
import { TrackType } from '../../tauri/types/media';

interface PresetDef {
  label: string;
  value: DirectoryPreset;
  format: string;
  example?: string;
}

const preferencesStore = usePreferencesStore();
const { t } = useI18n();

const props = defineProps<{
  idPrefix: string;
  presets: PresetDef[];
  modelValue: string;
  preset: DirectoryPreset;
  trackType: TrackType;
  presetLabel: string;
  formatLabel: string;
  exampleLabel?: string;
}>();

const emit = defineEmits<{
  'update:modelValue': [string];
  'update:preset': [DirectoryPreset];
}>();

const isCustom = computed(() => props.preset === DirectoryPreset.Custom);

const internalPreset = computed<DirectoryPreset>({
  get: () => props.preset,
  set(val) {
    emit('update:preset', val);

    if (val !== DirectoryPreset.Custom) {
      const preset = props.presets.find(p => p.value === val);
      if (preset) {
        emit('update:modelValue', preset.format);
      }
    }
  },
});

const internalTemplate = computed<string>({
  get() {
    if (!isCustom.value) {
      return (
        props.presets.find(p => p.value === props.preset)?.format
        ?? props.modelValue
        ?? props.presets[0]?.format
        ?? ''
      );
    }
    return props.modelValue ?? props.presets[0]?.format ?? '';
  },
  set(val: string) {
    emit('update:modelValue', val);
  },
});

const example = computed<string | undefined>(() => {
  if (isCustom.value) return undefined;
  return (
    props.presets.find(p => p.value === props.preset)?.example
    ?? props.modelValue
    ?? props.presets[0]?.example
  );
});

const displayExample = computed<string | undefined>(() => {
  const base = example.value;
  if (!base) {
    preferencesStore.setPathExample(props.trackType, '');
    return undefined;
  }
  preferencesStore.setPathExample(props.trackType, base);
  return base;
});

const fullExample = computed<string>(() => {
  return preferencesStore.getPathExample(props.trackType);
});

const presetOptions = computed(() => props.presets.map(p => ({ value: p.value, label: p.label })));
/** Reading displayExample keeps the store's combined folder + file name example current. */
const exampleHelp = computed(() => (displayExample.value ? t('folders.example', { path: fullExample.value }) : undefined));
</script>
