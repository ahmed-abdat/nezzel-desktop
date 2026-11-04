<template>
  <!-- Flat preference rows: how to pick the part, then the chapter or the start/stop times. -->
  <settings-row :for-id="`selection-mode-${selection.id}`" :label="t('perDownload.output.partBy')" :help="t('perDownload.output.partByHint')">
    <select-field
      :id="`selection-mode-${selection.id}`"
      :model-value="choice"
      :options="modeOptions"
      @update:model-value="setChoice"
    />
  </settings-row>

  <settings-row
    v-if="choice === 'chapter'"
    :for-id="`selection-chapter-${selection.id}`"
    :label="t('settings.output.partialDownload.chapter')"
  >
    <select-field
      :id="`selection-chapter-${selection.id}`"
      :model-value="selection.chapterKey"
      :options="chapterSelectionOptions"
      class="max-w-64"
      content-class="max-w-[min(28rem,90vw)]"
      @update:model-value="setChapter"
    />
  </settings-row>

  <template v-else-if="choice === 'time'">
    <settings-row
      v-for="field in timeFields"
      :key="field"
      :for-id="`selection-${field}-${selection.id}`"
      :label="t(field === 'start' ? 'settings.output.partialDownload.startAt' : 'settings.output.partialDownload.stopAt')"
    >
      <div class="flex flex-col items-end gap-1">
        <Input
          :id="`selection-${field}-${selection.id}`"
          v-model="selection[field]"
          type="text"
          inputmode="numeric"
          dir="ltr"
          spellcheck="false"
          autocomplete="off"
          placeholder="00:00:00"
          :class="['w-28 text-center font-mono text-[12px] tabular-nums', selectionError[field] && 'border-destructive']"
          :aria-invalid="!!selectionError[field] || undefined"
          :aria-describedby="selectionError[field] ? `selection-${field}-${selection.id}-error` : undefined"
          @blur="normalizeField(field)" />
        <p v-if="selectionError[field]" :id="`selection-${field}-${selection.id}-error`" class="text-nz-caption text-destructive">
          {{ t(`settings.output.partialDownload.errors.${selectionError[field]}`) }}
        </p>
      </div>
    </settings-row>
  </template>

  <p v-if="choice === 'time' && hasInvalidSelection" role="status" class="flex items-center gap-1.5 py-2 text-nz-caption text-amber-foreground">
    <TriangleAlert class="size-3.5 shrink-0" aria-hidden="true" />
    {{ t('settings.output.partialDownload.invalidHint') }}
  </p>
</template>

<script setup lang="ts">
import { Input } from '../ui/input';
import { computed, PropType, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { TriangleAlert } from '@lucide/vue';
import { SelectField } from '../ui/select';
import SettingsRow from '../settings/SettingsRow.vue';
import {
  createDefaultSection,
  formatSecondsAsClock,
  isDefaultTimeRange,
  normalizeTimestampInput,
  type PartialDownloadSelection,
  validateSelection,
} from '../../helpers/partialDownload.ts';
import { Chapter } from '../../tauri/types/media.ts';

interface ChapterOption {
  value: string;
  label: string;
  chapter?: Chapter;
}

const ALL_CHAPTERS_VALUE = '__all__';

const selection = defineModel<PartialDownloadSelection>({ required: true });

const props = defineProps({
  chapters: {
    type: Array as PropType<Chapter[]>,
    required: true,
  },
  durationSeconds: {
    type: Number,
    required: false,
    default: undefined,
  },
});

const { t } = useI18n();

const chapterOptions = computed<ChapterOption[]>(() =>
  props.chapters.map((chapter, index) => ({
    value: `${index}:${chapter.startTime}:${chapter.endTime}`,
    chapter,
    label: `${chapter.title} (${formatSecondsAsClock(Math.floor(chapter.startTime))} - ${formatSecondsAsClock(Math.floor(chapter.endTime))})`,
  })),
);
const chapterSelectionOptions = computed<ChapterOption[]>(() => (
  chapterOptions.value.length > 0
    ? [{ value: ALL_CHAPTERS_VALUE, label: t('settings.output.partialDownload.allChapters') }, ...chapterOptions.value]
    : []
));
const chapterOptionMap = computed(() =>
  new Map(chapterSelectionOptions.value.map(option => [option.value, option])),
);
const hasChapterOptions = computed(() => chapterOptions.value.length > 0);
const timeFields = ['start', 'end'] as const;
const modeOptions = computed(() => [
  { value: 'whole', label: t('perDownload.output.wholeVideo') },
  { value: 'time', label: t('perDownload.output.byTime') },
  { value: 'chapter', label: t('perDownload.output.byChapter'), disabled: !hasChapterOptions.value },
]);
const selectionError = computed(() => validateSelection(selection.value, props.durationSeconds));
const hasInvalidSelection = computed(() =>
  selectionError.value.start !== null || selectionError.value.end !== null,
);

function resetRange() {
  const defaults = createDefaultSection(selection.value.id, props.durationSeconds);
  selection.value.start = defaults.start;
  selection.value.end = defaults.end;
}

function normalizeField(field: 'start' | 'end') {
  if (selection.value.mode === 'chapter') return;
  const normalized = normalizeTimestampInput(selection.value[field]);
  if (normalized) {
    selection.value[field] = normalized;
  }
}

function applyChapterSelection() {
  const selected = chapterOptionMap.value.get(selection.value.chapterKey);
  if (!selected?.chapter) {
    resetRange();
    return;
  }

  selection.value.start = formatSecondsAsClock(Math.floor(selected.chapter.startTime));
  selection.value.end = formatSecondsAsClock(Math.floor(selected.chapter.endTime));
}

type Choice = 'whole' | 'time' | 'chapter';
/** "Whole video" is the untouched default: time mode with the full range, or all chapters. */
function choiceFromSelection(): Choice {
  if (selection.value.mode === 'chapter' && hasChapterOptions.value) {
    return selection.value.chapterKey === ALL_CHAPTERS_VALUE ? 'whole' : 'chapter';
  }
  return isDefaultTimeRange(selection.value, props.durationSeconds) ? 'whole' : 'time';
}
const choice = ref<Choice>(choiceFromSelection());
watch(() => selection.value.id, () => {
  choice.value = choiceFromSelection();
});

function setChoice(next: string | null | undefined) {
  if (next === 'whole') {
    selection.value.mode = 'time';
    selection.value.chapterKey = '';
    resetRange();
    choice.value = 'whole';
    return;
  }
  selection.value.mode = next === 'chapter' ? 'chapter' : 'time';
  choice.value = selection.value.mode;
  onModeChange();
}

function setChapter(key: string | null | undefined) {
  selection.value.chapterKey = key ?? '';
  applyChapterSelection();
}

function onModeChange() {
  if (selection.value.mode !== 'chapter') return;
  if (!selection.value.chapterKey) {
    selection.value.chapterKey = ALL_CHAPTERS_VALUE;
  }
  applyChapterSelection();
}
</script>
