<script setup lang="ts">
import { ContextMenuLabel } from 'reka-ui';
</script>

<template>
  <ContextMenuLabel data-slot="context-menu-label" class="px-2 pb-1 pt-1.5 text-[11.5px] font-semibold text-muted-foreground">
    <slot />
  </ContextMenuLabel>
</template>
