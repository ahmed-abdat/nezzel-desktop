<script setup lang="ts">
import type { HTMLAttributes } from 'vue';
import { computed } from 'vue';
import { CalendarCellTrigger, useForwardProps, type CalendarCellTriggerProps } from 'reka-ui';
import { cn } from 'cn';
import { buttonVariants } from '../button';

const props = withDefaults(defineProps<CalendarCellTriggerProps & { class?: HTMLAttributes['class'] }>(), { as: 'button' });
const delegated = computed(() => {
  const rest = { ...props };
  delete rest.class;
  return rest;
});
const forwarded = useForwardProps(delegated);
</script>

<template>
  <CalendarCellTrigger
    data-slot="calendar-cell-trigger"
    v-bind="forwarded"
    :class="cn(
      buttonVariants({ variant: 'ghost' }),
      'size-8 cursor-default p-0 text-[13px] font-normal tabular-nums',
      '[&[data-today]:not([data-selected])]:bg-muted [&[data-today]:not([data-selected])]:font-semibold',
      'data-[selected]:bg-primary data-[selected]:text-primary-foreground data-[selected]:hover:bg-primary-hover',
      'data-[disabled]:text-muted-foreground data-[disabled]:opacity-50',
      'data-[unavailable]:text-destructive-foreground data-[unavailable]:line-through',
      'data-[outside-view]:text-muted-foreground/60',
      props.class,
    )"
  >
    <slot />
  </CalendarCellTrigger>
</template>
