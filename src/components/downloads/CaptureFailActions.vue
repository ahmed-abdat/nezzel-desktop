<template>
  <!-- Failed read: only what can work. Quiet actions first, the primary last (inline end), like the picks. -->
  <div class="flex items-center gap-2" data-testid="capture-fail-actions">
    <Button
      v-for="action in actions.secondary"
      :key="action"
      type="button"
      :variant="action === 'report' ? 'ghost' : 'outline'"
      size="sm"
      :class="action === 'report' && 'px-2 text-muted-foreground'"
      :disabled="busy"
      @click="emit('act', action)"
    >
      {{ label(action) }}
    </Button>
    <Button v-if="actions.primary" type="button" size="sm" :variant="reason === 'login' ? 'amber' : 'default'" :disabled="busy" @click="emit('act', actions.primary)">
      <RotateCw v-if="actions.primary === 'retry'" aria-hidden="true" />{{ label(actions.primary) }}
    </Button>
  </div>
</template>

<script setup lang="ts">
import { computed } from 'vue';
import { useI18n } from 'vue-i18n';
import { RotateCw } from '@lucide/vue';
import { Button } from '@/components/ui/button';
import { failActions, type FailAction, type FailReason } from '../../helpers/captureFailure';

const props = defineProps<{ reason: FailReason; busy: boolean }>();
const emit = defineEmits<{ act: [action: FailAction] }>();
const { t } = useI18n();
const actions = computed(() => failActions(props.reason));
const label = (action: FailAction) => action === 'options' ? t(props.reason === 'login' ? 'capture.fail.useLogin' : 'capture.options') : t(`capture.fail.${action}`);
</script>
