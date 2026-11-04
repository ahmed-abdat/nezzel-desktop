<script setup lang="ts">
// Native <select>: the OS draws the menu, which is the most native popup on every platform.
import type { HTMLAttributes } from 'vue';
import { ChevronsUpDown } from '@lucide/vue';
import { cn } from 'cn';

defineOptions({ inheritAttrs: false });
const props = defineProps<{ class?: HTMLAttributes['class'] }>();
const model = defineModel<string | number | null | undefined>();
</script>

<template>
  <span data-slot="native-select" :class="cn('relative inline-flex h-7 max-w-full items-center rounded-[6px] border border-input bg-card shadow-xs transition-colors hover:border-foreground/25 focus-within:ring-2 focus-within:ring-ring/30 has-[select:disabled]:opacity-50 has-[select:disabled]:hover:border-input', props.class)">
    <select v-model="model" v-bind="$attrs" class="h-full max-w-full cursor-pointer truncate appearance-none bg-transparent ps-2.5 pe-7 text-[13px] text-foreground outline-none disabled:cursor-not-allowed">
      <slot />
    </select>
    <ChevronsUpDown class="pointer-events-none absolute end-2 size-3 text-muted-foreground" aria-hidden="true" />
  </span>
</template>
