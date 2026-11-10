<template>
  <div class="flex min-w-0 flex-col gap-0.5" role="alert" data-testid="capture-fail">
    <!-- A login need is "needs you" (amber), not a failure (red). -->
    <p class="flex items-start gap-1.5 text-[12.5px] font-medium" :class="reason === 'login' ? 'text-amber-foreground' : 'text-destructive-foreground dark:text-destructive'">
      <KeyRound v-if="reason === 'login'" class="mt-0.5 size-3.5 shrink-0" aria-hidden="true" /><TriangleAlert v-else class="mt-0.5 size-3.5 shrink-0" aria-hidden="true" /><bdi>{{ t(`capture.fail.${reason}.title`, { site: site || t('capture.fail.thisSite') }) }}</bdi></p>
    <p class="ps-5 text-[12px] text-muted-foreground text-pretty">{{ t(`capture.fail.${reason}.hint`) }}</p>
  </div>
</template>

<script setup lang="ts">
import { KeyRound, TriangleAlert } from '@lucide/vue';
import { useI18n } from 'vue-i18n';

defineProps<{ reason: string; site: string }>();
const { t } = useI18n();
</script>
