<template>
  <format-preset-selector
    v-model="videoTemplate"
    v-model:preset="selectedVideoFormatPreset"
    id-prefix="video"
    :presets="videoFormatPresets"
    :restrict-filenames="restrictFilenames"
    :track-type="TrackType.video"
    :preset-label="t('folders.videoNames')"
    :format-label="t('folders.namePattern')"
  />
  <format-preset-selector
    v-model="audioTemplate"
    v-model:preset="selectedAudioFormatPreset"
    id-prefix="audio"
    :presets="audioFormatPresets"
    :restrict-filenames="restrictFilenames"
    :track-type="TrackType.audio"
    :preset-label="t('folders.audioNames')"
    :format-label="t('folders.namePattern')"
  />
  <settings-row for-id="restrict-filenames" :label="t('perDownload.output.safeNames')" :help="t('perDownload.output.safeNamesHint')">
    <Switch id="restrict-filenames" v-model="restrictFilenames" />
  </settings-row>
  <settings-row for-id="reverse-playlist-numbering" :label="t('perDownload.output.reverse')" :help="t('perDownload.output.reverseHint')">
    <Switch id="reverse-playlist-numbering" v-model="reversePlaylistNumbering" />
  </settings-row>
</template>

<script setup lang="ts">
import SettingsRow from '../settings/SettingsRow.vue';
import Switch from '../ui/switch/Switch.vue';
import FormatPresetSelector from './FormatPresetSelector.vue';
import { FormatPreset, Settings } from '../../tauri/types/config';
import { useI18n } from 'vue-i18n';
import { computed, onMounted, ref } from 'vue';
import { TrackType } from '../../tauri/types/media';

interface PresetDef {
  label: string;
  value: FormatPreset;
  format: string;
  example?: string;
}

const { t } = useI18n();
const settings = defineModel<Settings>({ required: true });

const videoFormatPresets: PresetDef[] = [
  {
    label: t('location.filename.formatPreset.options.titleQuality'),
    example: t('location.filename.formatPreset.examples.video.titleQuality'),
    value: FormatPreset.TitleQuality,
    format: '%(title).200s-(%(height)sp%(fps).0d).%(ext)s',
  },
  {
    label: t('location.filename.formatPreset.options.titleOnly'),
    example: t('location.filename.formatPreset.examples.video.titleOnly'),
    value: FormatPreset.TitleOnly,
    format: '%(title).200s.%(ext)s',
  },
  {
    label: t('location.filename.formatPreset.options.titleQualityPlaylist'),
    example: t('location.filename.formatPreset.examples.video.titleQualityPlaylist'),
    value: FormatPreset.TitleQualityPlaylist,
    format: '%(playlist_index)02d-%(title).200s-(%(height)sp%(fps).0d).%(ext)s',
  },
  {
    label: t('location.filename.formatPreset.options.custom'),
    value: FormatPreset.Custom,
    format: '',
  },
];

const audioFormatPresets: PresetDef[] = [
  {
    label: t('location.filename.formatPreset.options.titleQuality'),
    example: t('location.filename.formatPreset.examples.audio.titleQuality'),
    value: FormatPreset.TitleQuality,
    format: '%(title).200s-(%(abr)dk).%(ext)s',
  },
  {
    label: t('location.filename.formatPreset.options.titleOnly'),
    example: t('location.filename.formatPreset.examples.audio.titleOnly'),
    value: FormatPreset.TitleOnly,
    format: '%(title).200s.%(ext)s',
  },
  {
    label: t('location.filename.formatPreset.options.titleQualityPlaylist'),
    example: t('location.filename.formatPreset.examples.audio.titleQualityPlaylist'),
    value: FormatPreset.TitleQualityPlaylist,
    format: '%(playlist_index)02d-%(title).200s-(%(abr)dk).%(ext)s',
  },
  {
    label: t('location.filename.formatPreset.options.custom'),
    value: FormatPreset.Custom,
    format: '',
  },
];

const selectedVideoFormatPreset = ref<FormatPreset>(FormatPreset.Custom);
const selectedAudioFormatPreset = ref<FormatPreset>(FormatPreset.Custom);

const videoTemplate = computed<string>({
  get: () => settings.value.output.fileNameTemplate ?? '',
  set(val) {
    settings.value.output.fileNameTemplate = val;
  },
});

const audioTemplate = computed<string>({
  get: () => settings.value.output.audioFileNameTemplate ?? '',
  set(val) {
    settings.value.output.audioFileNameTemplate = val;
  },
});

const restrictFilenames = computed<boolean>({
  get: () => settings.value.output.restrictFilenames ?? false,
  set(val) {
    settings.value.output.restrictFilenames = val;
  },
});

const reversePlaylistNumbering = computed<boolean>({
  get: () => settings.value.output.reversePlaylistNumbering ?? false,
  set(val) {
    settings.value.output.reversePlaylistNumbering = val;
  },
});

onMounted(() => {
  const videoMatch = videoFormatPresets.find(
    p => p.value !== FormatPreset.Custom
      && settings.value.output.fileNameTemplate === p.format,
  );
  selectedVideoFormatPreset.value = videoMatch?.value ?? FormatPreset.Custom;

  const audioMatch = audioFormatPresets.find(
    p => p.value !== FormatPreset.Custom
      && settings.value.output.audioFileNameTemplate === p.format,
  );
  selectedAudioFormatPreset.value = audioMatch?.value ?? FormatPreset.Custom;
});
</script>
