<template>
  <footer class="flex min-h-7 shrink-0 flex-wrap items-center gap-x-3 gap-y-1 border-t border-hairline bg-sidebar px-4 py-1 text-[11.5px] text-muted-foreground" :aria-label="t('shell.status.label')">
    <!-- Start: live state. One polite region so screen readers hear "2 downloading" without the speed ticking. -->
    <span class="inline-flex min-w-0 items-center gap-1.5" role="status">
      <span class="size-1.5 shrink-0 rounded-full" :class="dotCls" aria-hidden="true" />
      <span class="tnum truncate">{{ stateText }}</span>
      <template v-if="downloading.length && totalSpeed">
        <span class="nz-sep" aria-hidden="true" />
        <bdi dir="ltr" class="tnum" aria-hidden="true">{{ speed(totalSpeed) }}</bdi>
      </template>
      <template v-if="downloading.length && failed.length">
        <span class="nz-sep" aria-hidden="true" />
        <span class="tnum text-destructive">{{ t('shell.status.failed', { n: n(failed.length) }, failed.length) }}</span>
      </template>
    </span>
    <Button v-if="binaries.needsRepair" as-child variant="outline" size="sm">
      <RouterLink to="/install"><Wrench />{{ t('toolsRepair.action') }}</RouterLink>
    </Button>

    <!-- End: where files go. Click opens it in Finder/Explorer; the tooltip shows the full path. -->
    <tooltip v-if="downloadDir" :content="downloadDir" side="top">
      <button
        type="button"
        class="ms-auto inline-flex min-w-0 max-w-[50%] items-center gap-1.5 rounded px-1 outline-none transition-colors hover:text-foreground focus-visible:outline-solid focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--nz-focus)]"
        :aria-label="`${t('shell.status.openFolder')}: ${downloadDir}`"
        @click="openFolder"
      >
        <folder-open class="size-3.5 shrink-0" aria-hidden="true" />
        <bdi class="truncate" dir="ltr">{{ folderName }}</bdi>
      </button>
    </tooltip>
  </footer>
</template>

<script setup lang="ts">
import { computed } from 'vue';
import { useI18n } from 'vue-i18n';
import { FolderOpen, Wrench } from '@lucide/vue';
import { RouterLink } from 'vue-router';
import { Button } from '../ui/button';
import { useBinariesStore } from '../../stores/binaries';
import Tooltip from '@/components/ui/tooltip/Tooltip.vue';
import { useDownloadBuckets } from '../../composables/useDownloadBuckets';
import { useLocaleNumber } from '../../composables/useLocaleNumber';
import { useOpener } from '../../composables/useOpener';
import { useMediaProgressStore } from '../../stores/media/progress';
import { useToastStore } from '../../stores/toast';
import { useSettingsStore } from '../../stores/settings';
import { compactFolderPath, parentFolder } from '../../helpers/folderPath';

const { t } = useI18n();
const { count: n, speed } = useLocaleNumber();
const { downloading, failed, reading } = useDownloadBuckets();
const progressStore = useMediaProgressStore();
const settingsStore = useSettingsStore();
const binaries = useBinariesStore();
const { openPath } = useOpener();

const totalSpeed = computed(() => downloading.value.reduce((sum, group) => sum + (progressStore.findGroupProgress(group.id)?.speedBps ?? 0), 0));

const stateText = computed(() => {
  if (binaries.needsRepair) return t('toolsRepair.unavailable');
  if (downloading.value.length) return t('shell.status.downloading', { n: n(downloading.value.length) }, downloading.value.length);
  if (reading.value.length) return t('shell.status.reading', { n: n(reading.value.length) }, reading.value.length);
  if (failed.value.length) return t('shell.status.failed', { n: n(failed.value.length) }, failed.value.length);
  return t('shell.status.ready');
});
const dotCls = computed(() => {
  if (binaries.needsRepair) return 'bg-destructive';
  if (downloading.value.length) return 'bg-primary animate-pulse motion-reduce:animate-none';
  if (reading.value.length) return 'bg-primary/60 animate-pulse motion-reduce:animate-none';
  if (failed.value.length) return 'bg-destructive';
  return 'bg-primary/60';
});

const downloadDir = computed(() => settingsStore.settings.output?.downloadDir ?? null);
const folderName = computed(() => compactFolderPath(downloadDir.value ?? ''));

async function openFolder() {
  const dir = downloadDir.value;
  if (!dir) return;
  try {
    await openPath(dir);
  } catch {
    // The default Downloads/Nezzel folder is created by the first download; until then open its parent.
    const parent = parentFolder(dir);
    if (parent) {
      try {
        await openPath(parent);
        return;
      } catch { /* Both locations failed. */ }
    }
    useToastStore().showToast(t('feedback.folderFailed'), { style: 'error' });
  }
}
</script>
