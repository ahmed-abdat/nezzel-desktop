<template>
  <!-- "Advanced" disclosure at the end of a per-download tab: expert rows stay out of the way until asked for. -->
  <section class="mt-8">
    <button
      type="button"
      class="-ms-1 inline-flex h-7 items-center gap-1 rounded-[6px] px-1 text-[12px] font-semibold text-foreground/65 outline-none hover:text-foreground focus-visible:outline-solid focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--nz-focus)]"
      :aria-expanded="open"
      :aria-controls="panelId"
      @click="open = !open"
    >
      <ChevronRight class="size-3.5 transition-transform duration-(--nz-dur-fast) rtl:rotate-180" :class="open && 'rotate-90 rtl:rotate-90'" aria-hidden="true" />
      {{ title ?? t('perDownload.advanced') }}
    </button>
    <div v-show="open" :id="panelId" class="mt-1">
      <slot />
    </div>
  </section>
</template>

<script setup lang="ts">
import { ref, useId } from 'vue';
import { useI18n } from 'vue-i18n';
import { ChevronRight } from '@lucide/vue';

defineProps<{ title?: string }>();
const { t } = useI18n();
const open = ref(false);
const panelId = useId();
</script>
