<script setup lang="ts">
import type { HTMLAttributes } from 'vue';
import { computed } from 'vue';
import { CalendarCell, useForwardProps, type CalendarCellProps } from 'reka-ui';
import { cn } from 'cn';

const props = defineProps<CalendarCellProps & { class?: HTMLAttributes['class'] }>();
const delegated = computed(() => {
  const rest = { ...props };
  delete rest.class;
  return rest;
});
const forwarded = useForwardProps(delegated);
</script>

<template>
  <CalendarCell data-slot="calendar-cell" v-bind="forwarded" :class="cn('relative flex-1 p-0 text-center text-[13px] focus-within:relative focus-within:z-20', props.class)">
    <slot />
  </CalendarCell>
</template>
