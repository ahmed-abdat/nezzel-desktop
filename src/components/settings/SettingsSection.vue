<template>
  <section class="mt-9 first:mt-0" :aria-labelledby="title ? headingId : undefined">
    <h2 v-if="title" :id="headingId" class="mb-1 text-[13px] font-semibold text-foreground">{{ title }}</h2>
    <p v-if="note" class="mb-1 text-[12px] leading-relaxed text-muted-foreground text-pretty">{{ note }}</p>
    <div>
      <slot />
      <BaseDisclosure v-if="$slots.more" variant="more" :label="t('prefs.showMore')" :open-label="t('prefs.showLess')">
        <slot name="more" />
      </BaseDisclosure>
    </div>
  </section>
</template>

<script setup lang="ts">
// Flat section: small heading, then hairline-separated rows. No box, no card.
// `#more` holds rarely used rows behind a "Show more" link at the end of the section.
import { useId } from 'vue';
import { useI18n } from 'vue-i18n';
import BaseDisclosure from '../base/BaseDisclosure.vue';

defineProps<{ title?: string; note?: string }>();
const { t } = useI18n();
const headingId = useId();
</script>
