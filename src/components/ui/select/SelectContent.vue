<script setup lang="ts">
// Menu panel: popover surface, hairline border, shadow-lg (the one place menus get depth).
// Popper position below the trigger on every OS, so the menu looks the same on macOS, Windows and Linux.
import type { HTMLAttributes } from 'vue';
import {
  SelectContent,
  SelectPortal,
  SelectScrollDownButton,
  SelectScrollUpButton,
  SelectViewport,
  type SelectContentProps,
} from 'reka-ui';
import { ChevronDown, ChevronUp } from '@lucide/vue';
import { cn } from 'cn';

defineOptions({ inheritAttrs: false });
const props = withDefaults(
  defineProps<SelectContentProps & { class?: HTMLAttributes['class'] }>(),
  { position: 'popper', sideOffset: 4, align: 'start' },
);
</script>

<template>
  <SelectPortal>
    <SelectContent
      data-slot="select-content"
      v-bind="$attrs"
      :position="props.position"
      :side="props.side"
      :side-offset="props.sideOffset"
      :align="props.align"
      :avoid-collisions="props.avoidCollisions ?? true"
      :collision-padding="props.collisionPadding ?? 8"
      :body-lock="props.bodyLock"
      :class="cn(
        'relative z-[90] max-h-[min(20rem,var(--reka-select-content-available-height))] min-w-32 overflow-hidden rounded-lg border border-hairline bg-popover text-foreground shadow-lg',
        'data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0',
        props.position === 'popper' && 'w-full min-w-[var(--reka-select-trigger-width)] data-[side=bottom]:slide-in-from-top-1 data-[side=top]:slide-in-from-bottom-1',
        props.class,
      )"
    >
      <SelectScrollUpButton class="flex h-5 cursor-default items-center justify-center text-muted-foreground">
        <ChevronUp class="size-3.5" aria-hidden="true" />
      </SelectScrollUpButton>
      <SelectViewport
        :class="cn('p-1', props.position === 'popper' && 'h-[var(--reka-select-trigger-height)] w-full min-w-[var(--reka-select-trigger-width)] scroll-my-1')"
      >
        <slot />
      </SelectViewport>
      <SelectScrollDownButton class="flex h-5 cursor-default items-center justify-center text-muted-foreground">
        <ChevronDown class="size-3.5" aria-hidden="true" />
      </SelectScrollDownButton>
    </SelectContent>
  </SelectPortal>
</template>
