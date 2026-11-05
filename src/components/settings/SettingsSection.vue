<template>
  <section class="mt-9 first:mt-0" :aria-labelledby="title ? headingId : undefined">
    <h2 v-if="title" :id="headingId" class="mb-1 text-[13px] font-semibold text-foreground">{{ title }}</h2>
    <p v-if="note" class="mb-1 text-[12px] leading-relaxed text-muted-foreground text-pretty">{{ note }}</p>
    <div>
      <slot />
      <div v-if="$slots.more" v-show="open" :id="moreId" class="border-t border-foreground/[0.08]"><slot name="more" /></div>
    </div>
    <template v-if="$slots.more">
      <button
        type="button"
        class="mt-1 inline-flex h-7 items-center gap-1 rounded-[6px] text-[12.5px] font-medium text-primary outline-none hover:underline focus-visible:outline-solid focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--nz-focus)]"
        :aria-expanded="open"
        :aria-controls="moreId"
        @click="open = !open"
      >
        {{ open ? t('prefs.showLess') : t('prefs.showMore') }}
        <ChevronDown class="size-3.5 transition-transform duration-150" :class="open && 'rotate-180'" aria-hidden="true" />
      </button>
    </template>
  </section>
</template>

<script setup lang="ts">
// Flat section: small heading, then hairline-separated rows. No box, no card.
// `#more` holds rarely used rows behind a "Show more" link at the end of the section.
import { ref, useId } from 'vue';
import { useI18n } from 'vue-i18n';
import { ChevronDown } from '@lucide/vue';

defineProps<{ title?: string; note?: string }>();
const { t } = useI18n();
const headingId = useId();
const moreId = useId();
const open = ref(false);
</script>
