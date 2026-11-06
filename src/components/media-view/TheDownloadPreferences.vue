<template>
  <div v-if="group">
    <settings-section>
      <media-download-options
        v-model="selectedOptions"
        rows
        :formats="group.formats"
        :default-value="optionsStore.getGlobalOptions()"
        approximate
      />
      <media-track-options
        v-if="hasTracks"
        v-model="selectedTracks"
        rows
        :default-value="optionsStore.getGlobalTracks()"
        :audio-options="audioTrackOptions"
        :video-options="videoTrackOptions"
        :track-type="selectedTrackType"
        :audio-help="t('perDownload.quality.audioTrackHint')"
      />
    </settings-section>
    <base-disclosure v-if="hasCodecs" class="mt-8" :label="t('perDownload.advanced')">
      <media-encoding-options
        v-model="selectedEncodings"
        rows
        :default-value="optionsStore.getGlobalEncodings()"
        :audio-options="audioCodecOptions"
        :video-options="videoCodecOptions"
        :track-type="selectedTrackType"
        :video-help="t('perDownload.quality.codecHint')"
      />
    </base-disclosure>
  </div>
</template>

<script setup lang="ts">
import { useMediaGroupStore } from '../../stores/media/group.ts';
import { Group } from '../../tauri/types/group.ts';
import MediaDownloadOptions from '../media-card/MediaDownloadOptions.vue';
import MediaEncodingOptions from '../media-card/MediaEncodingOptions.vue';
import MediaTrackOptions from '../media-card/MediaTrackOptions.vue';
import { DownloadOptions, EncodingOptions, TrackOptions, TrackType } from '../../tauri/types/media.ts';
import { useMediaOptionsStore } from '../../stores/media/options.ts';
import { computed, watch } from 'vue';
import SettingsSection from '../settings/SettingsSection.vue';
import BaseDisclosure from '../base/BaseDisclosure.vue';
import { useI18n } from 'vue-i18n';
import { useMediaResolutionSelection } from '../../composables/useMediaResolutionSelection';

const groupStore = useMediaGroupStore();
const optionsStore = useMediaOptionsStore();
const { t } = useI18n();

const { groupId } = defineProps({
  groupId: {
    type: String,
    required: true,
  },
});

const group = computed<Group | undefined>(() => groupStore.findGroupById(groupId));

const selectedOptions = computed({
  get: () => optionsStore.getOptions(group.value?.id ?? ''),
  set: (value: DownloadOptions) => optionsStore.setOptions(group.value?.id ?? '', value),
});

const selectedEncodings = computed({
  get: () => optionsStore.getEncodings(group.value?.id ?? ''),
  set: (value: EncodingOptions | undefined) => {
    if (value) optionsStore.setEncodings(group.value?.id ?? '', value);
    else optionsStore.removeEncodings(group.value?.id ?? '');
  },
});
const selectedTracks = computed({
  get: () => optionsStore.getTracks(group.value?.id ?? ''),
  set: (value: TrackOptions | undefined) => {
    if (value) optionsStore.setTracks(group.value?.id ?? '', value);
    else optionsStore.removeTracks(group.value?.id ?? '');
  },
});
const selectedTrackType = computed(() => selectedOptions.value?.trackType ?? TrackType.both);

const unavailableTrackSuffix = computed(() => t('media.steps.configure.tracks.unavailableForSelectedResolution'));
const availableTrackPrefix = computed(() => t('media.steps.configure.tracks.availableInResolutions'));
const {
  audioCodecOptions,
  videoCodecOptions,
  audioTrackOptions,
  videoTrackOptions,
} = useMediaResolutionSelection({
  formats: () => group.value?.formats ?? [],
  audioCodecs: () => group.value?.audioCodecs ?? [],
  videoCodecs: () => group.value?.videoCodecs ?? [],
  audioTracks: () => group.value?.audioTracks ?? [],
  videoTracks: () => group.value?.videoTracks ?? [],
  selectedOptions,
  approximate: true,
  unavailableTrackSuffix,
  availableTrackPrefix,
});
const hasTracks = computed(() => audioTrackOptions.value.length > 0 || videoTrackOptions.value.length > 0);
const hasCodecs = computed(() => audioCodecOptions.value.length > 0 || videoCodecOptions.value.length > 0);

watch(selectedOptions, () => {
  if (!group.value) return;
  const globalOptions = optionsStore.getGlobalOptions();
  if (!selectedOptions.value && globalOptions) {
    optionsStore.applyOptionsToGroup(
      group.value,
      globalOptions,
    );
  }
});

watch(selectedEncodings, () => {
  if (!group.value) return;
  const globalEncodings = optionsStore.getGlobalEncodings();
  if (!selectedEncodings.value && globalEncodings) {
    optionsStore.applyEncodingsToGroup(
      group.value,
      globalEncodings,
    );
  }
});

watch(selectedTracks, () => {
  if (!group.value) return;
  const globalTracks = optionsStore.getGlobalTracks();
  if (!selectedTracks.value && globalTracks) {
    optionsStore.applyTracksToGroup(
      group.value,
      globalTracks,
    );
  }
});
</script>
