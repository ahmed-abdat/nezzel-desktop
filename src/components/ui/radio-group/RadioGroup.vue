<script setup lang="ts">
import type { HTMLAttributes } from 'vue';
import { RadioGroupRoot, useForwardPropsEmits, type RadioGroupRootEmits, type RadioGroupRootProps } from 'reka-ui';
import { cn } from 'cn';

const props = defineProps<RadioGroupRootProps & { class?: HTMLAttributes['class'] }>();
const emits = defineEmits<RadioGroupRootEmits>();
const forwarded = useForwardPropsEmits(props, emits);
</script>

<template>
  <RadioGroupRoot v-bind="forwarded" data-slot="radio-group" :class="cn('grid', props.class)">
    <slot />
  </RadioGroupRoot>
</template>
