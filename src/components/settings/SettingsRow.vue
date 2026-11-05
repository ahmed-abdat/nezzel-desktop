<template>
  <div
    class="grid min-h-[52px] items-center gap-x-8 gap-y-1 border-b border-foreground/[0.08] py-3 last:border-b-0"
    :class="stacked ? 'grid-cols-1' : 'grid-cols-[minmax(0,1fr)_auto]'"
  >
    <component :is="forId ? 'label' : 'span'" v-if="cleanLabel || $slots.label" :for="forId" class="col-start-1 text-[13px]">
      <slot name="label">{{ cleanLabel }}</slot>
    </component>
    <p v-if="help || $slots.help" class="col-start-1 text-[12px] leading-relaxed text-muted-foreground text-pretty">
      <slot name="help">{{ help }}</slot>
    </p>
    <div
      v-if="$slots.default"
      class="flex items-center gap-2"
      :class="stacked ? 'mt-1' : ['col-start-2 row-start-1 justify-self-end', help || $slots.help ? 'row-span-2' : '']"
    >
      <slot />
    </div>
  </div>
</template>

<script setup lang="ts">
// One preference: label (+ optional help) on the start side, control on the end side.
// `stacked` puts a wide control (textarea, multi-select) under the label instead.
import { computed } from 'vue';

const props = defineProps<{ label?: string; help?: string; forId?: string; stacked?: boolean }>();

// Some legacy labels end with a colon ("Connect via proxy:"); a preferences row doesn't want it.
const cleanLabel = computed(() => props.label?.replace(/\s*[:：]\s*$/, '') ?? '');
</script>
