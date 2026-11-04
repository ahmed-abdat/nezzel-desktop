<script setup lang="ts">
// The chevron mirrors in RTL: "previous" sits at the inline start and points that way.
import type { HTMLAttributes } from 'vue';
import { computed } from 'vue';
import { ChevronLeft } from '@lucide/vue';
import { CalendarPrev, useForwardProps, type CalendarPrevProps } from 'reka-ui';
import { cn } from 'cn';
import { buttonVariants } from '../button';

const props = defineProps<CalendarPrevProps & { class?: HTMLAttributes['class'] }>();
const delegated = computed(() => {
  const rest = { ...props };
  delete rest.class;
  return rest;
});
const forwarded = useForwardProps(delegated);
</script>

<template>
  <CalendarPrev
    data-slot="calendar-prev-button"
    v-bind="forwarded"
    :class="cn(buttonVariants({ variant: 'ghost', size: 'icon-xs' }), 'shrink-0 p-0 text-muted-foreground hover:text-foreground', props.class)"
  >
    <slot>
      <ChevronLeft class="size-4 rtl:-scale-x-100" aria-hidden="true" />
    </slot>
  </CalendarPrev>
</template>
