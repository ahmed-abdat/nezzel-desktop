<template>
  <header
    role="toolbar"
    :aria-busy="busy || undefined"
    :aria-label="t('downloads.selection.bar')"
    class="sticky top-0 z-[1] flex flex-wrap items-center gap-1 border-b border-hairline bg-primary-soft px-4 py-1 text-[12px] backdrop-blur"
    @keydown.esc.stop="!busy && clear()"
  >
    <span class="me-2 font-semibold text-foreground" role="status" aria-live="polite">{{ t('downloads.selection.count', { n: countLabel(count) }, count) }}</span>
    <tooltip v-if="count < total" :content="t('downloads.selection.selectAllHint')">
      <Button :disabled="busy" variant="ghost" class="h-6 px-2" @click="selectAll">{{ t('downloads.selection.selectAll', { n: countLabel(total) }) }}</Button>
    </tooltip>
    <Button :disabled="busy" v-if="targets.download.length" variant="ghost" class="h-6 px-2 text-primary" @click="downloadAll">{{ t('downloads.selection.download') }}</Button>
    <Button :disabled="busy" v-if="targets.resume.length" variant="ghost" class="h-6 px-2" @click="resumeAll">{{ t('downloads.selection.resume') }}</Button>
    <Button :disabled="busy" v-if="targets.pause.length" variant="ghost" class="h-6 px-2" @click="pauseAll">{{ t('downloads.selection.pause') }}</Button>
    <Button :disabled="busy" v-if="targets.retry.length" variant="ghost" class="h-6 px-2" @click="retryAll">{{ t('downloads.selection.retry') }}</Button>
    <Button :disabled="busy" variant="ghost" class="h-6 px-2" @click="removeAll">{{ t('downloads.selection.remove') }}</Button>
    <Button :disabled="busy" v-if="targets.deleteFiles.length" variant="ghost" class="h-6 px-2 text-destructive-foreground" @click="deleteAll">{{ t('downloads.selection.deleteFiles') }}</Button>
    <span class="flex-1" />
    <tooltip :content="t('downloads.selection.clear')">
      <Button :disabled="busy" size="icon-xs" variant="ghost" class="size-6 text-muted-foreground" :aria-label="t('downloads.selection.clear')" @click="clear">
        <x aria-hidden="true" />
      </Button>
    </tooltip>
  </header>
</template>

<script setup lang="ts">
import Tooltip from '../ui/tooltip/Tooltip.vue';
import { computed, ref, nextTick } from 'vue';
import { useI18n } from 'vue-i18n';
import { confirm } from '@tauri-apps/plugin-dialog';
import { X } from '@lucide/vue';
import { Button } from '@/components/ui/button';
import type { Group } from '../../tauri/types/group';
import { useMediaStateStore, MediaState } from '../../stores/media/state';
import { useMediaDestinationStore } from '../../stores/media/destination';
import { useToastStore } from '../../stores/toast';
import { useListRemoval } from '../../composables/useListRemoval';
import { isTrashRefused, trashBinKind, trashGroupFile, useGroupActions } from '../../composables/useDownloadActions';
import { LIST_SEARCH_ID, ROW_SELECTOR } from '../../helpers/domIds';
import { phaseOf } from '../../helpers/groupView';
import { useDownloadSelection } from '../../composables/useDownloadSelection';
import { useLocaleNumber } from '../../composables/useLocaleNumber';
import { batchTargets } from '../../helpers/selection';

/** Batch actions for the selected rows. Every action runs the same per-row function a single row uses. */
const props = defineProps<{ groups: Group[] }>();
const { t } = useI18n();
const { count: countLabel } = useLocaleNumber();
const { count, total, selectAll, clear } = useDownloadSelection();
const stateStore = useMediaStateStore();
const destinationStore = useMediaDestinationStore();
const toastStore = useToastStore();
const { removeRows } = useListRemoval();
const busy = ref(false);

const targets = computed(() => batchTargets(props.groups.map(group => ({
  id: group.id,
  phase: phaseOf(stateStore.getGroupState(group.id) ?? MediaState.fetching),
  // A file the engine already refused (no receipt) can't be trashed: don't count it for Delete files.
  hasFile: !!destinationStore.getPrimaryDestination(group.id)?.path && !isTrashRefused(group.id),
}))));

const actions = useGroupActions({ reportErrors: false });

function forIds(ids: string[]): Group[] {
  const wanted = new Set(ids);
  return props.groups.filter(group => wanted.has(group.id));
}
async function batch(ids: string[], run: (group: Group) => Promise<boolean>, message: string) {
  if (busy.value) return;
  busy.value = true;
  try {
    const results = await Promise.allSettled(forIds(ids).map(run));
    const succeeded = results.filter(result => result.status === 'fulfilled' && result.value).length;
    const failed = results.length - succeeded;
    if (succeeded) toastStore.showToast(t(message, { n: countLabel(succeeded) }), { style: 'success' });
    if (failed) toastStore.showToast(t('feedback.batchFailed', { n: countLabel(failed) }, failed), { style: 'error' });
  } finally { busy.value = false; }
}

const downloadAll = () => batch(targets.value.download, actions.downloadGroup, 'feedback.started');
const resumeAll = () => batch(targets.value.resume, actions.downloadGroup, 'feedback.started');
const pauseAll = () => batch(targets.value.pause, actions.pauseGroup, 'feedback.paused');
const retryAll = () => batch(targets.value.retry, actions.retryGroup, 'feedback.retrying');
/** The bar (and the button that had focus) unmounts with the selection: keep the keyboard in the list. */
function refocusList() {
  void nextTick(() => (document.querySelector<HTMLElement>(ROW_SELECTOR) ?? document.getElementById(LIST_SEARCH_ID))?.focus());
}

function removeAll() {
  if (busy.value) return;
  removeRows(targets.value.remove);
  clear();
  refocusList();
}

async function deleteAll() {
  const ids = targets.value.deleteFiles;
  const n = ids.length;
  const bin = trashBinKind();
  const ok = await confirm(t(`downloads.selection.deleteConfirm.body.${bin}`, { n: countLabel(n) }, n), {
    title: t('downloads.selection.deleteConfirm.title', { n: countLabel(n) }, n),
    kind: 'warning',
    okLabel: t(`downloads.row.deleteConfirm.ok.${bin}`),
    cancelLabel: t('downloads.row.deleteConfirm.cancel'),
  });
  if (!ok) return;
  let failed = 0;
  for (const group of forIds(ids)) {
    const result = await trashGroupFile(group.id, destinationStore.getPrimaryDestination(group.id)?.path);
    if (result !== 'ok' && result !== 'missing') failed += 1;
  }
  if (failed) toastStore.showToast(t('downloads.selection.deleteSomeFailed', { n: countLabel(failed) }, failed), { style: 'error' });
  else toastStore.showToast(t(`downloads.row.deleted.${bin}`), { style: 'success' });
  clear();
  refocusList();
}
</script>
