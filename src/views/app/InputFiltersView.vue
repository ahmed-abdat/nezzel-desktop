<template>
  <form @submit.prevent="save">
    <base-sub-nav>
      <template #default>
        <base-button type="button" :disabled="!canReset || isSaving" @click="reset">
          {{ t('settings.reset.label') }}
        </base-button>
        <base-button type="submit" class="btn-primary" :disabled="!hasChanges" :loading="isSaving">
          {{ t('common.save') }}
        </base-button>
      </template>
      <template #title>
        <h1>{{ t('prefs.adv.files.filters') }}</h1>
      </template>
    </base-sub-nav>

    <div class="mx-auto w-full max-w-[600px] px-6 pb-12 pt-5">
      <settings-section :title="t('inputFilters.dates.legend')" :note="t('inputFilters.dates.hint')">
        <settings-row for-id="input-filters-date-preset" :label="t('inputFilters.fields.date.presetsLabel')">
          <select-field id="input-filters-date-preset" v-model="presetModel" :options="presetOptions" />
        </settings-row>
        <settings-row for-id="input-filters-date-mode" :label="t('inputFilters.fields.date.modeLabel')">
          <select-field id="input-filters-date-mode" v-model="modeModel" :options="modeOptions" />
        </settings-row>
        <settings-row v-if="dateFilter.mode !== null" for-id="input-filters-date" :label="activeDateLabel">
          <date-picker id="input-filters-date" v-model="dateFilter.value" />
        </settings-row>
      </settings-section>

      <settings-section :title="t('inputFilters.filesize.legend')" :note="t('inputFilters.filesize.hint')">
        <settings-row
          v-for="field in sizeFields"
          :key="field.id"
          :for-id="field.id"
          :label="field.label"
        >
          <Input
            :id="field.id"
            v-model.number="field.model.value"
            type="number"
            min="0"
            step="0.1"
            dir="ltr"
            class="w-24 text-end tabular-nums"
            :placeholder="t('inputFilters.fields.size.valuePlaceholder')" />
          <select-field
            v-model="field.model.unit"
            :options="unitOptions"
            :aria-label="t('inputFilters.fields.size.unitPlaceholder')"
            class="min-w-20"
          />
        </settings-row>
      </settings-section>

      <base-disclosure class="mt-8" :label="t('perDownload.advanced')">
        <p class="pb-1 text-nz-caption text-muted-foreground">{{ t('inputFilters.advanced.hint') }}</p>
        <settings-row stacked for-id="input-filters-match-filters" :label="t('inputFilters.fields.matchFilters.label')" :help="t('inputFilters.fields.matchFilters.hint')">
          <Textarea
            id="input-filters-match-filters"
            v-model="draft.matchFilters"
            dir="ltr"
            rows="3"
            spellcheck="false"
            class="py-1.5 font-mono text-[12px]"
            :placeholder="t('inputFilters.fields.matchFilters.placeholder')" />
        </settings-row>
        <settings-row stacked for-id="input-filters-break-match-filters" :label="t('inputFilters.fields.breakMatchFilters.label')" :help="t('inputFilters.fields.breakMatchFilters.hint')">
          <Textarea
            id="input-filters-break-match-filters"
            v-model="draft.breakMatchFilters"
            dir="ltr"
            rows="3"
            spellcheck="false"
            class="py-1.5 font-mono text-[12px]"
            :placeholder="t('inputFilters.fields.breakMatchFilters.placeholder')" />
        </settings-row>
      </base-disclosure>
    </div>
  </form>
</template>

<script setup lang="ts">
import { Input } from '../../components/ui/input';
import { Textarea } from '../../components/ui/textarea';
import { computed, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import BaseButton from '../../components/base/BaseButton.vue';
import BaseSubNav from '../../components/base/BaseSubNav.vue';
import { DatePicker } from '../../components/ui/date-picker';
import { SelectField } from '../../components/ui/select';
import SettingsRow from '../../components/settings/SettingsRow.vue';
import SettingsSection from '../../components/settings/SettingsSection.vue';
import BaseDisclosure from '../../components/base/BaseDisclosure.vue';
import {
  applyDatePreset,
  cloneInputFilterSettings,
  createDefaultInputFilterSettings,
  normalizeInputFilterSettings,
  INPUT_FILTER_DATE_PRESETS,
  INPUT_FILTER_SIZE_UNITS,
  type InputFilterDatePreset,
} from '../../helpers/inputFilters.ts';
import { useSettingsStore } from '../../stores/settings.ts';
import { useToastStore } from '../../stores/toast.ts';
import type { InputFilterSettings } from '../../tauri/types/config.ts';

const { t } = useI18n();
const settingsStore = useSettingsStore();
const toastStore = useToastStore();

const draft = ref<InputFilterSettings>(
  cloneInputFilterSettings(settingsStore.settings.inputFilters),
);
const isSaving = ref(false);

const sizeUnits = INPUT_FILTER_SIZE_UNITS;
const dateModeOptions = computed(() => [
  { value: 'exact' as const, label: t('inputFilters.fields.date.mode.exact') },
  { value: 'before' as const, label: t('inputFilters.fields.date.mode.before') },
  { value: 'after' as const, label: t('inputFilters.fields.date.mode.after') },
]);
const datePresets = computed(() => INPUT_FILTER_DATE_PRESETS.map(value => ({
  value,
  label: t(`inputFilters.fields.date.presets.${value}`),
})));
const selectedDatePreset = computed<InputFilterDatePreset | null>({
  get: () => {
    const current = JSON.stringify(normalizeInputFilterSettings({
      ...createDefaultInputFilterSettings(),
      dateFilter: dateFilter.value,
    }).dateFilter);

    for (const preset of INPUT_FILTER_DATE_PRESETS) {
      const presetValue = JSON.stringify(applyDatePreset(preset));
      if (presetValue === current) {
        return preset;
      }
    }

    return null;
  },
  set: (value) => {
    if (value === null) {
      clearDateFilter();
      return;
    }
    selectDatePreset(value);
  },
});

const dateFilter = computed({
  get: () => draft.value.dateFilter,
  set: (value) => {
    draft.value.dateFilter = value;
  },
});
const minSize = computed({
  get: () => draft.value.minSize,
  set: (value) => {
    draft.value.minSize = value;
  },
});
const maxSize = computed({
  get: () => draft.value.maxSize,
  set: (value) => {
    draft.value.maxSize = value;
  },
});

/** Selects can't hold null: "" stands for "Off" / "no preset". */
const presetOptions = computed(() => [
  { value: '', label: t('inputFilters.fields.date.clearMode') },
  ...datePresets.value,
]);
const presetModel = computed<string>({
  get: () => selectedDatePreset.value ?? '',
  set: (value) => {
    selectedDatePreset.value = value === '' ? null : value as InputFilterDatePreset;
  },
});
const modeOptions = computed(() => [
  { value: '', label: t('inputFilters.fields.date.clearMode') },
  ...dateModeOptions.value,
]);
const modeModel = computed<string>({
  get: () => dateFilter.value.mode ?? '',
  set: (value) => {
    if (value === '') clearDateFilter();
    else dateFilter.value.mode = value as 'exact' | 'before' | 'after';
  },
});
const unitOptions = sizeUnits.map(unit => ({ value: unit, label: unit }));
const sizeFields = computed(() => [
  { id: 'input-filters-min-size', label: t('inputFilters.fields.minSize.label'), model: minSize.value },
  { id: 'input-filters-max-size', label: t('inputFilters.fields.maxSize.label'), model: maxSize.value },
]);

const normalizedDraft = computed(() => normalizeInputFilterSettings(draft.value));
const normalizedStore = computed(() => normalizeInputFilterSettings(settingsStore.settings.inputFilters));
const normalizedDefaults = computed(() => normalizeInputFilterSettings(createDefaultInputFilterSettings()));
const hasChanges = computed(() => JSON.stringify(normalizedDraft.value) !== JSON.stringify(normalizedStore.value));
const canReset = computed(() => JSON.stringify(normalizedDraft.value) !== JSON.stringify(normalizedDefaults.value));
const activeDateLabel = computed(() => {
  switch (dateFilter.value.mode) {
    case 'before':
      return t('inputFilters.fields.date.activeLabel.before');
    case 'after':
      return t('inputFilters.fields.date.activeLabel.after');
    case 'exact':
    default:
      return t('inputFilters.fields.date.activeLabel.exact');
  }
});

watch(
  () => settingsStore.settings.inputFilters,
  (value) => {
    draft.value = cloneInputFilterSettings(value);
  },
  { deep: true },
);

function clearDateFilter() {
  dateFilter.value = {
    mode: null,
    value: null,
  };
}

function selectDatePreset(preset: InputFilterDatePreset) {
  dateFilter.value = applyDatePreset(preset);
}

async function save() {
  const normalized = normalizeInputFilterSettings(draft.value);

  isSaving.value = true;
  try {
    await settingsStore.patch({ inputFilters: normalized });
    draft.value = cloneInputFilterSettings(normalized);
    toastStore.showToast(t('inputFilters.toasts.saved'), { style: 'success' });
  } catch (error) {
    console.error(error);
    toastStore.showToast(t('inputFilters.toasts.error', { error: String(error) }), {
      style: 'error',
    });
  } finally {
    isSaving.value = false;
  }
}

function reset() {
  draft.value = createDefaultInputFilterSettings();
}
</script>
