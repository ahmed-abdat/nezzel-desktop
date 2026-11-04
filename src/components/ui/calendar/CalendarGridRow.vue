<script setup lang="ts">
import type { HTMLAttributes } from 'vue';
import { computed } from 'vue';
import { CalendarGridRow, useForwardProps, type CalendarGridRowProps } from 'reka-ui';
import { cn } from 'cn';

const props = defineProps<CalendarGridRowProps & { class?: HTMLAttributes['class'] }>();
const delegated = computed(() => {
  const rest = { ...props };
  delete rest.class;
  return rest;
});
const forwarded = useForwardProps(delegated);
</script>

<template>
  <CalendarGridRow data-slot="calendar-grid-row" v-bind="forwarded" :class="cn('flex', props.class)">
    <slot />
  </CalendarGridRow>
</template>
