<template>
  <div ref="panel" class="flex h-full w-full flex-col bg-card text-foreground outline-none" role="dialog" :aria-modal="modal ? 'true' : undefined" :aria-labelledby="titleId" tabindex="-1">
    <header class="flex h-[52px] shrink-0 items-center justify-between border-b border-hairline ps-4 pe-2">
      <h2 :id="titleId" class="text-[13px] font-semibold">{{ t('sheet.title') }}</h2>
      <tooltip :content="t('sheet.close')">
        <Button size="icon-xs" variant="ghost" :aria-label="t('sheet.close')" @click="sheet.close()">
          <XMarkIcon aria-hidden="true" />
        </Button>
      </tooltip>
    </header>

    <div v-if="group" class="min-h-0 flex-1 overflow-y-auto">
      <div class="flex gap-3 border-b border-hairline p-4">
        <figure class="aspect-video w-24 shrink-0 overflow-hidden rounded-md bg-muted ring-1 ring-black/5 dark:ring-white/5" :class="isReading && !group.thumbnail && 'animate-pulse'">
          <file-type-icon v-if="fileKind" :category="fileKind" class="size-full p-4 text-muted-foreground" />
          <img v-else-if="thumbUrl" :src="thumbUrl" alt="" class="size-full object-cover" @error="thumbFailed = true">
          <div v-else-if="!isReading" class="grid size-full place-items-center text-muted-foreground"><film class="size-5" aria-hidden="true" /></div>
        </figure>
        <div class="min-w-0">
          <Tooltip :content="title" side="bottom" :disabled="!titleClipped">
            <p class="line-clamp-2 text-start text-[13px] font-medium leading-snug [overflow-wrap:anywhere]" @pointerenter="measureTitle"><bdi dir="auto">{{ title }}</bdi></p>
          </Tooltip>
          <p class="mt-1 flex flex-wrap items-center gap-x-1.5 text-[12px] text-muted-foreground">
            <span v-for="(part, index) in metaParts" :key="index" class="inline-flex items-center gap-x-1.5 whitespace-nowrap">
              <span v-if="index > 0" class="nz-sep" aria-hidden="true" />
              <bdi>{{ part }}</bdi>
            </span>
          </p>
          <LoginProfile :group-id="group.id" :disabled="loginBusy" class="mt-1" />
          <p v-if="sizeLabel && !isFile" class="mt-0.5 text-[12px] text-muted-foreground">{{ t('sheet.size', { size: sizeLabel }) }}</p>
        </div>
      </div>

      <dl v-if="fileDetails" class="grid grid-cols-[auto_1fr] gap-x-4 gap-y-2 border-b border-hairline px-4 py-3 text-[12.5px]">
        <template v-for="row in fileDetails" :key="row.key">
          <dt class="text-muted-foreground">{{ row.label }}</dt>
          <dd class="min-w-0 truncate font-medium" :title="row.value"><bdi :dir="row.ltr ? 'ltr' : undefined">{{ row.value }}</bdi></dd>
        </template>
      </dl>

      <MediaAttachmentPicker v-if="(group.attachments?.length ?? 0) > 1" :group="group" />

      <h3 v-if="hasFormatControls" :id="formatId" class="px-4 pb-1.5 pt-4 text-[11.5px] font-semibold text-muted-foreground">{{ t('sheet.format') }}</h3>
      <RadioGroup v-if="hasFormatControls" class="pb-2" :aria-labelledby="formatId" :model-value="choice" @update:model-value="setChoice($event as QuickChoice)">
        <RadioGroupItem v-for="tile in tiles" :key="tile.value" :value="tile.value" :disabled="tile.disabled" class="px-4 py-2 hover:bg-accent/70">
          <span class="min-w-0 flex-1">
            <span class="block text-[13px] font-medium">{{ tile.label }}</span>
            <span class="block text-[12px] text-muted-foreground">{{ tile.hint }}</span>
          </span>
        </RadioGroupItem>
      </RadioGroup>

      <div v-if="hasFormatControls" class="border-t border-hairline">
        <button
          type="button"
          class="flex w-full items-center gap-1.5 px-4 py-3 text-start text-[13px] font-medium outline-none hover:bg-accent/70 focus-visible:bg-accent focus-visible:outline-solid focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-[var(--nz-focus)]"
          :aria-expanded="advancedOpen"
          :aria-controls="advancedId"
          @click="advancedOpen = !advancedOpen; sheet.touch()"
        >
          <ChevronDownIcon class="size-4 text-muted-foreground transition-transform duration-150" :class="advancedOpen && 'rotate-180'" aria-hidden="true" />
          <span>{{ t('sheet.advanced') }}</span>
        </button>
        <div v-if="advancedOpen" :id="advancedId" class="flex flex-col gap-2 px-4 pb-3">
          <p class="text-[12px] text-muted-foreground">{{ t('sheet.advancedHint') }}</p>
          <media-download-options
            v-if="!isReading"
            :key="group.id"
            v-model="advancedOptions"
            :formats="group.formats"
            :default-value="currentOptions"
            class="flex flex-col gap-2 [&_select]:h-7! [&_select]:w-full [&_select]:rounded-[6px]! [&_select]:border! [&_select]:border-input! [&_select]:bg-card! [&_select]:font-sans! [&_select]:text-[13px]! [&_select]:shadow-xs! [&_select]:outline-none! [&_select:focus]:ring-2 [&_select:focus]:ring-ring/30"
            approximate
          />
          <router-link class="self-start text-[12.5px] font-medium text-primary hover:underline" :to="{ name: 'preferences.quality', params: { groupId: group.id } }">
            {{ t('sheet.moreOptions') }}
          </router-link>
        </div>
      </div>

      <div class="flex items-center gap-2 border-t border-hairline px-4 py-3 text-[12.5px] text-muted-foreground">
        <span class="shrink-0">{{ t('sheet.saveTo') }}</span>
        <bdi class="min-w-0 flex-1 truncate font-medium text-foreground" :dir="folder ? 'ltr' : undefined" :title="folder">{{ folder ? shortFolderPath(folder) : t('prefs.general.folderDefault') }}</bdi>
        <Button size="xs" variant="ghost" class="ms-auto shrink-0 text-primary" @click="pickFolder">{{ t('prefs.general.change') }}</Button>
      </div>
    </div>
    <div v-else class="flex-1" />

    <footer class="shrink-0 border-t border-hairline p-4">
      <div v-if="hasFormatControls" class="mb-3 flex items-start gap-2 text-[12.5px] text-muted-foreground">
        <Checkbox :id="rememberId" v-model="remember" class="mt-0.5" @update:model-value="sheet.touch()" />
        <label :for="rememberId" class="cursor-pointer">{{ t('sheet.remember') }}</label>
      </div>
      <div class="flex items-center justify-end gap-2">
        <p v-if="isReading" class="me-auto text-[12px] text-muted-foreground" role="status">{{ t('sheet.waitingFormats') }}</p>
        <Button variant="outline" @click="sheet.close()">{{ t('sheet.cancel') }}</Button>
        <Button class="px-4" :disabled="!canDownload" @click="confirm">{{ t('sheet.download') }}</Button>
      </div>
    </footer>
  </div>
</template>

<script setup lang="ts">
import LoginProfile from './LoginProfile.vue';
import FileTypeIcon from './FileTypeIcon.vue';
import { groupFileKind, groupSite, isImageOnly as isImageOnlyGroup } from '../../helpers/groupView';
import { hostOf } from '../../helpers/siteName';
import { useMediaDestinationStore } from '../../stores/media/destination';
// Flat inspector for one download's options. Renders only its panel; the shell decides where it sits.
// The choice is never kept in local state: it is read from the options stored on the group, so reopening
// always shows what was stored (the old sheet kept a separate `choice` ref and rewrote the store from it).
import { computed, nextTick, onBeforeUnmount, onMounted, ref, useId, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { ChevronDownIcon } from '@heroicons/vue/20/solid';
import { XMarkIcon } from '@heroicons/vue/24/outline';
import { Button } from '../ui/button';
import Tooltip from '../ui/tooltip/Tooltip.vue';
import { Film } from '@lucide/vue';
import { useToastStore } from '../../stores/toast';
import { useOverflowTip } from '../../composables/useOverflowTip';
import Checkbox from '../ui/checkbox/Checkbox.vue';
import { RadioGroup, RadioGroupItem } from '../ui/radio-group';
import MediaDownloadOptions from '../media-card/MediaDownloadOptions.vue';
import MediaAttachmentPicker from '../media-card/MediaAttachmentPicker.vue';
import { useDownloadSheetStore } from '../../stores/downloadSheet';
import { useMediaGroupStore } from '../../stores/media/group';
import { MediaState, useMediaStateStore } from '../../stores/media/state';
import { useMediaOptionsStore } from '../../stores/media/options';
import { useMediaStore } from '../../stores/media/media';
import { useMediaSizeStore } from '../../stores/media/size';
import { useSettingsStore } from '../../stores/settings';
import { remoteImagesAllowed } from '../../helpers/remoteImages';
import { choiceForOptions, choiceHeight, hasAudio, optionsForChoice, type QuickChoice } from '../../helpers/quickChoice';
import { formatDuration } from '../../helpers/units';
import { useUiPrefs } from '../../composables/useUiPrefs';
import { useDialog } from '../../composables/useDialog';
import { shortFolderPath } from '../../helpers/folderPath';
import { useLocaleNumber } from '../../composables/useLocaleNumber';
import type { DownloadOptions } from '../../tauri/types/media';
import { FILES_FOLDER } from '@/generated/contract';

defineProps<{ modal?: boolean }>();

const { t } = useI18n();
const { openPathDialog } = useDialog();
const sheet = useDownloadSheetStore();
const groupStore = useMediaGroupStore();
const stateStore = useMediaStateStore();
const optionsStore = useMediaOptionsStore();
const mediaStore = useMediaStore();
const sizeStore = useMediaSizeStore();
const settingsStore = useSettingsStore();
const { skipOptionsSheet, defaultChoice } = useUiPrefs();
const { size: formatSizeLocal } = useLocaleNumber();

const titleId = useId();
const formatId = useId();
const advancedId = useId();
const rememberId = useId();
const panel = ref<HTMLElement | null>(null);

const group = computed(() => (sheet.groupId ? groupStore.findGroupById(sheet.groupId) : undefined));
const { clipped: titleClipped, measure: measureTitle } = useOverflowTip();
const thumbFailed = ref(false);
const thumbUrl = computed(() => (!thumbFailed.value && remoteImagesAllowed(settingsStore.settings.network, group.value?.thumbnail) && group.value?.thumbnail) || null);
watch(() => group.value?.thumbnail, () => {
  thumbFailed.value = false;
});
const state = computed(() => (group.value ? stateStore.getGroupState(group.value.id) : undefined));
const isReading = computed(() => state.value === MediaState.fetching || state.value === MediaState.fetchingList);
const loginBusy = computed(() => isReading.value || state.value === MediaState.downloading || state.value === MediaState.downloadingList);
const formats = computed(() => group.value?.formats ?? []);
const isImageOnly = computed(() => isImageOnlyGroup(group.value));
const isFile = computed(() => group.value?.mediaKind === 'file');
const savedPath = computed(() => (group.value ? useMediaDestinationStore().getPrimaryDestination(group.value.id)?.path : undefined));
const fileKind = computed(() => groupFileKind(group.value, savedPath.value));
/** File downloads: type, size, where it was saved, and the source host (never the full link). */
const fileDetails = computed(() => {
  if (!isFile.value || !group.value || !fileKind.value) return undefined;
  const host = hostOf(group.value.url);
  return [
    { key: 'type', label: t('files.type'), value: t(`files.category.${fileKind.value}`), ltr: false },
    group.value.filesize ? { key: 'size', label: t('files.size'), value: formatSizeLocal(group.value.filesize), ltr: false } : undefined,
    savedPath.value ? { key: 'path', label: t('files.savedTo'), value: savedPath.value, ltr: true } : undefined,
    host ? { key: 'source', label: t('files.source'), value: host, ltr: true } : undefined,
  ].filter((row): row is NonNullable<typeof row> => !!row);
});
const hasFormatControls = computed(() => !isImageOnly.value && !isFile.value);
const hasAttachments = computed(() => !!group.value?.attachments?.length);
const title = computed(() => group.value?.title ?? group.value?.url ?? '');
const duration = computed(() => formatDuration(group.value?.duration ?? 0, { pad: true }));
const site = computed(() => (group.value ? groupSite(group.value) : undefined));
/** This download's folder: the one picked in the sheet, else the app default. */
const folder = computed(() => {
  const picked = group.value && optionsStore.getOverrides(group.value.id)?.output?.downloadDir;
  if (picked) return picked;
  const base = settingsStore.settings.output.downloadDir || '';
  return isFile.value && base ? `${base.replace(/[\\/]$/, '')}${base.includes('\\') ? '\\' : '/'}${FILES_FOLDER}` : base;
});

/** Native folder picker for this download only; the sheet stays open. */
async function pickFolder() {
  if (!group.value) return;
  let picked: unknown;
  try {
    picked = await openPathDialog(false, true);
  } catch {
    useToastStore().showToast(t('feedback.folderPickFailed'), { style: 'error' });
    return;
  }
  if (typeof picked !== 'string' || picked.length === 0) return;
  const existing = optionsStore.getOverrides(group.value.id) ?? {};
  optionsStore.setOverrides(group.value.id, { ...existing, output: { ...(existing.output ?? {}), downloadDir: picked } });
}

const currentOptions = computed(() => (group.value ? optionsStore.getOptions(group.value.id) : undefined));
/** A choice made while the link is still being read; applied once the formats arrive. */
const pending = ref<QuickChoice>();
const remember = ref(false);
const advancedOpen = ref(false);

/** The shown choice: derived from the stored options; undefined = a custom format from Advanced. */
const choice = computed<QuickChoice | undefined>(() => {
  const stored = currentOptions.value;
  if (!stored) {
    const preferred = pending.value ?? defaultChoice.value;
    if (isReading.value) return preferred === 'audio' ? 'best' : preferred;
    const resolved = choiceForOptions(optionsForChoice(preferred, formats.value, group.value?.mediaKind), formats.value, group.value?.mediaKind);
    return resolved === 'custom' ? undefined : resolved;
  }
  const read = choiceForOptions(stored, formats.value, group.value?.mediaKind);
  return read === 'custom' ? undefined : read;
});

function setChoice(value: QuickChoice) {
  sheet.touch();
  const g = group.value;
  if (!g || !value) return;
  if (isReading.value) {
    pending.value = value;
    return;
  }
  optionsStore.setOptions(g.id, optionsForChoice(value, formats.value, g.mediaKind));
}

// The inspector only makes sense while the link is being read or is ready; anything else closes it.
watch([group, state], ([g, s]) => {
  if (!sheet.open) return;
  if (!g || (s && s !== MediaState.fetching && s !== MediaState.fetchingList && s !== MediaState.configure)) {
    sheet.close();
  }
});

// Opening (or switching to another link) starts from what is stored; nothing is written here.
let returnFocus: HTMLElement | null = null;
watch(() => [sheet.open, sheet.groupId] as const, async ([open], previous) => {
  if (!open) {
    if (previous?.[0]) returnFocus?.focus?.();
    return;
  }
  if (!previous?.[0]) returnFocus = document.activeElement as HTMLElement | null;
  remember.value = false;
  pending.value = undefined;
  const stored = currentOptions.value;
  advancedOpen.value = !!stored && choiceForOptions(stored, formats.value, group.value?.mediaKind) === 'custom';
  await nextTick();
  const target = panel.value?.querySelector<HTMLElement>('[role="radio"][aria-checked="true"]') ?? panel.value;
  target?.focus();
}, { immediate: true });

// First time the formats are known and nothing is stored yet: store the default (or the pending) choice.
watch([() => sheet.open, group, isReading, formats], () => {
  const g = group.value;
  if (!sheet.open || !g || isReading.value || !hasFormatControls.value || currentOptions.value) return;
  optionsStore.setOptions(g.id, optionsForChoice(pending.value ?? defaultChoice.value, formats.value, g.mediaKind));
  pending.value = undefined;
}, { immediate: true });

const advancedOptions = computed<DownloadOptions | undefined>({
  get: () => currentOptions.value,
  set: (value) => {
    sheet.touch();
    // The picker emits a bare track type while it has no format yet; that is not a choice.
    if (!group.value || !value || (!value.id && !value.height && !value.abr)) return;
    const current = currentOptions.value;
    if (current && JSON.stringify(current) === JSON.stringify(value)) return;
    optionsStore.setOptions(group.value.id, value);
  },
});

const tiles = computed(() => {
  const best = choiceHeight('best', formats.value);
  const small = choiceHeight('small', formats.value);
  const audioOnly = !isReading.value && !!group.value && hasAudio(group.value) && formats.value.length > 0 && best === undefined
    && group.value?.mediaKind !== 'video' && group.value?.mediaKind !== 'mixed';
  return [
    { value: 'best', label: t('sheet.best'), hint: best ? t('sheet.bestHint', { height: best }) : t('sheet.bestHintPlain'), disabled: audioOnly },
    ...(group.value && hasAudio(group.value) ? [{ value: 'audio', label: t('sheet.audio'), hint: t('sheet.audioHint'), disabled: false }] : []),
    { value: 'small', label: t('sheet.small'), hint: small ? t('sheet.smallHint', { height: small }) : t('sheet.bestHintPlain'), disabled: audioOnly || (small !== undefined && small === best) },
  ];
});

const sizeLabel = computed(() => {
  if (isFile.value) return group.value?.filesize ? formatSizeLocal(group.value.filesize) : undefined;
  if (!group.value || isImageOnly.value || !currentOptions.value || group.value.isCombined) return undefined;
  const size = sizeStore.getSizeForGroup(group.value.id, currentOptions.value)?.size;
  return size ? formatSizeLocal(size) : undefined;
});

watch(currentOptions, (options) => {
  const g = group.value;
  if (!sheet.open || !g || hasAttachments.value || isFile.value || !options || g.isCombined || !settingsStore.settings.performance.autoLoadSize) return;
  const existing = sizeStore.getSizeForGroup(g.id, options);
  if (existing) return;
  const firstItem = g.items[Object.keys(g.items)[0]];
  if (firstItem) void sizeStore.requestSize(g.url ?? '', g.id, firstItem.id, options);
});

const metaParts = computed(() => [
  site.value,
  isReading.value ? t('sheet.reading') : (group.value?.duration ? duration.value : undefined),
].filter((part): part is string => !!part));

const canDownload = computed(() => !!group.value && state.value === MediaState.configure
  && (!(hasAttachments.value || isImageOnly.value || group.value.mediaKind === 'mixed')
    || optionsStore.getAttachmentSelection(group.value.id).length > 0));

function onKeydown(event: KeyboardEvent) {
  if (event.key !== 'Escape' || event.defaultPrevented || !sheet.open) return;
  sheet.close();
}
onMounted(() => document.addEventListener('keydown', onKeydown));
onBeforeUnmount(() => document.removeEventListener('keydown', onKeydown));

function confirm() {
  const g = group.value;
  if (!g || !canDownload.value) return;
  const options = currentOptions.value ?? optionsForChoice(choice.value ?? 'best', formats.value, g.mediaKind);
  optionsStore.setOptions(g.id, options);
  if (remember.value) {
    skipOptionsSheet.value = true;
    if (choice.value) defaultChoice.value = choice.value;
  }
  sheet.close();
  void mediaStore.downloadGroup(g.id, options);
}
</script>
