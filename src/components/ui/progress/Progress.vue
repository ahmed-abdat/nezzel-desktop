<script setup lang="ts">
// Thin bar; fill grows from the inline start. `null` = indeterminate.
defineProps<{ value: number | null; muted?: boolean }>();
</script>

<template>
  <div data-slot="progress" class="h-1 w-full overflow-hidden rounded-full bg-foreground/15 dark:bg-foreground/25" aria-hidden="true">
    <div
      v-if="value != null"
      class="h-full rounded-full transition-[width] duration-500 ease-out"
      :class="muted ? 'bg-muted-foreground/60' : 'bg-primary'"
      :style="{ width: `${value}%` }"
    />
    <div v-else class="nz-progress-indeterminate h-full w-1/3 rounded-full bg-primary/70" />
  </div>
</template>

<style scoped>
.nz-progress-indeterminate { animation: nz-indet 1.2s ease-in-out infinite; }
@keyframes nz-indet { from { transform: translateX(-100%); } to { transform: translateX(300%); } }
[dir="rtl"] .nz-progress-indeterminate { animation-name: nz-indet-rtl; }
@keyframes nz-indet-rtl { from { transform: translateX(100%); } to { transform: translateX(-300%); } }
@media (prefers-reduced-motion: reduce) { .nz-progress-indeterminate { animation: none; width: 100%; opacity: 0.35; } }
</style>
