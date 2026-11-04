<script setup lang="ts">
import type { HTMLAttributes } from 'vue';
import { Check } from '@lucide/vue';
import { CheckboxIndicator, CheckboxRoot, useForwardPropsEmits, type CheckboxRootEmits, type CheckboxRootProps } from 'reka-ui';
import { cn } from 'cn';

const props = defineProps<CheckboxRootProps & { class?: HTMLAttributes['class'] }>();
const emits = defineEmits<CheckboxRootEmits>();
const forwarded = useForwardPropsEmits(props, emits);
</script>

<template>
  <CheckboxRoot
    v-bind="forwarded"
    data-slot="checkbox"
    :class="cn('peer grid size-4 shrink-0 place-items-center rounded-[4px] border border-foreground/30 bg-card shadow-xs outline-none focus-visible:ring-2 focus-visible:ring-ring/40 disabled:opacity-50 data-[state=checked]:border-primary data-[state=checked]:bg-primary data-[state=checked]:text-primary-foreground', props.class)"
  >
    <CheckboxIndicator><Check class="size-3" :stroke-width="3" /></CheckboxIndicator>
  </CheckboxRoot>
</template>
