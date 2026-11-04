<script setup lang="ts">
// Multi-line field with the same chrome as Input (`field` variant). Attributes fall through.
import type { HTMLAttributes } from 'vue';
import { cn } from 'cn';

const props = defineProps<{ class?: HTMLAttributes['class'] }>();
const model = defineModel<string | null>();
</script>

<template>
  <textarea
    v-model="model"
    data-slot="textarea"
    :class="cn(
      'w-full min-w-0 rounded-[6px] border border-input bg-card px-2.5 py-1.5 text-[13px] text-foreground shadow-xs outline-none transition-[color,box-shadow,border-color] placeholder:text-muted-foreground/70 focus-visible:ring-2 focus-visible:ring-ring/30 disabled:cursor-not-allowed disabled:opacity-50 aria-invalid:border-destructive',
      props.class,
    )"
  />
</template>
