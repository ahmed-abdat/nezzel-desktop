<script setup lang="ts">
// Text field. `field` = the 28px kit control (hairline border, card fill, focus ring).
// `bare` = no chrome, for inputs that sit inside a composed shell (link field, list search).
// Attributes (id, type, placeholder, dir, aria-*) fall through to the <input>.
import type { HTMLAttributes } from 'vue';
import { ref } from 'vue';
import { cn } from 'cn';

const props = withDefaults(defineProps<{ class?: HTMLAttributes['class']; variant?: 'field' | 'bare' }>(), { variant: 'field' });
const model = defineModel<string | number | null>();
const el = ref<HTMLInputElement | null>(null);

defineExpose({
  el,
  focus: () => el.value?.focus(),
  select: () => el.value?.select(),
});
</script>

<template>
  <input
    ref="el"
    v-model="model"
    data-slot="input"
    :class="cn(
      'min-w-0 text-[13px] text-foreground outline-none placeholder:text-muted-foreground/70 disabled:cursor-not-allowed disabled:opacity-50',
      variant === 'field' && 'h-7 w-full rounded-[6px] border border-input bg-card px-2.5 shadow-xs transition-[color,box-shadow,border-color] focus-visible:ring-2 focus-visible:ring-ring/30 aria-invalid:border-destructive',
      variant === 'bare' && 'bg-transparent',
      props.class,
    )"
  >
</template>
