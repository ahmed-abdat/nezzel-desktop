<script setup lang="ts">
// The chevron mirrors in RTL: "previous" sits at the inline start and points that way.
import type { HTMLAttributes } from 'vue';
import { computed } from 'vue';
import { ChevronRight } from '@lucide/vue';
import { CalendarNext, useForwardProps, type CalendarNextProps } from 'reka-ui';
import { cn } from 'cn';
import { buttonVariants } from '../button';

const props = defineProps<CalendarNextProps & { class?: HTMLAttributes['class'] }>();
const delegated = computed(() => {
  const rest = { ...props };
  delete rest.class;
  return rest;
});
const forwarded = useForwardProps(delegated);
</script>

<template>
  <CalendarNext
    data-slot="calendar-next-button"
    v-bind="forwarded"
    :class="cn(buttonVariants({ variant: 'ghost', size: 'icon-xs' }), 'shrink-0 p-0 text-muted-foreground hover:text-foreground', props.class)"
  >
    <slot>
      <ChevronRight class="size-4 rtl:-scale-x-100" aria-hidden="true" />
    </slot>
  </CalendarNext>
</template>
