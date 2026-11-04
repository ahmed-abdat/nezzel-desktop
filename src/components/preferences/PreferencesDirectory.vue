<template>
  <!-- Video and audio folders, then how each is split into subfolders. Rows, no video/audio tabs. -->
  <settings-row :label="t('folders.video')" :help="t('folders.typeHint')">
    <base-file-input
      v-model="prefs.paths.videoDownloadDir"
      :label="t('folders.video')"
      :placeholder="t('folders.same')"
      :recent-values="recentValues.videoDir"
      show-recent
      directory
      @clear-recents="clearRecents('videoDir')"
    />
  </settings-row>
  <settings-row :label="t('folders.audio')" :help="t('folders.typeHint')">
    <base-file-input
      v-model="prefs.paths.audioDownloadDir"
      :label="t('folders.audio')"
      :placeholder="t('folders.same')"
      :recent-values="recentValues.audioDir"
      show-recent
      directory
      @clear-recents="clearRecents('audioDir')"
    />
  </settings-row>
  <directory-preset-selector
    v-model="videoTemplate"
    v-model:preset="selectedVideoPreset"
    id-prefix="video"
    :presets="dirPresets"
    :track-type="TrackType.video"
    :preset-label="t('folders.videoSub')"
    :format-label="t('folders.subPattern')"
  />
  <directory-preset-selector
    v-model="audioTemplate"
    v-model:preset="selectedAudioPreset"
    id-prefix="audio"
    :presets="dirPresets"
    :track-type="TrackType.audio"
    :preset-label="t('folders.audioSub')"
    :format-label="t('folders.subPattern')"
  />
</template>

<script setup lang="ts">
import SettingsRow from '../settings/SettingsRow.vue';
import DirectoryPresetSelector from './DirectoryPresetSelector.vue';
import { DirectoryPreset } from '../../tauri/types/config';
import { useI18n } from 'vue-i18n';
import { computed, onMounted, ref, watch } from 'vue';
import { Preferences } from '../../tauri/types/preferences';
import BaseFileInput from '../base/BaseFileInput.vue';
import { TrackType } from '../../tauri/types/media';
import { usePreferencesStore } from '../../stores/preferences';

interface PresetDef {
  label: string;
  value: DirectoryPreset;
  format: string;
  example?: string;
}

const { t } = useI18n();
const preferencesStore = usePreferencesStore();
const prefs = defineModel<Preferences>({ required: true });
const emit = defineEmits(['clearRecents']);
const { recentValues } = defineProps<{ recentValues: Record<string, string[]> }>();

const dirPresets: PresetDef[] = [
  {
    label: t('location.directory.directoryPreset.options.none'),
    example: '',
    value: DirectoryPreset.None,
    format: '',
  },
  {
    label: t('location.directory.directoryPreset.options.playlist'),
    example: t('location.directory.directoryPreset.examples.playlist'),
    value: DirectoryPreset.Playlist,
    format: '%(playlist_title)s',
  },
  {
    label: t('location.directory.directoryPreset.options.channel'),
    example: t('location.directory.directoryPreset.examples.channel'),
    value: DirectoryPreset.Channel,
    format: '%(uploader)s',
  },
  {
    label: t('location.directory.directoryPreset.options.channelPlaylist'),
    example: t('location.directory.directoryPreset.examples.channelPlaylist'),
    value: DirectoryPreset.ChannelPlaylist,
    format: '%(uploader)s/%(playlist_title)s',
  },
  {
    label: t('location.directory.directoryPreset.options.yearMonth'),
    example: t('location.directory.directoryPreset.examples.yearMonth'),
    value: DirectoryPreset.YearMonth,
    format: '%(upload_date>%Y)s/%(upload_date>%m)s',
  },
  {
    label: t('location.directory.directoryPreset.options.artistAlbum'),
    example: t('location.directory.directoryPreset.examples.artistAlbum'),
    value: DirectoryPreset.ArtistAlbum,
    format: '%(artist,uploader)s/%(album,playlist_title)s',
  },
  {
    label: t('location.directory.directoryPreset.options.custom'),
    value: DirectoryPreset.Custom,
    format: '',
  },

];

const selectedVideoPreset = ref<DirectoryPreset>(DirectoryPreset.Custom);
const selectedAudioPreset = ref<DirectoryPreset>(DirectoryPreset.Custom);

const videoTemplate = computed<string>({
  get: () => prefs.value.paths.videoDirectoryTemplate ?? '',
  set(val) {
    prefs.value.paths.videoDirectoryTemplate = val;
  },
});

const audioTemplate = computed<string>({
  get: () => prefs.value.paths.audioDirectoryTemplate ?? '',
  set(val) {
    prefs.value.paths.audioDirectoryTemplate = val;
  },
});

onMounted(() => {
  const videoMatch = dirPresets.find(
    p => p.value !== DirectoryPreset.Custom
      && prefs.value.paths.videoDirectoryTemplate === p.format,
  );
  selectedVideoPreset.value = videoMatch?.value ?? DirectoryPreset.Custom;

  const audioMatch = dirPresets.find(
    p => p.value !== DirectoryPreset.Custom
      && prefs.value.paths.audioDirectoryTemplate === p.format,
  );
  selectedAudioPreset.value = audioMatch?.value ?? DirectoryPreset.Custom;
});

const clearRecents = (label: string) => {
  emit('clearRecents', label);
};

watch(
  () => prefs.value.paths.audioDownloadDir,
  (newFile, oldFile) => {
    if (newFile && newFile !== oldFile) {
      void preferencesStore.addRecentPath('audioDir', newFile);
    }
  },
);

watch(
  () => prefs.value.paths.videoDownloadDir,
  (newFile, oldFile) => {
    if (newFile && newFile !== oldFile) {
      void preferencesStore.addRecentPath('videoDir', newFile);
    }
  },
);
</script>
