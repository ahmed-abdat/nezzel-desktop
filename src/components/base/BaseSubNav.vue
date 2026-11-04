<template>
  <!-- Sub-page header, same as the download detail: Back, a short title, actions at the end. 48px, hairline below. -->
  <nav class="sticky top-0 z-10 flex min-h-12 items-center gap-2 border-b border-hairline bg-background/90 px-4 py-1.5 backdrop-blur">
    <Button v-if="backButton" variant="ghost" size="sm" class="-ms-2 shrink-0" @click="goBack">
      <ChevronLeft class="rtl:rotate-180" aria-hidden="true" />{{ t('common.back') }}
    </Button>
    <div class="flex min-w-0 flex-1 items-center [&_h1]:truncate [&_h1]:text-nz-meta [&_h1]:font-semibold [&_p]:text-nz-caption [&_p]:text-muted-foreground">
      <slot name="title" />
    </div>
    <div class="flex shrink-0 items-center gap-2"><slot /></div>
  </nav>
</template>

<script setup lang="ts">
import { ChevronLeft } from '@lucide/vue';
import { useI18n } from 'vue-i18n';
import { useRouter } from 'vue-router';
import { Button } from '../ui/button';

const { t } = useI18n();
const router = useRouter();

const { backButton } = defineProps({
  backButton: {
    type: Boolean,
    required: false,
    default: true,
  },
});

function goBack() {
  const state = (typeof window !== 'undefined' ? window.history.state : null) as { back?: string | null } | null;
  if (state?.back) {
    router.back();
  } else {
    void router.push({ name: 'home' });
  }
}
</script>
