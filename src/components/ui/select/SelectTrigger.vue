<script setup lang="ts">
// 28px trigger, same box as the kit's text fields: hairline input border, card fill, chevron at the end.
import type { HTMLAttributes } from 'vue';
import { SelectIcon, SelectTrigger, type SelectTriggerProps } from 'reka-ui';
import { ChevronsUpDown } from '@lucide/vue';
import { cn } from 'cn';

const props = defineProps<SelectTriggerProps & { class?: HTMLAttributes['class'] }>();
</script>

<template>
  <SelectTrigger
    data-slot="select-trigger"
    :disabled="props.disabled"
    :as-child="props.asChild"
    :as="props.as"
    :reference="props.reference"
    :class="cn(
      'inline-flex h-7 w-fit min-w-32 max-w-full items-center justify-between gap-2 rounded-[6px] border border-input bg-card ps-2.5 pe-2 text-nz-meta text-foreground shadow-xs outline-none transition-[border-color,box-shadow] duration-(--nz-dur-fast)',
      'hover:border-foreground/25 focus-visible:ring-2 focus-visible:ring-ring/40 data-[state=open]:border-primary/50 data-[state=open]:ring-2 data-[state=open]:ring-ring/25',
      'disabled:cursor-not-allowed disabled:opacity-50 data-[placeholder]:text-muted-foreground',
      '[&>span]:min-w-0 [&>span]:truncate [&>span]:text-start',
      props.class,
    )"
  >
    <slot />
    <SelectIcon as-child>
      <ChevronsUpDown class="size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
    </SelectIcon>
  </SelectTrigger>
</template>
