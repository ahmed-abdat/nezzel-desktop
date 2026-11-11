<template>
  <!-- Quick picks: secondary first, primary last, so Download keeps the inline end in both directions. -->
  <div
    ref="group"
    role="group"
    :aria-label="t('capture.pick.group')"
    class="flex items-center gap-2"
    data-testid="capture-picks"
    @keydown="onArrow"
  >
    <div class="grid min-h-8 min-w-[272px] shrink-0 grid-cols-[auto_auto] gap-2">
      <div v-if="loading" class="col-span-2 grid grid-cols-2 gap-2" role="status" :aria-label="t('capture.loading')" data-testid="capture-choices-loading">
        <Skeleton v-for="slot in 2" :key="slot" class="h-8 w-full" />
      </div>
      <tooltip v-for="pick in secondary" :key="pick.key" :content="t('capture.pick.keyTip', { n: keyOf(pick.key) })">
        <Button
          type="button"
          variant="outline"
          size="sm"
          class="w-full min-w-0 last:col-start-2"
          :disabled="disabled"
          :aria-keyshortcuts="String(keyOf(pick.key))"
          :data-pick="pick.key"
          @click="emit('pick', pick.key)"
        >
          <Music v-if="pick.key === 'audio'" aria-hidden="true" />
          <bdi class="shrink-0">{{ label(pick) }}</bdi>
          <bdi v-if="!compact && pick.size" dir="ltr" class="tnum shrink-0 whitespace-nowrap font-normal text-muted-foreground">{{ formatSize(pick.size) }}</bdi>
        </Button>
      </tooltip>
    </div>
    <tooltip :content="t('capture.pick.keyTip', { n: keyOf('best') })" :disabled="secondary.length === 0">
      <Button
        :type="submit ? 'submit' : 'button'"
        size="sm"
        :disabled="disabled"
        :aria-busy="acting || undefined"
        aria-keyshortcuts="1 Enter"
        data-pick="best"
        @click="!submit && emit('pick', 'best')"
      >
        <LoaderCircle v-if="acting" class="animate-spin" aria-hidden="true" /><ArrowDownToLine v-else aria-hidden="true" />
        {{ acting ? t('capture.starting') : chooseItems ? t('capture.chooseItems') : compact ? t('capture.pick.download') : t('capture.downloadBest') }}
      </Button>
    </tooltip>
  </div>
</template>

<script setup lang="ts">
// The quick bar's and copied-link card's download picks (Best / Audio / Smaller). Which picks show is
// decided by `capturePicks` (helpers/captureChoices); this only draws them and moves focus with the arrow keys (RTL-aware).
import { computed, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import { ArrowDownToLine, LoaderCircle, Music } from '@lucide/vue';
import { Skeleton } from '@/components/ui/skeleton';
import { Button } from '@/components/ui/button';
import Tooltip from '@/components/ui/tooltip/Tooltip.vue';
import { useLocaleNumber } from '../../composables/useLocaleNumber';
import type { CapturePick, PickKey } from '../../helpers/captureChoices';
import { SMALL_MAX_HEIGHT } from '@/generated/contract';

const props = defineProps<{ picks: CapturePick[]; disabled: boolean; acting: boolean; compact?: boolean; submit?: boolean; loading?: boolean; chooseItems?: boolean }>();
const emit = defineEmits<{ pick: [key: PickKey] }>();
const { t } = useI18n();
const { size: formatSize } = useLocaleNumber();
const group = ref<HTMLElement | null>(null);

/** Smaller before Audio in the DOM, so reading order runs small → audio → best toward the inline end. */
const secondary = computed(() => (['small', 'audio'] as const).flatMap(key => props.picks.filter(pick => pick.key === key)));

/** 1 = Download (best), 2 = Audio, 3 = Smaller: fixed per pick, so a key never changes meaning after the read. */
function keyOf(key: PickKey) {
  return key === 'best' ? 1 : key === 'audio' ? 2 : 3;
}
function label(pick: CapturePick) {
  return pick.key === 'audio' ? t('capture.audio') : `${pick.height ?? SMALL_MAX_HEIGHT}p`;
}

function onArrow(event: KeyboardEvent) {
  if (event.key !== 'ArrowLeft' && event.key !== 'ArrowRight') return;
  const buttons = [...(group.value?.querySelectorAll<HTMLButtonElement>('button:not(:disabled)') ?? [])];
  const at = buttons.indexOf(document.activeElement as HTMLButtonElement);
  if (at < 0) return;
  event.preventDefault();
  const rtl = getComputedStyle(group.value!).direction === 'rtl';
  const step = (event.key === 'ArrowRight') !== rtl ? 1 : -1;
  buttons[(at + step + buttons.length) % buttons.length]?.focus();
}
</script>
