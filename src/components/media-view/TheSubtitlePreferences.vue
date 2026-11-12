<template>
  <div>
    <p v-if="!hasAnyAvailableLanguages" class="border-b border-hairline py-3 text-nz-meta text-muted-foreground">
      {{ t('perDownload.subtitles.empty') }}
    </p>

    <settings-section v-else :title="t('perDownload.subtitles.languages')">
      <settings-row for-id="override-all-subtitles" :label="t('perDownload.subtitles.all')" :help="selectionSummary">
        <Switch id="override-all-subtitles" :model-value="isAllSelected" @update:model-value="toggleAllLanguages" />
      </settings-row>

      <div v-for="group in languageGroups" :key="group.source" class="border-b border-hairline py-3 last:border-b-0">
        <h3 class="mb-2 text-nz-caption font-medium text-muted-foreground">{{ group.title }}</h3>
        <ul v-if="group.languages.length" class="grid grid-cols-[repeat(auto-fill,minmax(10rem,1fr))] gap-x-4 gap-y-1.5">
          <li v-for="language in group.languages" :key="`${group.source}-${language}`">
            <label class="flex min-h-6 cursor-pointer items-center gap-2 text-nz-meta" :class="isAllSelected && 'cursor-default text-muted-foreground'">
              <Checkbox
                :id="`${group.source}-language-${language}`"
                :model-value="isLanguageSelected(group.source, language)"
                :disabled="isAllSelected"
                @update:model-value="toggleLanguage(group.source, language)"
              />
              <span class="min-w-0 truncate">{{ languageLabel(language) }}</span>
            </label>
          </li>
        </ul>
        <p v-else class="text-nz-caption text-muted-foreground">{{ group.empty }}</p>
      </div>
    </settings-section>

    <settings-section :title="t('perDownload.subtitles.file')">
      <settings-row for-id="override-embed-subtitles" :label="t('perDownload.subtitles.embed')" :help="t('perDownload.subtitles.embedHint')">
        <Switch id="override-embed-subtitles" v-model="subtitleState.embedSubtitles" />
      </settings-row>
      <settings-row for-id="override-format-select" :label="t('perDownload.subtitles.format')" :help="t('perDownload.subtitles.formatHint')">
        <select-field id="override-format-select" v-model="primaryFormat" :options="subtitleFormatOptions" />
      </settings-row>
    </settings-section>
  </div>
</template>

<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { Checkbox } from '../ui/checkbox';
import { SelectField } from '../ui/select';
import Switch from '../ui/switch/Switch.vue';
import SettingsRow from '../settings/SettingsRow.vue';
import SettingsSection from '../settings/SettingsSection.vue';
import {
  getPreferredAutoSubtitleLanguages,
  getSubtitleLanguageLabel,
} from '../../helpers/subtitles/languages.ts';
import {
  areSubtitleSettingsEqual,
  buildSubtitleFormatOptions,
  createSubtitleSettings,
  getPrimarySubtitleFormat,
  mergeSubtitleSettings,
} from '../../helpers/subtitles/settings.ts';
import { sanitizeSubtitleFormats, sanitizeSubtitleLanguages } from '../../helpers/subtitles/sanitize.ts';
import { useMediaGroupStore } from '../../stores/media/group.ts';
import { useMediaOptionsStore } from '../../stores/media/options.ts';
import { useSettingsStore } from '../../stores/settings.ts';
import type { SubtitleSettings } from '../../tauri/types/config.ts';
import type { DownloadOverrides, SubtitleOverrides } from '../../tauri/types/media.ts';

const props = defineProps({
  groupId: {
    type: String,
    required: true,
  },
});

const { t } = useI18n();
const groupStore = useMediaGroupStore();
const optionsStore = useMediaOptionsStore();
const settingsStore = useSettingsStore();

const subtitleFormatOptions = computed(() => buildSubtitleFormatOptions(t));
const subtitleState = ref<SubtitleSettings>(createSubtitleSettings(settingsStore.settings.subtitles));
const manualLanguageSelection = ref<string[]>([]);
const autoLanguageSelection = ref<string[]>([]);
const allLanguagesSelected = ref(false);

const availableInventory = computed(() => {
  const group = groupStore.findGroupById(props.groupId);
  const manual = new Set<string>();
  const auto = new Set<string>();

  for (const item of Object.values(group?.items ?? {})) {
    item.subtitleInventory?.manualLanguages?.forEach(language => manual.add(language));
    item.subtitleInventory?.autoLanguages?.forEach(language => auto.add(language));
  }

  return {
    manual: Array.from(manual).sort(),
    auto: getPreferredAutoSubtitleLanguages(Array.from(auto)),
  };
});

const availableManualLanguages = computed(() => availableInventory.value.manual);
const availableAutoLanguages = computed(() => availableInventory.value.auto);
const hasAnyAvailableLanguages = computed(() =>
  availableManualLanguages.value.length > 0 || availableAutoLanguages.value.length > 0,
);

const orderedFormats = computed(() =>
  sanitizeSubtitleFormats(subtitleState.value.formatPreference),
);

const primaryFormat = computed({
  get: () => orderedFormats.value[0] ?? getPrimarySubtitleFormat(subtitleState.value),
  set: (value: string) => {
    subtitleState.value.formatPreference = sanitizeSubtitleFormats([value]);
  },
});

const isAllSelected = computed(() => allLanguagesSelected.value && hasAnyAvailableLanguages.value);

const languageGroups = computed(() => [
  { source: 'manual' as const, title: t('perDownload.subtitles.creator'), empty: t('perDownload.subtitles.noneCreator'), languages: availableManualLanguages.value },
  { source: 'auto' as const, title: t('perDownload.subtitles.auto'), empty: t('perDownload.subtitles.noneAuto'), languages: availableAutoLanguages.value },
]);

const selectionSummary = computed(() => {
  if (isAllSelected.value) return t('perDownload.subtitles.allHint');
  return t('perDownload.subtitles.selected', selectedLanguageBadges.value.length);
});

const selectedLanguageBadges = computed(() => {
  if (isAllSelected.value) {
    return [];
  }

  return [
    ...manualLanguageSelection.value.map(language => languageSelectionLabel('manual', language)),
    ...autoLanguageSelection.value.map(language => languageSelectionLabel('auto', language)),
  ]
    .filter((value, index, values) => values.indexOf(value) === index);
});

const resolvedSubtitleState = computed(() => {
  const globalSettings = settingsStore.settings.subtitles;
  const override = optionsStore.getOverrides(props.groupId)?.subtitles;
  return mergeSubtitleSettings(globalSettings, override);
});

function toggleAllLanguages() {
  if (isAllSelected.value) {
    allLanguagesSelected.value = false;
    manualLanguageSelection.value = [];
    autoLanguageSelection.value = [];
    return;
  }

  allLanguagesSelected.value = true;
  manualLanguageSelection.value = [...availableManualLanguages.value];
  autoLanguageSelection.value = [...availableAutoLanguages.value];
}

function toggleLanguage(source: 'manual' | 'auto', language: string) {
  allLanguagesSelected.value = false;
  const selection = source === 'manual' ? manualLanguageSelection : autoLanguageSelection;
  const next = new Set(selection.value);
  if (next.has(language)) {
    next.delete(language);
  } else {
    next.add(language);
  }
  selection.value = Array.from(next);
}

function isLanguageSelected(source: 'manual' | 'auto', language: string): boolean {
  const selection = source === 'manual' ? manualLanguageSelection : autoLanguageSelection;
  return selection.value.includes(language);
}

function languageLabel(language: string): string {
  return getSubtitleLanguageLabel(language);
}

function languageSelectionLabel(source: 'manual' | 'auto', language: string): string {
  return `${languageLabel(language)} (${t(`media.preferences.subtitles.languages.${source}`)})`;
}

function filterAvailableSelections(selected: string[], available: string[]): string[] {
  const availableSet = new Set(available);
  return sanitizeSubtitleLanguages(selected).filter(language => availableSet.has(language));
}

function resolveSourceSelections(
  settings: SubtitleSettings,
  override: SubtitleOverrides | undefined,
): { manual: string[]; auto: string[]; all: boolean } {
  const requested = sanitizeSubtitleLanguages(settings.languages);
  const wantsAll = requested.includes('all');
  const manualOverrideWantsAll = override?.manualLanguages?.includes('all') ?? false;
  const autoOverrideWantsAll = override?.autoLanguages?.includes('all') ?? false;
  const all = manualOverrideWantsAll || autoOverrideWantsAll || (!override && wantsAll);

  const manual = override?.manualLanguages
    ? (manualOverrideWantsAll
        ? [...availableManualLanguages.value]
        : filterAvailableSelections(override.manualLanguages, availableManualLanguages.value))
    : (wantsAll
        ? [...availableManualLanguages.value]
        : filterAvailableSelections(requested, availableManualLanguages.value));

  const auto = override?.autoLanguages
    ? (autoOverrideWantsAll
        ? [...availableAutoLanguages.value]
        : filterAvailableSelections(override.autoLanguages, availableAutoLanguages.value))
    : (settings.includeAutoGenerated
        ? (wantsAll
            ? [...availableAutoLanguages.value]
            : filterAvailableSelections(requested, availableAutoLanguages.value))
        : []);

  return { manual, auto, all };
}

function arraysEqual(a: string[], b: string[]): boolean {
  return JSON.stringify([...a].sort()) === JSON.stringify([...b].sort());
}

watch(
  [resolvedSubtitleState, availableManualLanguages, availableAutoLanguages],
  ([value]) => {
    if (!areSubtitleSettingsEqual(subtitleState.value, value)) {
      subtitleState.value = value;
    }

    const override = optionsStore.getOverrides(props.groupId)?.subtitles;
    const nextSelections = resolveSourceSelections(value, override);
    allLanguagesSelected.value = nextSelections.all;
    if (!arraysEqual(manualLanguageSelection.value, nextSelections.manual)) {
      manualLanguageSelection.value = nextSelections.manual;
    }
    if (!arraysEqual(autoLanguageSelection.value, nextSelections.auto)) {
      autoLanguageSelection.value = nextSelections.auto;
    }
  },
  { deep: true, immediate: true },
);

watch(
  [subtitleState, manualLanguageSelection, autoLanguageSelection, allLanguagesSelected],
  () => {
    const currentLanguages = sanitizeSubtitleLanguages([
      ...manualLanguageSelection.value,
      ...autoLanguageSelection.value,
    ]);
    const current = createSubtitleSettings({
      ...subtitleState.value,
      enabled: currentLanguages.length > 0,
      includeAutoGenerated: autoLanguageSelection.value.length > 0,
      languages: currentLanguages,
    });
    const base = createSubtitleSettings(settingsStore.settings.subtitles);
    const baseSelections = resolveSourceSelections(base, undefined);
    const subtitleOverride: SubtitleOverrides = {};
    if (current.enabled !== base.enabled) subtitleOverride.enabled = current.enabled;
    if (current.includeAutoGenerated !== base.includeAutoGenerated) {
      subtitleOverride.includeAutoGenerated = current.includeAutoGenerated;
    }
    if (current.embedSubtitles !== base.embedSubtitles) {
      subtitleOverride.embedSubtitles = current.embedSubtitles;
    }
    if (JSON.stringify(current.languages) !== JSON.stringify(base.languages)) {
      subtitleOverride.languages = current.languages;
    }
    if (JSON.stringify(current.formatPreference) !== JSON.stringify(base.formatPreference)) {
      subtitleOverride.formatPreference = current.formatPreference;
    }
    if (allLanguagesSelected.value) {
      subtitleOverride.manualLanguages = ['all'];
      subtitleOverride.autoLanguages = ['all'];
    } else {
      if (!arraysEqual(manualLanguageSelection.value, baseSelections.manual)) {
        subtitleOverride.manualLanguages = [...manualLanguageSelection.value];
      }
      if (!arraysEqual(autoLanguageSelection.value, baseSelections.auto)) {
        subtitleOverride.autoLanguages = [...autoLanguageSelection.value];
      }
    }
    if (allLanguagesSelected.value) {
      subtitleOverride.languages = ['all'];
    }
    const existing = optionsStore.getOverrides(props.groupId);
    const next: DownloadOverrides = { ...(existing ?? {}) };

    if (Object.keys(subtitleOverride).length > 0) {
      next.subtitles = subtitleOverride;
    } else {
      delete next.subtitles;
    }

    if (Object.values(next).some(value => value !== undefined)) {
      optionsStore.setOverrides(props.groupId, next);
    } else {
      optionsStore.removeOverrides(props.groupId);
    }
  },
  { deep: true },
);
</script>
