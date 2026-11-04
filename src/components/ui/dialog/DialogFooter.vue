<script setup lang="ts">
import type { HTMLAttributes } from 'vue';
import { useI18n } from 'vue-i18n';
import { cn } from 'cn';
import { DialogClose } from 'reka-ui';
import { Button } from '@/components/ui/button';

const { t } = useI18n();
const props = withDefaults(defineProps<{
  class?: HTMLAttributes['class'];
  showCloseButton?: boolean;
}>(), {
  showCloseButton: false,
});
</script>

<template>
  <div
    data-slot="dialog-footer"
    :class="cn('flex flex-col-reverse gap-2 sm:flex-row sm:justify-end', props.class)"
  >
    <slot />
    <DialogClose v-if="showCloseButton" as-child>
      <Button variant="outline">{{ t('common.close') }}</Button>
    </DialogClose>
  </div>
</template>
