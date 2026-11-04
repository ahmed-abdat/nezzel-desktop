<script setup lang="ts">
import type { HTMLAttributes } from 'vue';
import { computed } from 'vue';
import { CalendarHeader, useForwardProps, type CalendarHeaderProps } from 'reka-ui';
import { cn } from 'cn';

const props = defineProps<CalendarHeaderProps & { class?: HTMLAttributes['class'] }>();
const delegated = computed(() => {
  const rest = { ...props };
  delete rest.class;
  return rest;
});
const forwarded = useForwardProps(delegated);
</script>

<template>
  <CalendarHeader data-slot="calendar-header" v-bind="forwarded" :class="cn('relative flex w-full items-center justify-between gap-1', props.class)">
    <slot />
  </CalendarHeader>
</template>
