<template>
  <!-- One "show more / Advanced" disclosure. `heading`: trigger above the panel (per-download tabs, developer options).
       `more`: link-style trigger below the panel, at the end of a settings section. -->
  <div :class="variant === 'more' && 'flex flex-col-reverse items-start'">
    <button
      type="button"
      :class="triggerClass"
      :aria-expanded="open"
      :aria-controls="panelId"
      @click="open = !open"
    >
      <ChevronRight v-if="variant === 'heading'" class="size-3.5 transition-transform duration-(--nz-dur-fast) rtl:rotate-180" :class="open && 'rotate-90 rtl:rotate-90'" aria-hidden="true" />
      {{ open && openLabel ? openLabel : label }}
      <ChevronDown v-if="variant === 'more'" class="size-3.5 transition-transform duration-150" :class="open && 'rotate-180'" aria-hidden="true" />
    </button>
    <div v-show="open" :id="panelId" :class="variant === 'more' ? 'self-stretch border-t border-hairline' : 'mt-1'">
      <slot />
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed, ref, useId } from 'vue';
import { ChevronDown, ChevronRight } from '@lucide/vue';

const props = withDefaults(defineProps<{ label: string; openLabel?: string; variant?: 'heading' | 'more' }>(), { variant: 'heading' });
const open = ref(false);
const panelId = useId();

const focusRing = 'outline-none focus-visible:outline-solid focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--nz-focus)]';
const triggerClass = computed(() =>
  props.variant === 'more'
    ? `mt-1 inline-flex h-7 items-center gap-1 rounded-[6px] text-[12.5px] font-medium text-primary hover:underline ${focusRing}`
    : `-ms-1 inline-flex h-7 cursor-pointer items-center gap-1 rounded-[6px] px-1 text-[12px] font-semibold text-foreground/65 hover:text-foreground ${focusRing}`,
);
</script>
