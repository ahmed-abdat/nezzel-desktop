<script setup lang="ts">
import type { HTMLAttributes } from 'vue';
import { ContextMenuContent, ContextMenuPortal } from 'reka-ui';
import { cn } from 'cn';

const props = defineProps<{ class?: HTMLAttributes['class'] }>();
</script>

<template>
  <ContextMenuPortal>
    <ContextMenuContent  data-slot="context-menu-content" :class="cn('z-[80] min-w-48 overflow-hidden rounded-lg border bg-popover p-1 text-foreground shadow-lg data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0 data-[state=open]:zoom-in-95', props.class)">
      <slot />
    </ContextMenuContent>
  </ContextMenuPortal>
</template>
