<template>
  <!-- `rows`: a preference row per select, hidden when that side has nothing to choose. -->
  <template v-if="rows">
    <settings-row v-if="showAudio" :for-id="audioSelectId" :label="locale.audioSelect.placeholder" :help="audioHelp">
      <select-field
        :id="audioSelectId"
        v-model="selectedAudio"
        :options="audioOptions"
        :placeholder="audioPlaceholder"
        :disabled="isAudioDisabled || audioOptions.length === 0"
      />
    </settings-row>
    <settings-row v-if="showVideo" :for-id="videoSelectId" :label="locale.videoSelect.placeholder" :help="videoHelp">
      <select-field
        :id="videoSelectId"
        v-model="selectedVideo"
        :options="videoOptions"
        :placeholder="videoPlaceholder"
        :disabled="isVideoDisabled || videoOptions.length === 0"
      />
    </settings-row>
  </template>
  <div v-else v-bind="$attrs">
    <select-field
      :id="audioSelectId"
      v-model="selectedAudio"
      :options="audioOptions"
      :placeholder="audioPlaceholder"
      :disabled="isAudioDisabled || audioOptions.length === 0"
      :aria-label="locale.audioSelect.screenReader"
    />
    <select-field
      :id="videoSelectId"
      v-model="selectedVideo"
      :options="videoOptions"
      :placeholder="videoPlaceholder"
      :disabled="isVideoDisabled || videoOptions.length === 0"
      :aria-label="locale.videoSelect.screenReader"
    />
  </div>
</template>

<script setup lang="ts">
import { useI18n } from 'vue-i18n';
import { computed, ComputedRef, PropType, ref, toRefs, useId, watch } from 'vue';
import { TrackType } from '../../tauri/types/media.ts';
import { SelectOption } from '../../helpers/forms.ts';
import { resolveSelectableValue } from '../../helpers/resolutionSelection.ts';
import { SelectField } from '../ui/select';
import SettingsRow from '../settings/SettingsRow.vue';

type DualSelectValue = {
  audio?: string;
  video?: string;
};

const i18n = useI18n();

const props = defineProps({
  audioOptions: {
    type: Array as PropType<SelectOption[]>,
    default: () => [],
  },
  videoOptions: {
    type: Array as PropType<SelectOption[]>,
    default: () => [],
  },
  defaultValue: {
    type: Object as PropType<DualSelectValue | undefined>,
    default: undefined,
  },
  modelValue: {
    type: Object as PropType<DualSelectValue | undefined>,
    default: undefined,
  },
  trackType: {
    type: String as PropType<TrackType>,
    default: TrackType.both,
  },
  localeKey: {
    type: String,
    required: true,
  },
  autoSelect: {
    type: Boolean,
    default: false,
  },
  join: {
    type: Boolean,
    default: false,
  },
  rows: {
    type: Boolean,
    default: false,
  },
  audioHelp: {
    type: String,
    default: undefined,
  },
  videoHelp: {
    type: String,
    default: undefined,
  },
});

const emit = defineEmits<{
  'update:modelValue': [DualSelectValue | undefined];
}>();

defineOptions({
  inheritAttrs: false,
});

const {
  audioOptions,
  videoOptions,
  defaultValue,
  modelValue,
  localeKey,
  trackType,
  autoSelect,
} = toRefs(props);

const locale: ComputedRef<{
  audioSelect: Record<string, string>;
  videoSelect: Record<string, string>;
}> = computed(() => i18n.tm(localeKey.value));

const baseId = useId();
const audioSelectId = `${baseId}-audio`;
const videoSelectId = `${baseId}-video`;

const selectedAudio = ref('');
const selectedVideo = ref('');
const hasDefaulted = ref(false);

const isAudioDisabled = computed(() => trackType.value === TrackType.video);
const isVideoDisabled = computed(() => trackType.value === TrackType.audio);
const showAudio = computed(() => !isAudioDisabled.value && audioOptions.value.length > 0);
const showVideo = computed(() => !isVideoDisabled.value && videoOptions.value.length > 0);
const placeholderFor = (options: SelectOption[], copy: Record<string, string>) => {
  if (options.length === 0) return copy.noFormats;
  return copy.placeholder;
};
const audioPlaceholder = computed(() => placeholderFor(audioOptions.value, locale.value.audioSelect));
const videoPlaceholder = computed(() => placeholderFor(videoOptions.value, locale.value.videoSelect));

watch(() => modelValue?.value, (val) => {
  syncFromModel(val);
}, { immediate: true });

watch(
  [audioOptions, videoOptions, isAudioDisabled, isVideoDisabled],
  () => {
    ensureValidSelections();
  },
  { immediate: true },
);

watch(
  [selectedAudio, selectedVideo, isAudioDisabled, isVideoDisabled],
  () => {
    const payload: DualSelectValue = {};

    if (!isAudioDisabled.value && selectedAudio.value) {
      payload.audio = selectedAudio.value;
    }
    if (!isVideoDisabled.value && selectedVideo.value) {
      payload.video = selectedVideo.value;
    }

    emit('update:modelValue', Object.keys(payload).length ? payload : undefined);
  },
  { immediate: true, flush: 'post' },
);

function ensureValidSelections() {
  if (!isAudioDisabled.value) {
    selectedAudio.value = resolveSelectableValue(
      selectedAudio.value,
      audioOptions.value,
      autoSelect.value,
    );
  }
  if (!isVideoDisabled.value) {
    selectedVideo.value = resolveSelectableValue(
      selectedVideo.value,
      videoOptions.value,
      autoSelect.value,
    );
  }
}

function syncFromModel(model: DualSelectValue | undefined) {
  if (model) {
    hasDefaulted.value = true;
    selectedAudio.value = model.audio ?? '';
    selectedVideo.value = model.video ?? '';
    return;
  }

  if (hasDefaulted.value) return;
  hasDefaulted.value = true;

  const fromDefault = defaultValue?.value;
  if (fromDefault) {
    selectedAudio.value = fromDefault.audio ?? '';
    selectedVideo.value = fromDefault.video ?? '';
    return;
  }

  if (autoSelect.value) {
    ensureValidSelections();
  }
}
</script>
