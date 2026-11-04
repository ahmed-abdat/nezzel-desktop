<template>
  <!-- One flat row per tool: status icon, name + version, a thin bar while it downloads. Failed = red-tinted row with the log behind a disclosure. -->
  <li
      class="flex items-start gap-3 border-b border-hairline px-4 py-3"
      :class="hasError ? 'bg-destructive-soft text-destructive-foreground' : 'text-foreground'"
      :data-state="state"
  >
    <span
        class="grid size-7 shrink-0 place-items-center rounded-full"
        :class="{
          'bg-muted text-muted-foreground': state === 'progress',
          'bg-primary-soft text-primary-soft-foreground': state === 'done',
          'text-destructive-foreground': state === 'error',
        }"
        aria-hidden="true"
    >
      <Download v-if="state === 'progress'" class="size-4" />
      <Check v-else-if="state === 'done'" class="size-4" />
      <CircleX v-else class="size-5" />
    </span>
    <div class="min-w-0 flex-1">
      <div class="flex min-h-7 items-center gap-2">
        <h2 class="text-nz-meta font-medium"><bdi>{{ name }}</bdi></h2>
        <span
            v-if="tool.version"
            :title="tool.version"
            dir="ltr"
            class="min-w-0 truncate text-nz-caption"
            :class="hasError ? 'opacity-80' : 'text-muted-foreground'"
        >{{ tool.version }}</span>
        <span v-if="state === 'progress'" class="ms-auto shrink-0 text-nz-caption text-muted-foreground">
          {{ t('common.percentage', { percent: clampedPercent }) }}
        </span>
      </div>
      <template v-if="!hasError">
        <div
            v-if="state === 'progress'"
            class="mt-1"
            role="progressbar"
            :aria-valuemin="0"
            :aria-valuemax="100"
            :aria-valuenow="clampedPercent"
            :aria-label="name"
        >
          <Progress :value="clampedPercent" />
        </div>
        <p v-if="tool.total > 0" class="mt-1 text-nz-caption text-muted-foreground">
          <bdi dir="ltr">{{ t('common.divide', { left: formatBytes(tool.received), right: formatBytes(tool.total) }) }}</bdi>
        </p>
      </template>
      <Collapsible v-else class="group" :unmount-on-hide="false">
        <CollapsibleTrigger class="flex w-fit cursor-pointer items-center gap-1 rounded-[6px] text-nz-caption outline-none focus-visible:ring-2 focus-visible:ring-ring/40">
          {{ t('components.base.toolCard.failed') }}
          <ChevronDown class="size-3.5 transition-transform duration-150 group-data-[state=open]:rotate-180" aria-hidden="true" />
        </CollapsibleTrigger>
        <CollapsibleContent>
          <pre dir="ltr" class="mt-2 max-h-48 overflow-auto whitespace-pre-wrap break-words rounded-md bg-card/70 p-2 text-start font-mono text-[11.5px] text-foreground"><code>{{ tool.error }}</code></pre>
        </CollapsibleContent>
      </Collapsible>
    </div>
  </li>
</template>
<script setup lang="ts">
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from '../ui/collapsible';
import { formatBytes } from '../../helpers/units';
import { Check, ChevronDown, CircleX, Download } from '@lucide/vue';
import { computed, PropType } from 'vue';
import { BinaryProgress } from '../../tauri/types/binaries';
import { useI18n } from 'vue-i18n';
import { Progress } from '../ui/progress';

const { t } = useI18n();

const { tool } = defineProps({
  name: {
    type: String,
    required: true,
  },
  tool: {
    type: Object as PropType<BinaryProgress>,
    required: true,
  },
});

const hasError = computed(() => !!tool.error);
const clampedPercent = computed(() => Math.round(Math.min(Math.max(tool.percent, 0), 100)));
const state = computed(() => (hasError.value ? 'error' : clampedPercent.value >= 100 ? 'done' : 'progress'));
</script>

<style scoped>
summary::-webkit-details-marker {
  display: none;
}
</style>
