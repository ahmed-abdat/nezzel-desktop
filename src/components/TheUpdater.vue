<template>
  <div class="fixed bottom-30 end-0 z-40 pointer-events-none">
    <Transition
        appear
        enter-active-class="transition duration-250 ease-out"
        enter-from-class="translate-y-4 opacity-0"
        enter-to-class="translate-y-0 opacity-100"
        leave-active-class="transition duration-200 ease-in"
        leave-from-class="translate-y-0 opacity-100"
        leave-to-class="translate-y-4 opacity-0"
    >
      <div
          v-if="!isIgnored && checkResult?.available"
          role="alert"
          data-testid="updater-alert"
          class="m-4 flex w-80 max-w-[calc(100vw-2rem)] items-start gap-3 rounded-xl border border-hairline bg-card p-3 text-foreground shadow-lg pointer-events-auto will-change-transform"
      >
        <span class="mt-0.5 grid size-7 shrink-0 place-items-center rounded-full bg-primary-soft text-primary-soft-foreground" aria-hidden="true">
          <RefreshCw class="size-4" :class="{ 'animate-spin motion-reduce:animate-none': isUpdating }" />
        </span>
        <div class="min-w-0 flex-1">
          <template v-if="lastError">
            <h3 class="text-[13px] font-medium leading-5 text-destructive-foreground">{{ t(lastOperation === 'install' ? 'feedback.updateInstallFailed' : 'feedback.updateDownloadFailed') }}</h3>
            <div class="mt-2 flex justify-end gap-2">
              <Button variant="ghost" @click="updaterStore.ignore()">{{ t('common.later') }}</Button>
              <Button @click="updaterStore.retry()">{{ t('common.retry') }}</Button>
            </div>
          </template>
          <template v-else-if="!isUpdating && !isNeedingRestart">
            <h3 class="text-[13px] font-medium leading-7">{{ t('updater.available') }}</h3>
            <div class="mt-2 flex justify-end gap-2">
              <Button variant="ghost" @click="updaterStore.ignore()">{{ t('common.later') }}</Button>
              <Button @click="updaterStore.download()">{{ t('common.download') }}</Button>
            </div>
          </template>
          <template v-else-if="isNeedingRestart && !isUpdating">
            <h3 class="text-[13px] font-medium leading-7">{{ t('updater.install.title') }}</h3>
            <p class="text-[12px] text-muted-foreground">{{ t('updater.install.subtitle') }}</p>
            <div class="mt-2 flex justify-end gap-2">
              <Button variant="ghost" @click="updaterStore.ignore()">{{ t('common.later') }}</Button>
              <Button @click="updaterStore.install()">{{ t('common.install') }}</Button>
            </div>
          </template>
          <template v-else>
            <h3 class="text-[13px] font-medium leading-7">{{ t(lastOperation === 'install' ? 'common.pleaseWait' : 'updater.downloading') }}</h3>
            <div v-if="lastOperation === 'download'" class="flex items-center gap-2">
              <div
                  class="flex-1"
                  role="progressbar"
                  aria-valuemin="0"
                  aria-valuemax="100"
                  :aria-valuenow="progress == null ? undefined : Math.round(progress)"
                  :aria-label="t('updater.downloading')"
              >
                <Progress :value="progress" />
              </div>
              <p v-if="progress != null" class="shrink-0 text-[12px] text-muted-foreground">{{ t('common.percentage', { percent: progress.toFixed() }) }}</p>
            </div>
          </template>
        </div>
      </div>
    </Transition>
  </div>
</template>

<script setup lang="ts">
import { RefreshCw } from '@lucide/vue';
import { Button } from '@/components/ui/button';
import { Progress } from '@/components/ui/progress';
import { useUpdaterStore } from '../stores/updater';
import { computed } from 'vue';
import { storeToRefs } from 'pinia';
import { useI18n } from 'vue-i18n';

const { t } = useI18n();
const updaterStore = useUpdaterStore();
const { checkResult, downloadProgress, isIgnored, isUpdating, isNeedingRestart, lastError, lastOperation } = storeToRefs(updaterStore);

/** 0–100, or null (indeterminate) until the server says how big the update is. */
const progress = computed<number | null>(() => {
  const received = downloadProgress.value.received ?? 0;
  const total = downloadProgress.value.total ?? 0;
  if (total <= 0) return null;
  return Math.min(100, (received / total) * 100);
});
</script>
