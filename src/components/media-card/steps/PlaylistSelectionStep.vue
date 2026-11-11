<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import { useVirtualList } from '@vueuse/core';
import { useI18n } from 'vue-i18n';
import { Film, Loader2 } from '@lucide/vue';
import { Button } from '../../ui/button';
import { Checkbox } from '../../ui/checkbox';
import { SelectField } from '../../ui/select';
import { useMediaStore } from '../../../stores/media/media';
import { useMediaOptionsStore } from '../../../stores/media/options';
import { useSettingsStore } from '../../../stores/settings';
import { useToastStore } from '../../../stores/toast';
import type { Group } from '../../../tauri/types/group';
import { SMALL_MAX_HEIGHT } from '@/generated/contract';
import { TrackType } from '../../../tauri/types/media';
import { optionsForCaptureChoice } from '../../../helpers/captureChoices';
import { remoteImagesAllowed } from '../../../helpers/remoteImages';
import { formatDuration } from '../../../helpers/units';
import { useLocaleNumber } from '../../../composables/useLocaleNumber';

const { group, idPrefix, showAction = true } = defineProps<{ group: Group; idPrefix?: string; showAction?: boolean }>();
const { t } = useI18n();
const { size: formatSize } = useLocaleNumber();
const mediaStore = useMediaStore();
const optionsStore = useMediaOptionsStore();
const settingsStore = useSettingsStore();
const toastStore = useToastStore();
const checkboxPrefix = computed(() => idPrefix ?? `playlist-selection-${group.id}`);
const entries = computed(() => group.entries ?? []);
const selected = computed(() => new Set(optionsStore.getPlaylistSelection(group)));
const failedImages = ref(new Set<number>());
const isApplying = ref(false);
// Equivalent rereads may replace every object while only refreshing titles/thumbnails.
const entriesSignature = computed(() => JSON.stringify([group.id, entries.value.map(entry => [entry.index, entry.videoUrl])]));
watch(entriesSignature, () => {
  failedImages.value = new Set();
}, { immediate: true });

const { list, containerProps, wrapperProps } = useVirtualList(entries, { itemHeight: 64, overscan: 8 });
const selectedSize = computed(() => {
  let total = 0;
  if (!selected.value.size) return undefined;
  for (const position of selected.value) {
    const bytes = entries.value[position]?.filesize;
    if (bytes == null) return undefined;
    total += bytes;
  }
  return formatSize(total);
});
const summary = computed(() => t('inputFilters.playlistSelection.checkedCount', {
  count: selected.value.size, total: entries.value.length,
}, selected.value.size));
const quality = computed({
  get() {
    const options = optionsStore.getOptions(group.id);
    if (options?.trackType === TrackType.audio) return options.abr ? `audio:${options.abr}` : 'audio';
    return options?.height ? `video:${options.height}:${options.fps ?? 0}` : 'best';
  },
  set(value: string) {
    if (value === quality.value) return;
    const choice = value.startsWith('video:')
      ? { kind: 'video' as const, height: Number(value.split(':')[1]) }
      : { kind: value === 'audio' ? 'audio' as const : 'best' as const };
    optionsStore.setOptions(group.id, optionsForCaptureChoice(choice, [], 'video'));
  },
});
const qualityOptions = computed(() => {
  const options = [
    { value: 'best', label: t('sheet.best') },
    ...[1080, SMALL_MAX_HEIGHT, 480].map(height => ({ value: `video:${height}:0`, label: t('capture.formats.height', { n: height }) })),
    { value: 'audio', label: t('capture.audio') },
  ];
  const stored = optionsStore.getOptions(group.id);
  if (stored?.height && !options.some(option => option.value === quality.value)) {
    const label = [t('capture.formats.height', { n: stored.height }), stored.fps ? t('capture.formats.fps', { n: stored.fps }) : ''].filter(Boolean).join(' · ');
    options.push({ value: quality.value, label });
  }
  if (stored?.trackType === TrackType.audio && stored.abr) options.push({ value: quality.value, label: t('capture.formats.kbps', { n: stored.abr }) });
  return options;
});
function toggle(position: number, checked: boolean | 'indeterminate') {
  const positions = new Set(selected.value);
  if (checked === true) positions.add(position);
  else positions.delete(position);
  optionsStore.setPlaylistSelection(group, [...positions]);
}
function onKeydown(event: KeyboardEvent) {
  if (isApplying.value) return;
  if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'a') {
    event.preventDefault();
    event.stopPropagation();
    optionsStore.setPlaylistSelection(group, entries.value.map((_, position) => position));
  } else if (event.key === ' ' && event.target === event.currentTarget) {
    event.preventDefault();
    const position = Number((event.currentTarget as HTMLElement).dataset.position);
    if (Number.isInteger(position)) toggle(position, !selected.value.has(position));
  }
}
/** Resolves true once the checked items are queued. */
async function applySelection(): Promise<boolean> {
  if (isApplying.value || selected.value.size === 0) return false;
  isApplying.value = true;
  try {
    await mediaStore.downloadPlaylistSelection(group.id, { rows: [], positions: [...selected.value].sort((a, b) => a - b) });
    return true;
  } catch (error) {
    console.error(error);
    toastStore.showToast(t('media.card.toasts.applyFailed'), { style: 'error' });
    return false;
  } finally {
    isApplying.value = false;
  }
}
defineExpose({ applySelection, isApplying });
</script>

<template>
  <div class="flex min-w-0 flex-col gap-2" @keydown="onKeydown">
    <p class="text-[13px] text-muted-foreground">{{ t('inputFilters.playlistSelection.description') }}</p>
    <div class="flex min-w-0 items-center gap-1">
      <Button variant="ghost" size="xs" class="shrink-0 px-1" :disabled="isApplying" @click="optionsStore.setPlaylistSelection(group, entries.map((_, position) => position))">{{ t('inputFilters.playlistSelection.selectAll') }}</Button>
      <Button variant="ghost" size="xs" class="shrink-0 px-1" :disabled="isApplying" @click="optionsStore.setPlaylistSelection(group, [])">{{ t('inputFilters.playlistSelection.selectNone') }}</Button>
      <span class="ms-auto min-w-0 text-end text-[12px] text-muted-foreground tabular-nums" role="status">{{ summary }}</span>
    </div>
    <i18n-t v-if="selectedSize" keypath="sheet.size" tag="p" scope="global" data-testid="playlist-size" class="text-end text-[12px] text-muted-foreground"><template #size><bdi dir="ltr">{{ selectedSize }}</bdi></template></i18n-t>
    <div
      v-bind="containerProps"
      class="overflow-y-auto overscroll-contain border-y border-hairline"
      :style="{ height: `${Math.min(entries.length * 64, 320)}px` }"
      tabindex="0"
      role="region"
      :aria-label="t('inputFilters.playlistSelection.title')"
    >
      <div v-bind="wrapperProps" role="list">
        <div v-for="{ data: entry, index: position } in list" :key="position" role="listitem" :aria-setsize="entries.length" :aria-posinset="position + 1" :data-position="position" tabindex="0" class="flex h-16 min-w-0 items-center gap-2 border-b border-hairline px-1 hover:bg-accent/70 focus-visible:outline-solid focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-[var(--nz-focus)]" @keydown="onKeydown">
          <Checkbox :id="`${checkboxPrefix}-item-${position}`" :model-value="selected.has(position)" :disabled="isApplying" @update:model-value="toggle(position, $event)" />
          <label :for="`${checkboxPrefix}-item-${position}`" class="flex min-w-0 flex-1 cursor-pointer items-center gap-2 text-start">
            <span class="flex h-9 w-14 shrink-0 items-center justify-center overflow-hidden rounded-[5px] bg-muted">
              <img v-if="entry.thumbnail && !failedImages.has(position) && remoteImagesAllowed(settingsStore.settings.network, entry.thumbnail)" :src="entry.thumbnail" alt="" loading="lazy" referrerpolicy="no-referrer" class="size-full object-cover" @error="failedImages.add(position)" />
              <Film v-else class="size-4 text-muted-foreground" aria-hidden="true" />
            </span>
            <span class="min-w-0 flex-1">
              <bdi dir="auto" class="block truncate text-[13px]">{{ entry.title || t('inputFilters.playlistSelection.itemFallback', { n: entry.index + 1 }) }}</bdi>
              <bdi v-if="entry.duration != null" dir="ltr" class="block text-[12px] text-muted-foreground">{{ formatDuration(entry.duration, { pad: true }) }}</bdi>
            </span>
          </label>
        </div>
      </div>
    </div>
    <div class="flex flex-wrap items-center justify-between gap-2">
      <SelectField v-model="quality" :options="qualityOptions" :aria-label="t('sheet.format')" :disabled="isApplying" />
      <Button v-if="showAction" size="xs" :disabled="isApplying || selected.size === 0" @click="applySelection">
        <Loader2 v-if="isApplying" class="animate-spin motion-reduce:animate-none" aria-hidden="true" />
        {{ t('inputFilters.playlistSelection.applySelection', { count: selected.size }, selected.size) }}
      </Button>
    </div>
  </div>
</template>
