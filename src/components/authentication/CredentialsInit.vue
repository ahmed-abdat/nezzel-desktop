<template>
  <!-- Full-bleed tinted row (DESIGN: tint the row, don't nest a notice): neutral when off, red when enabling failed. -->
  <div
    role="alert"
    data-testid="auth-init-alert"
    class="flex items-start gap-3 rounded-[6px] px-3 py-3"
    :class="hasError ? 'bg-destructive-soft text-destructive-foreground' : 'bg-muted text-foreground'"
  >
    <CircleAlert v-if="hasError" class="mt-0.5 size-4 shrink-0" aria-hidden="true" />
    <KeyRound v-else class="mt-0.5 size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
    <div class="min-w-0 flex-1">
      <p class="text-nz-meta font-medium">{{ hasError ? t('auth.init.error.title') : t('auth.init.title') }}</p>
      <p class="mt-0.5 text-nz-caption text-pretty" :class="!hasError && 'text-muted-foreground'">
        {{ hasError ? t('auth.init.error.hint', { error: strongholdStore.status.initError ?? t('auth.init.error.unknown') }) : t('auth.init.description') }}
      </p>
    </div>
    <base-button
      :class="hasError ? 'btn-error' : 'btn-primary'"
      :loading="isInitializing"
      type="button"
      @click="initialize()"
    >
      {{ hasError ? t('common.retry') : t('common.enable') }}
    </base-button>
  </div>
</template>

<script setup lang="ts">
import { CircleAlert, KeyRound } from '@lucide/vue';
import { useStrongholdStore } from '../../stores/stronghold';
import { computed, ref } from 'vue';
import { useToastStore } from '../../stores/toast';
import BaseButton from '../base/BaseButton.vue';
import { useI18n } from 'vue-i18n';

const { t } = useI18n();
const strongholdStore = useStrongholdStore();
const toastStore = useToastStore();

const isInitializing = ref(false);

const hasError = computed(() => strongholdStore.status.initError != null);

const initialize = async () => {
  try {
    isInitializing.value = true;
    await strongholdStore.initialize();
  } catch (e) {
    const message = e instanceof Error ? e.message : 'Unknown error';
    toastStore.showToast(t('auth.init.toasts.error', { error: message }), {
      style: 'error',
      duration: 5000,
    });
  } finally {
    isInitializing.value = false;
  }
};
</script>
