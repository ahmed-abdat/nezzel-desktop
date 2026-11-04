<script setup lang="ts">
import type { HTMLAttributes } from 'vue';
import { SelectLabel } from 'reka-ui';
import { cn } from 'cn';

const props = defineProps<{ class?: HTMLAttributes['class'] }>();
</script>

<template>
  <SelectLabel data-slot="select-label" :class="cn('px-2 pb-0.5 pt-1.5 text-nz-caption font-semibold text-muted-foreground', props.class)">
    <slot />
  </SelectLabel>
</template>
