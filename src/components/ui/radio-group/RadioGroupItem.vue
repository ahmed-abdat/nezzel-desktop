<script setup lang="ts">
// A full-width radio row: the indicator plus a label slot. The whole row is the radio (role="radio").
import type { HTMLAttributes } from 'vue';
import { RadioGroupIndicator, RadioGroupItem, useForwardProps, type RadioGroupItemProps } from 'reka-ui';
import { cn } from 'cn';

const props = defineProps<RadioGroupItemProps & { class?: HTMLAttributes['class'] }>();
const forwarded = useForwardProps(props);
</script>

<template>
  <RadioGroupItem
    v-bind="forwarded"
    data-slot="radio-group-item"
    :class="cn('group/radio flex w-full items-center gap-3 text-start outline-none focus-visible:bg-accent focus-visible:outline-solid focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-[var(--nz-focus)] disabled:cursor-not-allowed disabled:opacity-50', props.class)"
  >
    <span class="grid size-4 shrink-0 place-items-center rounded-full border border-foreground/25 bg-card group-data-[state=checked]/radio:border-primary group-data-[state=checked]/radio:bg-primary">
      <RadioGroupIndicator class="size-1.5 rounded-full bg-primary-foreground" />
    </span>
    <slot />
  </RadioGroupItem>
</template>
