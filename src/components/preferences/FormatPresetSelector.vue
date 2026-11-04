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
import { FormatPreset } from '../../tauri/types/config.ts';
import { usePreferencesStore } from '../../stores/preferences';
import { TrackType } from '../../tauri/types/media';

interface PresetDef {
  label: string;
  value: FormatPreset;
  format: string;
  example?: string;
}

const preferencesStore = usePreferencesStore();
const { t } = useI18n();

const props = defineProps<{
  idPrefix: string;
  presets: PresetDef[];
  modelValue: string;
  preset: FormatPreset;
  trackType: TrackType;
  presetLabel: string;
  formatLabel: string;
  exampleLabel?: string;
  restrictFilenames: boolean;
}>();

const emit = defineEmits<{
  'update:modelValue': [string];
  'update:preset': [FormatPreset];
}>();

const isCustom = computed(() => props.preset === FormatPreset.Custom);

const internalPreset = computed<FormatPreset>({
  get: () => props.preset,
  set(val) {
    emit('update:preset', val);

    if (val !== FormatPreset.Custom) {
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
  let base = example.value;
  if (!base) {
    preferencesStore.setFilenameExample(props.trackType, '');
    return undefined;
  }
  if (props.restrictFilenames) {
    base = base.replace(/\s+/g, '_').replace(/&/g, '_and_');
  }
  preferencesStore.setFilenameExample(props.trackType, base);
  return base;
});

const fullExample = computed<string>(() => {
  return preferencesStore.getPathExample(props.trackType);
});

const presetOptions = computed(() => props.presets.map(p => ({ value: p.value, label: p.label })));
/** LRI/PDI keep the Latin path in order inside Arabic help text. Reading displayExample keeps the store's combined folder + file name example current. */
const exampleHelp = computed(() => (displayExample.value ? t('folders.example', { path: `\u2066${fullExample.value}\u2069` }) : undefined));
</script>
