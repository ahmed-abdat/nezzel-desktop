<template>
  <!-- Compact path picker for a settings row: the path (short, full on hover), Change…, clear, recents. -->
  <div class="flex min-w-0 items-center gap-1.5" v-bind="$attrs">
    <bdi
      class="min-w-0 max-w-56 truncate text-nz-meta"
      :class="invalid ? 'text-destructive' : path ? 'text-foreground' : 'text-muted-foreground'"
      dir="ltr"
      :title="invalid ? (validatorMessage ?? invalidText) : (path ?? undefined)"
    >{{ path ? shortFolderPath(path, 34) : (placeholder ?? inputText) }}</bdi>
    <Button
      variant="outline"
      size="xs"
      class="shrink-0"
      :aria-label="`${label}: ${directory ? t('components.base.folderInput.select') : t('components.base.fileInput.select')}`"
      :disabled="isSelecting"
      @click="select"
    >
      <Loader2 v-if="isSelecting" class="animate-spin" aria-hidden="true" />
      {{ t('prefs.general.change') }}
    </Button>
    <tooltip v-if="clearable && path" :content="directory ? t('components.base.folderInput.clear') : t('components.base.fileInput.clear')">
      <Button
        variant="ghost"
        size="icon-xs"
        class="shrink-0 text-muted-foreground"
        :aria-label="directory ? t('components.base.folderInput.clear') : t('components.base.fileInput.clear')"
        @click="clear"
      >
        <X aria-hidden="true" />
      </Button>
    </tooltip>
    <DropdownMenu v-if="showRecent">
      <DropdownMenuTrigger as-child :disabled="!hasRecents">
        <Button
          variant="ghost"
          size="icon-xs"
          class="shrink-0 text-muted-foreground"
          :disabled="!hasRecents"
          :aria-label="directory ? t('components.base.folderInput.recents.open') : t('components.base.fileInput.recents.open')"
        >
          <History aria-hidden="true" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent class="max-w-96">
        <DropdownMenuItem v-for="recentValue in recentValues" :key="recentValue" :title="recentValue" @select="selectRecent(recentValue)">
          <bdi class="min-w-0 truncate" dir="ltr">{{ directory ? shortFolderPath(recentValue, 48) : basename(recentValue) }}</bdi>
        </DropdownMenuItem>
        <DropdownMenuSeparator />
        <DropdownMenuItem danger @select="clearRecents">
          <Trash2 aria-hidden="true" />
          {{ directory ? t('components.base.folderInput.recents.clear') : t('components.base.fileInput.recents.clear') }}
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  </div>
</template>

<script setup lang="ts">

import Tooltip from '../ui/tooltip/Tooltip.vue';
import { computed, PropType, ref } from 'vue';
import { useDialog } from '../../composables/useDialog';
import { useI18n } from 'vue-i18n';
import { History, Loader2, Trash2, X } from '@lucide/vue';
import { Button } from '../ui/button';
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger } from '../ui/dropdown-menu';
import { shortFolderPath } from '../../helpers/folderPath';

const { openPathDialog } = useDialog();
const { t } = useI18n();

const emit = defineEmits(['update:modelValue', 'clearRecents']);
defineOptions({
  inheritAttrs: false,
});

const isSelecting = ref(false);
const hasRecents = computed(() => (recentValues?.length ?? 0) > 0);
const path = defineModel({ type: String as PropType<string | null>, required: false, default: null });

const { label, invalid, clearable, directory, showRecent, validatorMessage, recentValues } = defineProps({
  label: {
    type: String,
    required: true,
  },
  invalid: {
    type: Boolean,
    required: false,
    default: false,
  },
  clearable: {
    type: Boolean,
    required: false,
    default: true,
  },
  directory: {
    type: Boolean,
    required: false,
    default: false,
  },
  validatorMessage: {
    type: String,
    required: false,
  },
  showRecent: {
    type: Boolean,
    required: false,
    default: false,
  },
  recentValues: {
    type: Array as PropType<string[]>,
    required: false,
    default: () => [],
  },
  recentTitle: {},
  /** Shown when no path is set, e.g. what the empty value falls back to. */
  placeholder: {
    type: String,
    required: false,
  },
});

const invalidText = computed(() => (directory ? t('components.base.folderInput.invalid') : t('components.base.fileInput.invalid')));
const inputText = computed(() => {
  const placeholderText = directory ? t('components.base.folderInput.placeholder') : t('components.base.fileInput.placeholder');
  return path.value ?? placeholderText;
});

const select = async () => {
  isSelecting.value = true;
  try {
    const selected = await openPathDialog(false, directory);
    if (!clearable && !selected) return;
    path.value = selected;
    emit('update:modelValue', selected);
  } catch (e) {
    console.error(e);
  } finally {
    isSelecting.value = false;
  }
};

const selectRecent = (value: string) => {
  path.value = value;
  emit('update:modelValue', value);
};

const clear = () => {
  path.value = null;
  emit('update:modelValue', null);
};

function clearRecents() {
  emit('clearRecents');
}

function basename(path: string) {
  const normalized = path.replace(/\\/g, '/');
  const lastSegmentIndex = normalized.lastIndexOf('/');
  return lastSegmentIndex >= 0 ? normalized.slice(lastSegmentIndex + 1) : normalized;
}

</script>
