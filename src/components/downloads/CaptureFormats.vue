<template>
  <!-- The card's format list: flat full-bleed rows under one hairline (no box, no shadow), grouped Video / Audio.
       Each row downloads that format. Opened from the ▾ half of Download best; the window grows to fit it. -->
  <div
    :id="id"
    ref="root"
    role="menu"
    :aria-label="t('capture.formats.list')"
    class="max-h-[300px] shrink-0 overflow-y-auto overscroll-contain border-t border-hairline pb-1.5"
    data-testid="capture-formats"
    @keydown="onKey"
  >
    <div v-for="group in groups" :key="group.kind" role="group" :aria-label="group.label">
      <p
        aria-hidden="true"
        class="sticky top-0 z-[1] flex h-7 items-end bg-background px-3.5 pb-1 text-[11px] font-semibold text-muted-foreground"
        :class="group.kind === 'audio' && groups.length > 1 && 'mt-1.5 shadow-[inset_0_1px_0_var(--hairline)]'"
      >
        {{ group.label }}
      </p>
      <button
        v-for="row in group.rows"
        :key="row.key"
        type="button"
        role="menuitem"
        tabindex="-1"
        :disabled="disabled"
        :data-choice="row.key"
        class="group/row grid h-8 w-full grid-cols-[80px_minmax(0,1fr)_auto_14px] items-center gap-x-2.5 px-3.5 text-start hover:bg-accent focus-visible:bg-accent focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-[var(--nz-focus)] disabled:opacity-50"
        @click="emit('pick', row.choice)"
      >
        <bdi class="tnum truncate font-semibold">{{ quality(row) }}</bdi>
        <span class="flex min-w-0 items-baseline gap-1.5 truncate text-[12px] text-muted-foreground">
          <span v-if="row.best" class="text-[11.5px] font-semibold text-primary">{{ t('capture.formats.best') }}</span>
          <bdi class="truncate">{{ detail(row) }}</bdi>
        </span>
        <bdi
          :dir="row.size ? 'ltr' : undefined"
          class="tnum text-end text-[12px] whitespace-nowrap text-muted-foreground"
          :class="!row.size && 'opacity-60'"
          :title="!row.size && !row.upTo && row.choice.kind !== 'best' ? t('capture.formats.sizeUnknown') : undefined"
        >{{ sizeOf(row) }}</bdi>
        <ArrowDownToLine class="size-3.5 text-primary opacity-0 group-hover/row:opacity-100 group-focus-visible/row:opacity-100" aria-hidden="true" />
      </button>
    </div>
  </div>
</template>

<script setup lang="ts">
// Rows come from `cardRows` (helpers/captureChoices); this only draws them and moves focus (menu keyboard model).
import { computed, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import { ArrowDownToLine } from '@lucide/vue';
import { useLocaleNumber } from '../../composables/useLocaleNumber';
import type { CardRow, RowChoice } from '../../helpers/captureChoices';

const props = defineProps<{ id: string; rows: CardRow[]; bestExt: string; disabled: boolean }>();
const emit = defineEmits<{ pick: [choice: RowChoice]; close: [] }>();
const { t } = useI18n();
const { size: formatSize } = useLocaleNumber();
const root = ref<HTMLElement | null>(null);

const groups = computed(() => {
  const video = props.rows.filter(row => row.choice.kind !== 'audio');
  const audio = props.rows.filter(row => row.choice.kind === 'audio');
  return [
    ...(video.length ? [{ kind: 'video', label: t('capture.formats.video'), rows: video }] : []),
    ...(audio.length ? [{ kind: 'audio', label: t('capture.formats.audio'), rows: audio }] : []),
  ];
});

function quality({ choice }: CardRow) {
  if (choice.kind === 'best') return t('capture.formats.best');
  if (choice.kind === 'audio') return choice.kbps ? t('capture.formats.kbps', { n: choice.kbps }) : t('capture.audio');
  return t('capture.formats.height', { n: choice.height });
}
function detail(row: CardRow) {
  const { choice } = row;
  if (choice.kind === 'best') return t('capture.formats.bestEach');
  if (choice.kind === 'audio') {
    if (!choice.kbps) return t('capture.formats.anyAudio');
    return t(choice.kbps >= 128 ? 'capture.formats.high' : choice.kbps >= 64 ? 'capture.formats.medium' : 'capture.formats.low');
  }
  const h = choice.height;
  return [
    choice.fps ? t('capture.formats.fps', { n: choice.fps }) : '',
    h >= 4320 ? t('capture.formats.res8k') : h >= 2160 ? t('capture.formats.res4k') : '',
    row.upTo ? t('capture.formats.orLower') : props.bestExt.toUpperCase(),
  ].filter(Boolean).join(t('capture.formats.sep'));
}
/** Unknown sizes show a quiet dash (tooltip says why); generic rows have no size to show. */
function sizeOf(row: CardRow) {
  if (row.size) return formatSize(row.size);
  return row.upTo || row.choice.kind === 'best' ? '' : t('capture.formats.noSize');
}

function items() {
  return [...(root.value?.querySelectorAll<HTMLButtonElement>('[role=menuitem]:not(:disabled)') ?? [])];
}
/** Opened from the keyboard: the first row takes focus, like a native menu. */
function focusFirst() {
  items()[0]?.focus();
}
function onKey(event: KeyboardEvent) {
  const list = items();
  const at = list.indexOf(document.activeElement as HTMLButtonElement);
  const move = (index: number) => {
    event.preventDefault();
    list[(index + list.length) % list.length]?.focus();
  };
  if (event.key === 'ArrowDown') move(at + 1);
  else if (event.key === 'ArrowUp') move(at < 0 ? -1 : at - 1);
  else if (event.key === 'Home') move(0);
  else if (event.key === 'End') move(-1);
  else if (event.key === 'Escape' || event.key === 'Tab') {
    // Esc closes the list, not the card.
    if (event.key === 'Escape') {
      event.preventDefault();
      event.stopPropagation();
    }
    emit('close');
  }
}
defineExpose({ focusFirst });
</script>
