<script setup lang="ts">
import type { HTMLAttributes } from 'vue';
import { Check } from '@lucide/vue';
import { ContextMenuCheckboxItem, ContextMenuItemIndicator } from 'reka-ui';
import { cn } from 'cn';

const props = defineProps<{ class?: HTMLAttributes['class'] }>();
const model = defineModel<boolean>({ default: false });
</script>

<template>
  <ContextMenuCheckboxItem v-model="model" data-slot="context-menu-checkbox-item" :class="cn('relative flex cursor-default select-none items-center gap-2 rounded-[5px] px-2 py-1 text-[13px] outline-none data-[disabled]:pointer-events-none data-[disabled]:opacity-50 data-[highlighted]:bg-primary data-[highlighted]:text-primary-foreground [&_svg]:size-4 [&_svg]:shrink-0 ps-7', props.class)" @select.prevent>
    <span class="absolute start-2 grid size-4 place-items-center"><ContextMenuItemIndicator><Check class="!size-3.5" /></ContextMenuItemIndicator></span>
    <slot />
  </ContextMenuCheckboxItem>
</template>
