<template>
  <!-- A row that navigates (Logins ›, Subtitles ›): the whole row is the link, chevron at the end. -->
  <router-link
    :to="to"
    class="group grid min-h-[52px] grid-cols-[minmax(0,1fr)_auto] items-center gap-x-8 gap-y-0.5 border-b border-hairline py-3 outline-none last:border-b-0 focus-visible:ring-2 focus-visible:ring-ring/40"
  >
    <span class="col-start-1 text-[13px] group-hover:text-primary">{{ label }}</span>
    <span v-if="help" class="col-start-1 text-[12px] leading-relaxed text-muted-foreground text-pretty">{{ help }}</span>
    <span class="col-start-2 row-start-1 inline-flex items-center gap-1 text-[12.5px] text-muted-foreground" :class="help && 'row-span-2'">
      <span v-if="value">{{ value }}</span>
      <ChevronRight class="size-4 rtl:rotate-180" aria-hidden="true" />
    </span>
  </router-link>
</template>

<script setup lang="ts">
import type { RouteLocationRaw } from 'vue-router';
import { ChevronRight } from '@lucide/vue';

defineProps<{ to: RouteLocationRaw; label: string; help?: string; value?: string }>();
</script>
