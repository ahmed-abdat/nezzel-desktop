<script setup lang="ts">
import type { HTMLAttributes } from 'vue';
import { computed } from 'vue';
import { CalendarGrid, useForwardProps, type CalendarGridProps } from 'reka-ui';
import { cn } from 'cn';

const props = defineProps<CalendarGridProps & { class?: HTMLAttributes['class'] }>();
const delegated = computed(() => {
  const rest = { ...props };
  delete rest.class;
  return rest;
});
const forwarded = useForwardProps(delegated);
</script>

<template>
  <CalendarGrid data-slot="calendar-grid" v-bind="forwarded" :class="cn('w-full border-collapse', props.class)">
    <slot />
  </CalendarGrid>
</template>
