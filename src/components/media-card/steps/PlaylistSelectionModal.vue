<template>
  <Dialog v-model:open="isOpen">
    <DialogContent :show-close-button="false" class="block w-[min(36rem,calc(100vw-2rem))] max-w-none sm:max-w-none">
      <DialogTitle as="h3">{{ t('inputFilters.playlistSelection.advancedTitle') }}</DialogTitle>
      <DialogDescription class="mb-4 mt-0.5">
        {{ t('inputFilters.playlistSelection.advancedHint') }}
      </DialogDescription>

      <div class="flex flex-col gap-3">
        <div class="flex flex-wrap gap-2">
          <base-button type="button" class="btn-sm btn-soft" @click="addRow('single')">
            {{ t('inputFilters.playlistSelection.addSingle') }}
          </base-button>
          <base-button type="button" class="btn-sm btn-soft" @click="addRow('range')">
            {{ t('inputFilters.playlistSelection.addRange') }}
          </base-button>
        </div>

        <div v-if="draft.rows.length === 0" class="border-y border-hairline py-3 text-nz-caption text-muted-foreground">
          {{ t('inputFilters.playlistSelection.advancedEmpty') }}
        </div>

        <div v-else class="border-y border-hairline">
          <div class="flex items-center justify-between gap-3 border-b border-hairline py-2 text-nz-caption text-muted-foreground">
            <span>{{ t('inputFilters.playlistSelection.advancedDescription') }}</span>
            <span>{{ draft.rows.length }}</span>
          </div>

          <div class="flex max-h-80 flex-col overflow-y-auto">
            <article
              v-for="row in draft.rows"
              :key="row.id"
              class="border-b border-hairline py-2 last:border-b-0"
            >
              <div class="flex flex-col gap-2">
                <div class="flex flex-wrap items-center gap-2">
                  <div class="inline-flex h-7 items-center gap-0.5 rounded-[7px] bg-foreground/[0.06] p-0.5" role="group">
                    <button
                      type="button"
                      class="h-6 rounded-[5px] px-3 text-[12.5px] font-medium outline-none focus-visible:ring-2 focus-visible:ring-ring/40"
                      :class="row.type === 'single' ? 'bg-card text-foreground shadow-xs dark:bg-foreground/15' : 'text-muted-foreground hover:text-foreground'"
                      :aria-pressed="row.type === 'single'"
                      @click="setRowType(row.id, 'single')"
                    >
                      {{ t('inputFilters.playlistSelection.row.single') }}
                    </button>
                    <button
                      type="button"
                      class="h-6 rounded-[5px] px-3 text-[12.5px] font-medium outline-none focus-visible:ring-2 focus-visible:ring-ring/40"
                      :class="row.type === 'range' ? 'bg-card text-foreground shadow-xs dark:bg-foreground/15' : 'text-muted-foreground hover:text-foreground'"
                      :aria-pressed="row.type === 'range'"
                      @click="setRowType(row.id, 'range')"
                    >
                      {{ t('inputFilters.playlistSelection.row.range') }}
                    </button>
                  </div>

                  <div v-if="row.type === 'single'" class="flex min-w-0 flex-1 items-center gap-2">
                    <label class="shrink-0 text-nz-caption" :for="`playlist-row-index-${row.id}`">
                      {{ t('inputFilters.playlistSelection.fields.index') }}
                    </label>
                    <Input
                      :id="`playlist-row-index-${row.id}`"
                      v-model.number="row.index"
                      type="number"
                      step="1"
                      dir="ltr" class="h-7 w-full rounded-[6px] border border-input bg-card px-2 text-nz-meta tabular-nums shadow-xs outline-none focus-visible:ring-2 focus-visible:ring-ring/30"
                      :class="{ 'border-destructive': rowErrors[row.id] }"
                      :placeholder="t('inputFilters.playlistSelection.fields.indexPlaceholder')" />
                  </div>

                  <div v-else class="flex min-w-0 flex-1 flex-wrap items-center gap-2">
                    <label class="shrink-0 text-nz-caption" :for="`playlist-row-start-${row.id}`">
                      {{ t('inputFilters.playlistSelection.fields.start') }}
                    </label>
                    <Input
                      :id="`playlist-row-start-${row.id}`"
                      v-model.number="row.start"
                      type="number"
                      step="1"
                      dir="ltr" class="h-7 w-20 rounded-[6px] border border-input bg-card px-2 text-nz-meta tabular-nums shadow-xs outline-none focus-visible:ring-2 focus-visible:ring-ring/30"
                      :class="{ 'border-destructive': rowErrors[row.id] }"
                      :placeholder="t('inputFilters.playlistSelection.fields.startPlaceholder')" />
                    <label class="shrink-0 text-nz-caption" :for="`playlist-row-end-${row.id}`">
                      {{ t('inputFilters.playlistSelection.fields.end') }}
                    </label>
                    <Input
                      :id="`playlist-row-end-${row.id}`"
                      v-model.number="row.end"
                      type="number"
                      step="1"
                      dir="ltr" class="h-7 w-20 rounded-[6px] border border-input bg-card px-2 text-nz-meta tabular-nums shadow-xs outline-none focus-visible:ring-2 focus-visible:ring-ring/30"
                      :class="{ 'border-destructive': rowErrors[row.id] }"
                      :placeholder="t('inputFilters.playlistSelection.fields.endPlaceholder')" />
                  </div>

                  <base-button type="button" class="btn-sm btn-soft ms-auto" @click="removeRow(row.id)">
                    {{ t('inputFilters.playlistSelection.remove') }}
                  </base-button>
                </div>

                <p v-if="rowErrors[row.id]" class="text-nz-caption text-destructive">
                  {{ t(`inputFilters.playlistSelection.errors.${rowErrors[row.id]}`) }}
                </p>
              </div>
            </article>
          </div>
        </div>
      </div>

      <div class="mt-4 flex justify-end gap-2">
        <base-button type="button" class="btn-sm btn-soft" @click="cancel">
          {{ t('common.back') }}
        </base-button>
        <base-button type="button" class="btn-sm btn-primary" :disabled="hasValidationErrors" @click="save">
          {{ t('inputFilters.playlistSelection.useAdvanced') }}
        </base-button>
      </div>
    </DialogContent>
  </Dialog>
</template>

<script setup lang="ts">
import { Dialog, DialogContent, DialogDescription, DialogTitle } from '../../ui/dialog';
import { Input } from '../../ui/input';
import { computed, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import BaseButton from '../../base/BaseButton.vue';
import {
  clonePlaylistSelection,
  createPlaylistSelectionRow,
  getPlaylistSelectionRowError,
  type PlaylistSelection,
  type PlaylistSelectionRow,
} from '../../../helpers/playlistSelection.ts';

const emit = defineEmits<{
  apply: [selection: PlaylistSelection];
}>();

const { t } = useI18n();

const isOpen = ref(false);
const draft = ref<PlaylistSelection>({ rows: [] });
const rowErrors = computed<Record<string, string | null>>(() => Object.fromEntries(
  draft.value.rows.map(row => [row.id, getPlaylistSelectionRowError(row)]),
));
const hasValidationErrors = computed(() => Object.values(rowErrors.value).some(Boolean));

function open(selection: PlaylistSelection) {
  draft.value = clonePlaylistSelection(selection);
  isOpen.value = true;
}

function cancel() {
  isOpen.value = false;
}

function save() {
  emit('apply', clonePlaylistSelection(draft.value));
  isOpen.value = false;
}

function addRow(type: PlaylistSelectionRow['type']) {
  draft.value.rows.push(createPlaylistSelectionRow(type));
}

function removeRow(id: string) {
  draft.value.rows = draft.value.rows.filter(row => row.id !== id);
}

function setRowType(id: string, type: PlaylistSelectionRow['type']) {
  draft.value.rows = draft.value.rows.map(row => (
    row.id === id
      ? (type === 'single' ? { id, type, index: null } : { id, type, start: null, end: null })
      : row
  ));
}

defineExpose({
  open,
});
</script>
