<script setup lang="ts">
// Compact stepper for small counts (parallel downloads, parts). Typing works too.
import { Minus, Plus } from '@lucide/vue';
import { NumberFieldDecrement, NumberFieldIncrement, NumberFieldInput, NumberFieldRoot } from 'reka-ui';

withDefaults(defineProps<{ min?: number; max?: number; id?: string; ariaLabel?: string }>(), { min: 1, max: 32 });
const model = defineModel<number>({ required: true });
</script>

<template>
  <NumberFieldRoot v-model="model" :min="min" :max="max" :id="id" dir="ltr" class="inline-flex h-7 items-center rounded-[6px] border border-input bg-card shadow-xs focus-within:ring-2 focus-within:ring-ring/30">
    <NumberFieldDecrement class="grid h-full w-7 place-items-center text-muted-foreground hover:text-foreground disabled:opacity-40"><Minus class="size-3.5" /></NumberFieldDecrement>
    <NumberFieldInput :aria-label="ariaLabel" class="tnum w-8 bg-transparent text-center text-[13px] font-medium outline-none" />
    <NumberFieldIncrement class="grid h-full w-7 place-items-center text-muted-foreground hover:text-foreground disabled:opacity-40"><Plus class="size-3.5" /></NumberFieldIncrement>
  </NumberFieldRoot>
</template>
