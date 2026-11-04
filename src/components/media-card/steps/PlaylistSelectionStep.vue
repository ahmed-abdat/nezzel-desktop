<template>
  <!-- Inline in the download row (the row already shows the title): which items, then one action. -->
  <div class="flex min-w-0 flex-col gap-1.5">
    <div class="flex flex-wrap items-center gap-x-2 gap-y-1.5 text-nz-caption">
      <span class="basis-full text-muted-foreground">{{ t('inputFilters.playlistSelection.description') }}</span>
      <template v-if="!hasAdvancedSelection">
        <label for="playlist-selection-start">{{ t('inputFilters.playlistSelection.fields.start') }}</label>
        <Input
          id="playlist-selection-start"
          v-model.number="simpleStart"
          type="number"
          step="1"
          dir="ltr"
          :class="cn(fieldClass, 'h-7 w-16 text-center tabular-nums', displaySimpleValidationError && 'border-destructive')"
          :aria-invalid="!!displaySimpleValidationError || undefined"
          :placeholder="t('inputFilters.playlistSelection.fields.startPlaceholder')"
          @blur="simpleTouched = true" />
        <label for="playlist-selection-end">{{ t('inputFilters.playlistSelection.fields.end') }}</label>
        <Input
          id="playlist-selection-end"
          v-model.number="simpleEnd"
          type="number"
          step="1"
          dir="ltr"
          :class="cn(fieldClass, 'h-7 w-16 text-center tabular-nums', displaySimpleValidationError && 'border-destructive')"
          :aria-invalid="!!displaySimpleValidationError || undefined"
          :placeholder="endPlaceholder"
          @blur="simpleTouched = true" />
      </template>
      <span v-else class="inline-flex min-w-0 items-center gap-1.5">
        <span>{{ t('inputFilters.playlistSelection.fields.advancedLabel') }}</span>
        <code class="truncate rounded-[4px] bg-muted px-1.5 py-0.5 font-mono text-[11.5px]" dir="ltr">{{ advancedSpec }}</code>
      </span>
    </div>
    <div class="flex flex-wrap items-center gap-1.5">
      <span class="me-auto text-nz-caption text-muted-foreground tabular-nums">{{ selectionSummary }}</span>
      <Button variant="ghost" size="xs" @click="openAdvanced">{{ t('inputFilters.playlistSelection.advancedButton') }}</Button>
      <Button v-if="hasAdvancedSelection" variant="ghost" size="xs" :disabled="isApplying" @click="clearAdvanced">
        {{ t('inputFilters.playlistSelection.simpleButton') }}
      </Button>
      <Button
        size="xs"
        :disabled="isApplying || !!validationError || (hasSelection && selectedEntries.length === 0)"
        @click="applySelection"
      >
        <Loader2 v-if="isApplying" class="animate-spin" aria-hidden="true" />
        {{ hasSelection ? t('inputFilters.playlistSelection.applySelection') : t('inputFilters.playlistSelection.applyFull') }}
      </Button>
    </div>
    <playlist-selection-modal ref="advancedModal" @apply="saveAdvanced" />
  </div>
</template>

<script setup lang="ts">
import { Input } from '../../ui/input';
import { computed, PropType, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import PlaylistSelectionModal from './PlaylistSelectionModal.vue';
import { Loader2 } from '@lucide/vue';
import { Button } from '../../ui/button';
import { fieldClass } from '../../settings/fieldClass';
import { cn } from 'cn';
import { useMediaStore } from '../../../stores/media/media.ts';
import { useToastStore } from '../../../stores/toast.ts';
import type { Group } from '../../../tauri/types/group.ts';
import {
  applyPlaylistSelectionToEntries,
  buildPlaylistItemsSpec,
  type PlaylistSelection,
} from '../../../helpers/playlistSelection.ts';

const { t } = useI18n();
const mediaStore = useMediaStore();
const toastStore = useToastStore();

type PlaylistSelectionModalApi = {
  open: (selection: PlaylistSelection) => void;
};

type SimplePlaylistIndex = number | '' | null;

const { group } = defineProps({
  group: {
    type: Object as PropType<Group>,
    required: true,
  },
});

const simpleStart = ref<SimplePlaylistIndex>(null);
const simpleEnd = ref<SimplePlaylistIndex>(null);
const simpleTouched = ref(false);
const advancedSelection = ref<PlaylistSelection>({ rows: [] });
const advancedModal = ref<PlaylistSelectionModalApi | null>(null);
const isApplying = ref(false);

const endPlaceholder = computed(() => `${group.entries?.length ?? ''}`);
const hasAdvancedSelection = computed(() => advancedSelection.value.rows.length > 0);
const advancedSpec = computed(() => buildPlaylistItemsSpec(advancedSelection.value));
const normalizedSimpleStart = computed(() => normalizeSimplePlaylistIndex(simpleStart.value));
const normalizedSimpleEnd = computed(() => normalizeSimplePlaylistIndex(simpleEnd.value));
const simpleValidationError = computed<string | null>(() => {
  if (hasAdvancedSelection.value) {
    return null;
  }

  const isStartEmpty = normalizedSimpleStart.value == null;
  const isEndEmpty = normalizedSimpleEnd.value == null;
  if (isStartEmpty && isEndEmpty) {
    return null;
  }

  if (normalizedSimpleStart.value == null || normalizedSimpleEnd.value == null) {
    return 'playlistRangeBounds';
  }

  return null;
});
const displaySimpleValidationError = computed(() => simpleTouched.value && simpleValidationError.value);

const selection = computed<PlaylistSelection>(() => {
  if (hasAdvancedSelection.value) {
    return advancedSelection.value;
  }

  if (
    simpleValidationError.value
    || (normalizedSimpleStart.value == null && normalizedSimpleEnd.value == null)
  ) {
    return { rows: [] };
  }

  return {
    rows: [{
      id: 'playlist-selection-range',
      type: 'range',
      start: normalizedSimpleStart.value,
      end: normalizedSimpleEnd.value,
    }],
  };
});

const selectedEntries = computed(() => applyPlaylistSelectionToEntries(group.entries ?? [], selection.value));
const hasSelection = computed(() => selection.value.rows.length > 0);
const validationError = computed(() => hasAdvancedSelection.value ? null : simpleValidationError.value);
const selectionSummary = computed(() => {
  const total = group.entries?.length ?? 0;
  const count = hasSelection.value ? selectedEntries.value.length : total;
  return total === count
    ? t('inputFilters.playlistSelection.selectionSummaryAll', { count })
    : t('inputFilters.playlistSelection.selectionSummary', { count, total });
});

function openAdvanced() {
  advancedModal.value?.open(advancedSelection.value);
}

function saveAdvanced(value: PlaylistSelection) {
  advancedSelection.value = value;
}

function clearAdvanced() {
  advancedSelection.value = { rows: [] };
  simpleTouched.value = false;
}

async function applySelection() {
  simpleTouched.value = true;
  isApplying.value = true;
  try {
    await mediaStore.expandPlaylistGroup(group.id, selection.value);
  } catch (error) {
    console.error(error);
    toastStore.showToast(t('media.card.toasts.applyFailed'), { style: 'error' });
  } finally {
    isApplying.value = false;
  }
}

function isValidPlaylistIndex(value: number | null): value is number {
  return typeof value === 'number' && Number.isInteger(value) && value !== 0;
}

function normalizeSimplePlaylistIndex(value: SimplePlaylistIndex): number | null {
  return typeof value === 'number' && isValidPlaylistIndex(value) ? value : null;
}
</script>
