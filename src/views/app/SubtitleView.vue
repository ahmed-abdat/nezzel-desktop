<template>
  <form @submit.prevent="save">
    <base-sub-nav>
      <template #default>
        <base-button type="submit" class="btn-primary" :disabled="!hasChanges" :loading="isSaving">
          {{ t('common.save') }}
        </base-button>
      </template>
      <template #title>
        <h1>{{ t('subtitles.title') }}</h1>
      </template>
    </base-sub-nav>

    <div class="mx-auto w-full max-w-[600px] px-6 pb-12 pt-5">
      <settings-section :title="t('perDownload.subtitles.languages')" :note="t('subtitles.defaults.hint')">
        <settings-row for-id="all-languages" :label="t('perDownload.subtitles.all')" :help="selectionSummary">
          <Switch id="all-languages" :model-value="isAllSelected" @update:model-value="toggleAllLanguages" />
        </settings-row>
        <div class="py-3">
          <label for="languageSearch" class="sr-only">{{ t('subtitles.options.languages.search') }}</label>
          <div class="relative">
            <Search class="pointer-events-none absolute start-2.5 top-1/2 size-3.5 -translate-y-1/2 text-muted-foreground" aria-hidden="true" />
            <Input
              id="languageSearch"
              v-model="languageQuery"
              type="search"
              autocomplete="off"
              class="ps-8"
              :placeholder="t('subtitles.options.languages.searchPlaceholder')" />
          </div>
          <ul
            class="mt-2 max-h-72 overflow-y-auto rounded-[6px] border border-input bg-card py-1"
            :aria-label="t('perDownload.subtitles.languages')"
            :class="isAllSelected && 'opacity-60'"
          >
            <li v-for="option in filteredLanguageOptions" :key="option.code">
              <label class="flex min-h-8 cursor-pointer items-center gap-2.5 px-3 text-nz-meta hover:bg-accent/60" :class="isAllSelected && 'cursor-default hover:bg-transparent'">
                <Checkbox
                  :id="`language-${option.code}`"
                  :model-value="isLanguageSelected(option.code)"
                  :disabled="isAllSelected"
                  @update:model-value="toggleLanguage(option.code)"
                />
                <span class="min-w-0 truncate">{{ option.englishName }}</span>
                <span
                  v-if="option.nativeName && option.nativeName !== option.englishName"
                  class="min-w-0 truncate text-nz-caption text-muted-foreground"
                  dir="auto"
                >{{ option.nativeName }}</span>
              </label>
            </li>
            <li v-if="!filteredLanguageOptions.length" class="px-3 py-6 text-center text-nz-caption text-muted-foreground">
              {{ t('subtitles.options.languages.noResult', { query: languageQuery }) }}
            </li>
          </ul>
        </div>
      </settings-section>

      <settings-section :title="t('perDownload.subtitles.file')">
        <settings-row for-id="embed-subtitles" :label="t('perDownload.subtitles.embed')" :help="t('perDownload.subtitles.embedHint')">
          <Switch id="embed-subtitles" v-model="subtitleSettings.embedSubtitles" />
        </settings-row>
        <settings-row for-id="format-select" :label="t('perDownload.subtitles.format')" :help="t('perDownload.subtitles.formatHint')">
          <select-field id="format-select" v-model="primaryFormat" :options="subtitleFormatOptions" />
        </settings-row>
      </settings-section>
    </div>
  </form>
</template>

<script setup lang="ts">
import { Input } from '../../components/ui/input';
import { computed, ref, watch } from 'vue';
import BaseButton from '../../components/base/BaseButton.vue';
import BaseSubNav from '../../components/base/BaseSubNav.vue';
import { Search } from '@lucide/vue';
import { Checkbox } from '../../components/ui/checkbox';
import { SelectField } from '../../components/ui/select';
import Switch from '../../components/ui/switch/Switch.vue';
import SettingsRow from '../../components/settings/SettingsRow.vue';
import SettingsSection from '../../components/settings/SettingsSection.vue';
import {
  languageOptions,
  getSubtitleLanguageLabel,
  type SubtitleLanguageOption,
} from '../../helpers/subtitles/languages.ts';
import {
  areSubtitleSettingsEqual,
  buildSubtitleFormatOptions,
  createSubtitleSettings,
  getPrimarySubtitleFormat,
} from '../../helpers/subtitles/settings.ts';
import { sanitizeSubtitleFormats, sanitizeSubtitleLanguages } from '../../helpers/subtitles/sanitize.ts';
import { useSettingsStore } from '../../stores/settings.ts';
import { useToastStore } from '../../stores/toast.ts';
import { useI18n } from 'vue-i18n';
import type { SubtitleSettings } from '../../tauri/types/config.ts';

const { t } = useI18n();
const settingsStore = useSettingsStore();
const toastStore = useToastStore();

const subtitleFormatOptions = computed(() => buildSubtitleFormatOptions(t));

const subtitleSettings = ref<SubtitleSettings>(
  createSubtitleSettings(settingsStore.settings.subtitles),
);
const languageQuery = ref('');
const isSaving = ref(false);

const hasChanges = computed(() => {
  return !areSubtitleSettingsEqual(subtitleSettings.value, settingsStore.settings.subtitles);
});

const languageSelection = computed({
  get: () => subtitleSettings.value.languages,
  set: (value: string[]) => {
    subtitleSettings.value.languages = sanitizeSubtitleLanguages(value);
  },
});

const orderedFormats = computed(() =>
  sanitizeSubtitleFormats(subtitleSettings.value.formatPreference),
);

const primaryFormat = computed({
  get: () => orderedFormats.value[0] ?? getPrimarySubtitleFormat(subtitleSettings.value),
  set: (value: string) => {
    subtitleSettings.value.formatPreference = sanitizeSubtitleFormats([value]);
  },
});

const isAllSelected = computed(() => languageSelection.value.includes('all'));

function toggleAllLanguages() {
  languageSelection.value = isAllSelected.value ? [] : ['all'];
}

function toggleLanguage(code: string) {
  const next = new Set(languageSelection.value);
  if (next.has(code)) {
    next.delete(code);
  } else {
    next.add(code);
  }
  next.delete('all');
  languageSelection.value = Array.from(next);
}

function isLanguageSelected(code: string): boolean {
  return languageSelection.value.includes(code);
}

function matchesQuery(option: SubtitleLanguageOption): boolean {
  const query = languageQuery.value.trim().toLowerCase();
  if (!query) {
    return true;
  }

  return (
    option.englishName.toLowerCase().includes(query)
    || option.nativeName.toLowerCase().includes(query)
    || option.code.toLowerCase().includes(query)
  );
}

const filteredLanguageOptions = computed(() =>
  languageOptions.filter(option => matchesQuery(option) || isLanguageSelected(option.code)),
);

const selectionSummary = computed(() => {
  if (isAllSelected.value) return t('perDownload.subtitles.allHint');
  return t('perDownload.subtitles.selected', selectedLanguageBadges.value.length);
});

const selectedLanguageBadges = computed(() => {
  if (isAllSelected.value) {
    return [];
  }

  return languageSelection.value
    .map(code => getSubtitleLanguageLabel(code))
    .filter((label, index, array) => array.indexOf(label) === index);
});

watch(
  () => settingsStore.settings.subtitles,
  (value) => {
    subtitleSettings.value = createSubtitleSettings(value);
  },
  { deep: true },
);

async function save() {
  const normalized = createSubtitleSettings(subtitleSettings.value);

  isSaving.value = true;
  try {
    await settingsStore.patch({ subtitles: normalized });
    subtitleSettings.value = normalized;
    toastStore.showToast(t('subtitles.toasts.saved'), { style: 'success' });
  } catch (error) {
    console.error(error);
    toastStore.showToast(t('subtitles.toasts.error', { error: String(error) }), {
      style: 'error',
    });
  } finally {
    isSaving.value = false;
  }
}
</script>
