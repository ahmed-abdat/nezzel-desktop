<template>
  <settings-section :title="t('prefs.adv.files.title')">
    <settings-row for-id="out-video-format" :label="t('prefs.adv.files.videoFormat')" :help="t('prefs.adv.files.videoFormatHint')">
      <SelectField id="out-video-format" v-model="output.video.container" :options="videoContainerOptions" />
    </settings-row>
    <settings-row for-id="out-audio-format" :label="t('prefs.adv.files.audioFormat')" :help="t('prefs.adv.files.audioFormatHint')">
      <SelectField id="out-audio-format" v-model="output.audio.format" :options="audioFormatOptions" />
    </settings-row>
    <settings-row for-id="out-cover" :label="t('prefs.adv.files.cover')" :help="t('prefs.adv.files.coverHint')">
      <Switch id="out-cover" v-model="output.addThumbnail" />
    </settings-row>
    <settings-row for-id="out-metadata" :label="t('prefs.adv.files.metadata')" :help="t('prefs.adv.files.metadataHint')">
      <Switch id="out-metadata" v-model="output.addMetadata" />
    </settings-row>
    <settings-link-row :to="{ name: 'location' }" :label="t('prefs.adv.files.folders')" :help="t('prefs.adv.files.foldersHint')" />
    <settings-link-row :to="{ name: 'subtitles' }" :label="t('prefs.adv.files.subtitles')" :help="t('prefs.adv.files.subtitlesHint')" />
    <settings-link-row :to="{ name: 'input-filters' }" :label="t('prefs.adv.files.filters')" :help="t('prefs.adv.files.filtersHint')" />

    <template #more>
      <settings-row v-if="isMac" for-id="out-keep-codec" :label="t('prefs.adv.files.keepOriginalCodec')" :help="t('prefs.adv.files.keepOriginalCodecHint')">
        <Switch id="out-keep-codec" v-model="output.video.keepOriginalCodec" />
      </settings-row>
      <settings-row for-id="out-save-cover" :label="t('prefs.adv.files.saveCover')" :help="t('prefs.adv.files.saveCoverHint')">
        <Switch id="out-save-cover" v-model="output.saveThumbnail" />
      </settings-row>
      <settings-row for-id="out-keep-video" :label="t('prefs.adv.files.keepVideo')" :help="t('prefs.adv.files.keepVideoHint')">
        <Switch id="out-keep-video" v-model="keepOriginalStreamsVideo" />
      </settings-row>
      <settings-row for-id="out-keep-audio" :label="t('prefs.adv.files.keepAudio')" :help="t('prefs.adv.files.keepAudioHint')">
        <Switch id="out-keep-audio" v-model="keepOriginalStreamsAudio" />
      </settings-row>
      <settings-row for-id="out-video-after" :label="t('prefs.adv.files.videoAfter')" :help="t('prefs.adv.files.videoAfterHint')">
        <SelectField id="out-video-after" v-model="output.video.postprocessPreset" :options="videoPresetOptions" />
      </settings-row>
      <p v-if="showMp42ContainerConflict" class="pb-3 text-[12px] text-destructive" role="alert">{{ t('settings.output.postprocess.mp42RequiresMp4') }}</p>
      <p v-else-if="showKeepOriginalStreamsConflict" class="pb-3 text-[12px] text-destructive" role="alert">{{ t('prefs.adv.files.keepConflict') }}</p>
      <p v-else-if="showReencodeWarning" class="pb-3 text-[12px] text-amber-foreground" role="status">{{ t('prefs.adv.files.slowWarning') }}</p>
      <settings-row v-if="showVideoPostprocessMode" for-id="out-video-mode" :label="t('prefs.adv.files.customMode')" :help="t('prefs.adv.files.customModeHint')">
        <SelectField id="out-video-mode" v-model="output.video.customPostprocessMode" :options="videoModeOptions" />
      </settings-row>
      <settings-row v-if="showVideoPostprocessArgs" stacked for-id="out-video-args" :label="t('prefs.adv.files.ffmpegArgs')" :help="t('prefs.adv.files.ffmpegArgsHint')">
        <Textarea id="out-video-args" v-model="output.video.postprocessArgs" dir="ltr" rows="2" class="py-1.5 font-mono text-[12px]" />
      </settings-row>
      <settings-row for-id="out-audio-after" :label="t('prefs.adv.files.audioAfter')" :help="t('prefs.adv.files.audioAfterHint')">
        <SelectField id="out-audio-after" v-model="output.audio.postprocessPreset" :options="audioPresetOptions" />
      </settings-row>
      <settings-row v-if="showAudioPostprocessArgs" stacked for-id="out-audio-args" :label="t('prefs.adv.files.ffmpegArgs')" :help="t('prefs.adv.files.ffmpegArgsHint')">
        <Textarea id="out-audio-args" v-model="output.audio.postprocessArgs" dir="ltr" rows="2" class="py-1.5 font-mono text-[12px]" />
      </settings-row>
    </template>
  </settings-section>
</template>

<script setup lang="ts">
import { Textarea } from '../ui/textarea';
import { computed } from 'vue';
import { useI18n } from 'vue-i18n';
import { SelectField } from '@/components/ui/select';
import Switch from '@/components/ui/switch/Switch.vue';
import SettingsLinkRow from './SettingsLinkRow.vue';
import SettingsRow from './SettingsRow.vue';
import SettingsSection from './SettingsSection.vue';
import { Settings } from '../../tauri/types/config';
import { AudioFormat, AudioPostprocessPreset, VideoContainer, VideoPostprocessMode, VideoPostprocessPreset } from '../../tauri/types/media';
import { useOutputSettingsEditor } from '../../composables/useOutputSettingsEditor';
import { usePlatform } from '../../composables/usePlatform';

const { t } = useI18n();
const { isMac } = usePlatform();
const settings = defineModel<Settings>({ required: true });

const output = computed({
  get: () => settings.value.output,
  set: (value) => { settings.value.output = value; },
});

const optionsOf = (values: string[], key: string) => computed(() => values.map(value => ({ value, label: t(`${key}.${value}`) })));
const videoContainerOptions = optionsOf(Object.values(VideoContainer), 'settings.output.tabs.video.container.options');
const audioFormatOptions = optionsOf(Object.values(AudioFormat), 'settings.output.tabs.audio.format.options');
const videoPresetOptions = optionsOf(Object.values(VideoPostprocessPreset), 'prefs.adv.files.videoAfterOptions');
const videoModeOptions = optionsOf(Object.values(VideoPostprocessMode), 'prefs.adv.files.customModeOptions');
const audioPresetOptions = optionsOf(Object.values(AudioPostprocessPreset), 'prefs.adv.files.videoAfterOptions');

const {
  keepOriginalStreamsVideo,
  keepOriginalStreamsAudio,
  showVideoPostprocessArgs,
  showVideoPostprocessMode,
  showAudioPostprocessArgs,
  showReencodeWarning,
  showKeepOriginalStreamsConflict,
  showMp42ContainerConflict,
} = useOutputSettingsEditor(output);
</script>
