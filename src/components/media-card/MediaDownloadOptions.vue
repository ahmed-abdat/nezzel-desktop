<template>
  <!-- `rows`: one preference row per choice (per-download page). Otherwise two stacked fields (options sheet). -->
  <template v-if="rows">
    <settings-row :for-id="trackSelectId" :label="t('perDownload.quality.type')">
      <select-field :id="trackSelectId" v-model="selectedTrackType" :options="trackOptions" />
    </settings-row>
    <settings-row :for-id="formatSelectId" :label="t('perDownload.quality.quality')" :help="t('perDownload.quality.qualityHint')">
      <select-field
        :id="formatSelectId"
        :key="selectedTrackType"
        v-model="selectedFormatId"
        :options="formatOptions"
        :placeholder="formatPlaceholder"
        :disabled="formatOptions.length === 0"
      />
    </settings-row>
  </template>
  <div v-else v-bind="$attrs">
    <select-field
      :id="trackSelectId"
      v-model="selectedTrackType"
      :options="trackOptions"
      :aria-label="locale.trackSelect.screenReader"
      class="w-full"
    />
    <select-field
      :id="formatSelectId"
      :key="selectedTrackType"
      v-model="selectedFormatId"
      :options="formatOptions"
      :placeholder="formatPlaceholder"
      :disabled="formatOptions.length === 0"
      :aria-label="locale.formatSelect.screenReader"
      class="w-full"
    />
  </div>
</template>

<script setup lang="ts">
import { useI18n } from 'vue-i18n';
import { computed, PropType, ref, watch, toRefs, reactive, useId, ComputedRef } from 'vue';
import { DownloadOptions, MediaFormat, TrackType } from '../../tauri/types/media.ts';
import { SelectOption } from '../../helpers/forms.ts';
import { hasAudio } from '../../helpers/quickChoice';
import { approxAudio, approxVideo, oneFormatPerHeight, qualityLabel, sortFormats } from '../../helpers/formats.ts';
import { usePreferencesStore } from '../../stores/preferences.ts';
import { SelectField } from '../ui/select';
import SettingsRow from '../settings/SettingsRow.vue';

const i18n = useI18n();
const { t } = i18n;

const props = defineProps({
  formats: {
    type: Array as PropType<MediaFormat[]>,
    default: () => [],
  },
  defaultValue: {
    type: Object as PropType<DownloadOptions | undefined>,
    default: undefined,
  },
  modelValue: {
    type: Object as PropType<DownloadOptions | undefined>,
    default: undefined,
  },
  localeKey: {
    type: String,
    default: 'media.steps.configure.format',
  },
  autoSelect: {
    type: Boolean,
    default: true,
  },
  approximate: {
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
});

const emit = defineEmits<{
  'update:modelValue': [DownloadOptions | undefined];
}>();

defineOptions({
  inheritAttrs: false,
});

const {
  formats,
  defaultValue,
  localeKey,
  modelValue,
  autoSelect,
  approximate,
} = toRefs(props);

const preferencesStore = usePreferencesStore();
const audioAvailable = computed(() => hasAudio({ formats: formats.value }));
const validTrackType = (trackType: TrackType) => trackType === TrackType.audio && !audioAvailable.value ? TrackType.both : trackType;
const selectedTrackType = ref<TrackType>(validTrackType(preferencesStore.preferences.formats.trackType));
watch(audioAvailable, () => {
  selectedTrackType.value = validTrackType(selectedTrackType.value);
});
const selectedFormatId = ref('');
const hasDefaulted = ref(false);
const lastPick = reactive<{ audio?: string; video?: string }>({});

const locale: ComputedRef<{
  formatSelect: Record<string, string>;
  trackSelect: Record<string, string>;
}> = computed(() => i18n.tm(localeKey.value));
const isVideoLike = (t: TrackType) => t === TrackType.video || t === TrackType.both;

function makeKey(format: MediaFormat): string {
  return [
    `id=${format.id}`,
    `h=${format.height ?? ''}`,
    `fps=${format.fps ?? ''}`,
    `abr=${format.abr ?? ''}`,
  ].join('|');
}

const formatByKey = computed<Map<string, MediaFormat>>(() => {
  const map = new Map<string, MediaFormat>();
  for (const format of filteredFormats.value) {
    map.set(makeKey(format), format);
  }
  return map;
});

const baseId = useId();
const trackSelectId = `${baseId}-track`;
const formatSelectId = `${baseId}-format`;

const trackOptions = computed<SelectOption[]>(() => {
  const i18nLabels = i18n.tm('media.steps.configure.trackTypes');
  return Object.entries(TrackType).filter(([, value]) => value !== TrackType.audio || audioAvailable.value).map(([label, value]) => ({
    value,
    label: i18nLabels[label] as string,
  }));
});

const BEST_AUDIO_FORMAT: MediaFormat = Object.freeze({
  id: 'best',
  audioCodecs: [],
  videoCodecs: [],
  audioTrackIds: [],
  videoTrackIds: [],
});

const formatsByTrackType = computed<Record<TrackType, MediaFormat[]>>(() => {
  const list = formats.value ?? [];
  return {
    [TrackType.audio]: audioAvailable.value ? [BEST_AUDIO_FORMAT, ...sortFormats(list.filter(f => f.abr))] : [],
    [TrackType.video]: sortFormats(list.filter(f => f.height)),
    [TrackType.both]: sortFormats(list.filter(f => f.height)),
  };
});

const filteredFormats = computed<MediaFormat[]>(() =>
  formatsByTrackType.value[selectedTrackType.value],
);

const formatOptions = computed<SelectOption[]>(() => {
  // One entry per height (highest fps); the current choice keeps its own entry so it stays selected.
  const shown = oneFormatPerHeight(filteredFormats.value, format => makeKey(format) === selectedFormatId.value);
  const options: SelectOption[] = shown.map(format => ({
    value: makeKey(format),
    label: getFormatLabel(format, selectedTrackType.value),
  }));
  return options;
});

const formatPlaceholder = computed(() => {
  if (formatOptions.value.length === 0) return locale.value.formatSelect.noFormats;
  return locale.value.formatSelect.placeholder;
});

const selectedFormat = computed<MediaFormat | undefined>(() =>
  formatByKey.value.get(selectedFormatId.value),
);

watch(() => modelValue?.value, (val) => {
  syncFromModel(val);
}, { immediate: true });

watch(selectedTrackType, (trackType, oldTrackType) => {
  void preferencesStore.patchNow({ formats: { trackType } });
  if (!autoSelect.value) {
    selectedFormatId.value = '';
    return;
  }

  const switchedBetweenGroups = isVideoLike(trackType) !== isVideoLike(oldTrackType);
  if (!switchedBetweenGroups) return;

  const wantedId = isVideoLike(trackType) ? lastPick.video : lastPick.audio;
  const allowedIds = new Set(formatOptions.value.map(option => option.value));

  if (wantedId && allowedIds.has(wantedId)) {
    selectedFormatId.value = wantedId;
  } else {
    selectedFormatId.value = formatOptions.value[0]?.value ?? '';
  }
});

watch(
  [selectedTrackType, selectedFormat],
  () => {
    const format = selectedFormat.value;
    const payload: DownloadOptions = {
      trackType: selectedTrackType.value,
      ...(format ?? {}),
    };

    emit('update:modelValue', payload);

    if (format && isVideoLike(selectedTrackType.value)) {
      lastPick.video = format.id;
    } else if (format) {
      lastPick.audio = format.id;
    }
  },
  { immediate: true, flush: 'post' },
);

watch(
  formats,
  () => {
    if (!autoSelect.value) return;
    if (!selectedFormat.value && formatOptions.value.length) {
      selectedFormatId.value = formatOptions.value[0].value;
    }
  },
  { immediate: true },
);

function syncFromModel(model: DownloadOptions | undefined) {
  if (model) {
    hasDefaulted.value = true;
    selectedTrackType.value = validTrackType(model.trackType);

    const match = matchByDownloadOptions(model);

    if (autoSelect.value) {
      selectedFormatId.value = match ? makeKey(match) : (formatOptions.value[0]?.value ?? '');
      const picked = selectedFormatId.value;
      if (picked) {
        if (isVideoLike(model.trackType)) lastPick.video = picked;
        else lastPick.audio = picked;
      }
    }

    return;
  }

  if (!hasDefaulted.value) {
    hasDefaulted.value = true;

    const fromDefault = defaultValue?.value;
    if (fromDefault) {
      selectedTrackType.value = validTrackType(fromDefault.trackType);
      const match = matchByDownloadOptions(fromDefault);
      if (autoSelect.value) {
        selectedFormatId.value = match ? makeKey(match) : (formatOptions.value[0]?.value ?? '');
        const picked = selectedFormatId.value;
        if (picked) {
          if (isVideoLike(fromDefault.trackType)) lastPick.video = picked;
          else lastPick.audio = picked;
        }
      }
      return;
    }

    if (autoSelect.value && formatOptions.value.length) {
      selectedTrackType.value = TrackType.both;
      selectedFormatId.value = formatOptions.value[0].value;
    }
  }
}

function matchByDownloadOptions(options: DownloadOptions): MediaFormat | undefined {
  let match: MediaFormat | undefined;
  if (options.trackType === TrackType.audio) {
    if (!options.height && !options.fps && !options.abr) {
      return {
        id: 'best',
        audioCodecs: [],
        videoCodecs: [],
        audioTrackIds: [],
        videoTrackIds: [],
      };
    }
    match
      = (formats.value ?? []).find(f => f.abr === options.abr)
        || (approximate.value ? approxAudio(filteredFormats.value, options.abr) : undefined);
  } else {
    match
      = (formats.value ?? []).find(f => f.height === options.height && f.fps === options.fps)
        || (approximate.value ? approxVideo(filteredFormats.value, options.height, options.fps) : undefined);
  }
  return match;
}

function getFormatLabel(format: MediaFormat, trackType: TrackType): string {
  switch (trackType) {
    case TrackType.audio:
      if (format.id === 'best') {
        return i18n.t('common.best');
      }
      return `${format.abr}kbps`;
    case TrackType.video:
    case TrackType.both:
    default:
      return format.height != null ? qualityLabel(format.height, format.fps) : format.id;
  }
}
</script>
