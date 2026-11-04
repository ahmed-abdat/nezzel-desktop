<script setup lang="ts">
import type { HTMLAttributes } from 'vue';
import { SelectSeparator } from 'reka-ui';
import { cn } from 'cn';

const props = defineProps<{ class?: HTMLAttributes['class'] }>();
</script>

<template>
  <SelectSeparator data-slot="select-separator" :class="cn('-mx-1 my-1 h-px bg-hairline', props.class)" />
</template>
