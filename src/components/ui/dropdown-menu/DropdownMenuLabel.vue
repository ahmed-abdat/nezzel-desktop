<script setup lang="ts">
import { DropdownMenuLabel } from 'reka-ui';
</script>

<template>
  <DropdownMenuLabel data-slot="dropdown-menu-label" class="px-2 pb-1 pt-1.5 text-[11.5px] font-semibold text-muted-foreground">
    <slot />
  </DropdownMenuLabel>
</template>
