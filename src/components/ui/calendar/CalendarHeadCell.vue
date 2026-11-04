<script setup lang="ts">
import type { HTMLAttributes } from 'vue';
import { computed } from 'vue';
import { CalendarHeadCell, useForwardProps, type CalendarHeadCellProps } from 'reka-ui';
import { cn } from 'cn';

const props = defineProps<CalendarHeadCellProps & { class?: HTMLAttributes['class'] }>();
const delegated = computed(() => {
  const rest = { ...props };
  delete rest.class;
  return rest;
});
const forwarded = useForwardProps(delegated);
</script>

<template>
  <CalendarHeadCell data-slot="calendar-head-cell" v-bind="forwarded" :class="cn('flex-1 rounded-md text-[11px] font-normal text-muted-foreground', props.class)">
    <slot />
  </CalendarHeadCell>
</template>
