<script setup lang="ts">
import type { HTMLAttributes } from 'vue';
import { ContextMenuItem } from 'reka-ui';
import { cn } from 'cn';

const props = defineProps<{ class?: HTMLAttributes['class']; disabled?: boolean; danger?: boolean }>();
const emit = defineEmits<{ select: [event: Event] }>();
</script>

<template>
  <ContextMenuItem
    data-slot="context-menu-item"
    :disabled="disabled"
    :class="cn('relative flex cursor-default select-none items-center gap-2 rounded-[5px] px-2 py-1 text-[13px] outline-none data-[disabled]:pointer-events-none data-[disabled]:opacity-50 data-[highlighted]:bg-primary data-[highlighted]:text-primary-foreground [&_svg]:size-4 [&_svg]:shrink-0', danger && 'text-destructive-foreground data-[highlighted]:bg-destructive data-[highlighted]:text-white', props.class)"
    @select="emit('select', $event)"
  >
    <slot />
  </ContextMenuItem>
</template>
