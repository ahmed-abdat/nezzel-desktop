<template>
  <div>
    <settings-section>
      <settings-row :label="t('prefs.general.folder')">
        <bdi class="min-w-0 max-w-56 truncate text-nz-meta text-muted-foreground" dir="ltr" :title="downloadDir || globalFolder">
          {{ folderLabel }}
        </bdi>
        <Button variant="outline" size="xs" @click="pickFolder">{{ t('prefs.general.change') }}</Button>
      </settings-row>
      <settings-row for-id="override-video-format" :label="t('prefs.adv.files.videoFormat')" :help="t('prefs.adv.files.videoFormatHint')">
        <select-field id="override-video-format" v-model="outputState.video.container" :options="videoContainerOptions" />
      </settings-row>
      <settings-row for-id="override-audio-format" :label="t('prefs.adv.files.audioFormat')" :help="t('prefs.adv.files.audioFormatHint')">
        <select-field id="override-audio-format" v-model="outputState.audio.format" :options="audioFormatOptions" />
      </settings-row>
      <settings-row for-id="override-add-thumbnail" :label="t('prefs.adv.files.cover')" :help="t('prefs.adv.files.coverHint')">
        <Switch id="override-add-thumbnail" v-model="outputState.addThumbnail" />
      </settings-row>
      <settings-row for-id="override-add-metadata" :label="t('prefs.adv.files.metadata')" :help="t('prefs.adv.files.metadataHint')">
        <Switch id="override-add-metadata" v-model="outputState.addMetadata" />
      </settings-row>
    </settings-section>

    <settings-section :title="t('perDownload.output.partTitle')">
      <base-partial-download-selector
        v-if="currentSelection"
        v-model="currentSelection"
        :chapters="chapters"
        :duration-seconds="durationSeconds"
      />
    </settings-section>

    <prefs-advanced>
      <settings-row for-id="override-save-thumbnail" :label="t('prefs.adv.files.saveCover')" :help="t('prefs.adv.files.saveCoverHint')">
        <Switch id="override-save-thumbnail" v-model="outputState.saveThumbnail" />
      </settings-row>
      <settings-row stacked for-id="video-file-template" :label="t('perDownload.output.fileName')" :help="t('perDownload.output.fileNameHint')">
        <Input id="video-file-template" v-model="outputState.fileNameTemplate" type="text" dir="ltr" spellcheck="false" autocomplete="off" class="font-mono text-[12px]" />
      </settings-row>
      <settings-row stacked for-id="audio-file-template" :label="t('perDownload.output.audioFileName')">
        <Input id="audio-file-template" v-model="outputState.audioFileNameTemplate" type="text" dir="ltr" spellcheck="false" autocomplete="off" class="font-mono text-[12px]" />
      </settings-row>
      <settings-row v-if="isPlaylistGroup" for-id="override-reverse-playlist-numbering" :label="t('perDownload.output.reverse')" :help="t('perDownload.output.reverseHint')">
        <Switch id="override-reverse-playlist-numbering" v-model="outputState.reversePlaylistNumbering" />
      </settings-row>
      <settings-row for-id="override-restrict-filenames" :label="t('perDownload.output.safeNames')" :help="t('perDownload.output.safeNamesHint')">
        <Switch id="override-restrict-filenames" v-model="outputState.restrictFilenames" />
      </settings-row>
      <settings-row for-id="override-precise-cuts" :label="t('perDownload.output.precise')" :help="t('perDownload.output.preciseHint')">
        <Switch id="override-precise-cuts" v-model="outputState.preciseCuts" />
      </settings-row>
      <settings-row for-id="override-keep-video" :label="t('prefs.adv.files.keepVideo')" :help="t('prefs.adv.files.keepVideoHint')">
        <Switch id="override-keep-video" v-model="keepOriginalStreamsVideo" />
      </settings-row>
      <settings-row for-id="override-keep-audio" :label="t('prefs.adv.files.keepAudio')" :help="t('prefs.adv.files.keepAudioHint')">
        <Switch id="override-keep-audio" v-model="keepOriginalStreamsAudio" />
      </settings-row>
      <settings-row for-id="override-video-after" :label="t('prefs.adv.files.videoAfter')" :help="t('prefs.adv.files.videoAfterHint')">
        <select-field id="override-video-after" v-model="outputState.video.postprocessPreset" :options="videoPresetOptions" />
      </settings-row>
      <p v-if="showMp42ContainerConflict" class="pb-3 text-nz-caption text-destructive" role="alert">{{ t('settings.output.postprocess.mp42RequiresMp4') }}</p>
      <p v-else-if="showKeepOriginalStreamsConflict" class="pb-3 text-nz-caption text-destructive" role="alert">{{ t('prefs.adv.files.keepConflict') }}</p>
      <p v-else-if="showReencodeWarning" class="pb-3 text-nz-caption text-amber-foreground" role="status">{{ t('prefs.adv.files.slowWarning') }}</p>
      <settings-row v-if="showVideoPostprocessMode" for-id="override-video-mode" :label="t('prefs.adv.files.customMode')" :help="t('prefs.adv.files.customModeHint')">
        <select-field id="override-video-mode" v-model="outputState.video.customPostprocessMode" :options="videoModeOptions" />
      </settings-row>
      <settings-row v-if="showVideoPostprocessArgs" stacked for-id="override-video-args" :label="t('prefs.adv.files.ffmpegArgs')" :help="t('prefs.adv.files.ffmpegArgsHint')">
        <Textarea id="override-video-args" v-model="outputState.video.postprocessArgs" dir="ltr" rows="2" class="py-1.5 font-mono text-[12px]" />
      </settings-row>
      <settings-row for-id="override-audio-after" :label="t('prefs.adv.files.audioAfter')" :help="t('prefs.adv.files.audioAfterHint')">
        <select-field id="override-audio-after" v-model="outputState.audio.postprocessPreset" :options="audioPresetOptions" />
      </settings-row>
      <settings-row v-if="showAudioPostprocessArgs" stacked for-id="override-audio-args" :label="t('prefs.adv.files.ffmpegArgs')" :help="t('prefs.adv.files.ffmpegArgsHint')">
        <Textarea id="override-audio-args" v-model="outputState.audio.postprocessArgs" dir="ltr" rows="2" class="py-1.5 font-mono text-[12px]" />
      </settings-row>
    </prefs-advanced>
  </div>
</template>

<script setup lang="ts">
import { Input } from '../ui/input';
import { Textarea } from '../ui/textarea';
import { computed, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { v4 as uuidv4 } from 'uuid';
import BasePartialDownloadSelector from '../base/BasePartialDownloadSelector.vue';
import { Button } from '../ui/button';
import { SelectField } from '../ui/select';
import Switch from '../ui/switch/Switch.vue';
import SettingsRow from '../settings/SettingsRow.vue';
import SettingsSection from '../settings/SettingsSection.vue';
import PrefsAdvanced from './PrefsAdvanced.vue';
import { useOutputSettingsEditor } from '../../composables/useOutputSettingsEditor.ts';
import { useDialog } from '../../composables/useDialog.ts';
import { shortFolderPath } from '../../helpers/folderPath.ts';
import {
  buildPartialDownloadOverride,
  createDefaultSelection,
  createSelectionFromSection,
  type PartialDownloadSelection,
} from '../../helpers/partialDownload.ts';
import { useMediaGroupStore } from '../../stores/media/group.ts';
import { useMediaOptionsStore } from '../../stores/media/options.ts';
import { useSettingsStore } from '../../stores/settings.ts';
import {
  AudioFormat,
  AudioPostprocessPreset,
  DownloadOverrides,
  VideoContainer,
  VideoPostprocessMode,
  VideoPostprocessPreset,
} from '../../tauri/types/media.ts';
import { type OutputSettings } from '../../tauri/types/config.ts';

const props = defineProps({
  groupId: {
    type: String,
    required: true,
  },
});

const { t } = useI18n();
const settingsStore = useSettingsStore();
const optionsStore = useMediaOptionsStore();
const groupStore = useMediaGroupStore();

const outputState = ref<OutputSettings>({
  ...settingsStore.settings.output,
  video: { ...settingsStore.settings.output.video },
  audio: { ...settingsStore.settings.output.audio },
});
const currentSelection = ref<PartialDownloadSelection>();
/** Folder picked for this download only (here or in the options sheet); empty = the app default. */
const downloadDir = ref('');

const {
  keepOriginalStreamsVideo,
  keepOriginalStreamsAudio,
  showVideoPostprocessArgs,
  showVideoPostprocessMode,
  showAudioPostprocessArgs,
  showReencodeWarning,
  showKeepOriginalStreamsConflict,
  showMp42ContainerConflict,
} = useOutputSettingsEditor(outputState);

const optionsOf = <T extends string>(values: T[], key: (value: T) => string) =>
  values.map(value => ({ value, label: t(key(value)) }));
const videoContainerOptions = computed(() => optionsOf(Object.values(VideoContainer), v => `settings.output.tabs.video.container.options.${v}`));
const audioFormatOptions = computed(() => optionsOf(Object.values(AudioFormat), v => `settings.output.tabs.audio.format.options.${v}`));
const videoPresetOptions = computed(() => optionsOf(Object.values(VideoPostprocessPreset), v => `prefs.adv.files.videoAfterOptions.${v}`));
const audioPresetOptions = computed(() => optionsOf(Object.values(AudioPostprocessPreset), v => `prefs.adv.files.videoAfterOptions.${v}`));
const videoModeOptions = computed(() => optionsOf(Object.values(VideoPostprocessMode), v => `prefs.adv.files.customModeOptions.${v}`));

const { openPathDialog } = useDialog();
const globalFolder = computed(() => settingsStore.settings.output.downloadDir ?? '');
const folderLabel = computed(() => {
  const folder = downloadDir.value || globalFolder.value;
  return folder ? shortFolderPath(folder, 32) : t('prefs.general.folderDefault');
});

async function pickFolder() {
  const picked = await openPathDialog(false, true);
  if (typeof picked === 'string' && picked.length > 0) downloadDir.value = picked;
}

const durationSeconds = computed<number | undefined>(() => {
  const duration = groupStore.findGroupById(props.groupId)?.duration;
  return duration != null && duration > 0 ? duration : undefined;
});
const chapters = computed(() => groupStore.findGroupById(props.groupId)?.chapters ?? []);
const isPlaylistGroup = computed(() => {
  const group = groupStore.findGroupById(props.groupId);
  return !!group?.playlistId || (group?.playlistCount ?? 0) > 1;
});
const resolvedOutput = computed(() => {
  const globalOutput = settingsStore.settings.output;
  const override = optionsStore.getOverrides(props.groupId)?.output;

  return {
    ...globalOutput,
    ...override,
    video: { ...globalOutput.video, ...(override?.video ?? {}) },
    audio: { ...globalOutput.audio, ...(override?.audio ?? {}) },
    partialDownload: override?.partialDownload,
  };
});
const outputOverride = computed<DownloadOverrides['output'] | undefined>(() => {
  const global = settingsStore.settings.output;
  const output: NonNullable<DownloadOverrides['output']> = {};
  const changedVideo = pickChangedFields(outputState.value.video, global.video);
  const changedAudio = pickChangedFields(outputState.value.audio, global.audio);
  const changedScalars = pickChangedFields(
    {
      fileNameTemplate: outputState.value.fileNameTemplate,
      audioFileNameTemplate: outputState.value.audioFileNameTemplate,
      addThumbnail: outputState.value.addThumbnail,
      saveThumbnail: outputState.value.saveThumbnail,
      addMetadata: outputState.value.addMetadata,
      preciseCuts: outputState.value.preciseCuts,
      reversePlaylistNumbering: outputState.value.reversePlaylistNumbering,
      restrictFilenames: outputState.value.restrictFilenames,
    },
    {
      fileNameTemplate: global.fileNameTemplate,
      audioFileNameTemplate: global.audioFileNameTemplate,
      addThumbnail: global.addThumbnail,
      saveThumbnail: global.saveThumbnail,
      addMetadata: global.addMetadata,
      preciseCuts: global.preciseCuts,
      reversePlaylistNumbering: global.reversePlaylistNumbering,
      restrictFilenames: global.restrictFilenames,
    },
  );

  if (Object.keys(changedVideo).length > 0) output.video = changedVideo;
  if (Object.keys(changedAudio).length > 0) output.audio = changedAudio;
  Object.assign(output, changedScalars);

  if (downloadDir.value) output.downloadDir = downloadDir.value;

  const partialDownload = buildPartialDownloadOverride(currentSelection.value, durationSeconds.value);
  if (partialDownload) {
    output.partialDownload = partialDownload;
  }

  return Object.keys(output).length > 0 ? output : undefined;
});

function createDefaultSelectionState(): PartialDownloadSelection {
  return createDefaultSelection(uuidv4(), chapters.value, durationSeconds.value);
}

function pickChangedFields<T extends Record<string, unknown>>(current: T, base: T): Partial<T> {
  return (Object.keys(current) as Array<keyof T>).reduce((changed, key) => {
    if (current[key] !== base[key]) {
      changed[key] = current[key];
    }
    return changed;
  }, {} as Partial<T>);
}

function createOutputState(output: OutputSettings): OutputSettings {
  return {
    ...output,
    video: { ...output.video },
    audio: { ...output.audio },
  };
}

function syncFromStore() {
  outputState.value = createOutputState(resolvedOutput.value);
  downloadDir.value = optionsStore.getOverrides(props.groupId)?.output?.downloadDir ?? '';
  currentSelection.value = resolvedOutput.value.partialDownload?.section
    ? createSelectionFromSection(
        { ...resolvedOutput.value.partialDownload.section },
        uuidv4(),
        chapters.value,
        durationSeconds.value,
      )
    : createDefaultSelectionState();
}

watch(() => props.groupId, syncFromStore, { immediate: true });

watch(
  [durationSeconds, chapters],
  () => {
    if (!currentSelection.value) {
      currentSelection.value = createDefaultSelectionState();
    }
  },
  { immediate: true },
);

watch([() => props.groupId, outputOverride], () => {
  const existing = optionsStore.getOverrides(props.groupId);
  const next: DownloadOverrides = { ...(existing ?? {}) };

  if (outputOverride.value) next.output = outputOverride.value;
  else delete next.output;

  if (Object.values(next).some(value => value !== undefined)) {
    optionsStore.setOverrides(props.groupId, next);
  } else {
    optionsStore.removeOverrides(props.groupId);
  }
}, { deep: true });
</script>
