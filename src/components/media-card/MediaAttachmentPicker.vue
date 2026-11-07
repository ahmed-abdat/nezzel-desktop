<template>
  <fieldset class="border-b border-hairline px-4 pb-4 pt-3" :aria-describedby="countId">
    <legend class="sr-only">{{ t('attachments.title') }}</legend>
    <div class="flex h-7 items-center gap-2">
      <span class="text-[11.5px] font-semibold text-muted-foreground" aria-hidden="true">{{ t('attachments.title') }}</span>
      <span :id="countId" role="status" class="tnum text-[11.5px] text-muted-foreground/80">{{ t('attachments.selectedCount', { selected: selected.length, total: attachments.length }) }}</span>
      <span class="flex-1" />
      <Checkbox :id="`${baseId}-all`" :model-value="allSelected" @update:model-value="selectAll($event === true)" />
      <label :for="`${baseId}-all`" class="cursor-pointer text-[12px]">{{ t('attachments.selectAll') }}</label>
    </div>
    <ul class="mt-2 grid grid-cols-3 gap-1.5">
      <li v-for="(attachment, index) in attachments" :key="attachment.id">
        <tooltip :content="label(attachment.kind, index)">
          <button
              type="button"
              role="checkbox"
              :aria-checked="isSelected(attachment.id)"
              :aria-label="label(attachment.kind, index)"
              class="group/tile relative block aspect-square w-full overflow-hidden rounded-[5px] bg-muted outline-none ring-offset-2 ring-offset-card transition-[box-shadow] duration-[var(--nz-dur-fast,120ms)] focus-visible:ring-2 focus-visible:ring-ring/60"
              :class="isSelected(attachment.id) ? 'ring-2 ring-primary' : 'ring-1 ring-foreground/10 hover:ring-foreground/25'"
              @click="toggle(attachment.id, !isSelected(attachment.id))"
          >
            <img
                v-if="preview(attachment) && !failed.has(attachment.id)"
                :src="preview(attachment)"
                :alt="attachment.alt ?? ''"
                class="size-full object-cover transition-opacity duration-[var(--nz-dur-fast,120ms)]"
                :class="!isSelected(attachment.id) && 'opacity-55'"
                loading="lazy"
                referrerpolicy="no-referrer"
                @error="failed.add(attachment.id)"
            >
            <span v-else class="flex size-full items-center justify-center text-muted-foreground/60">
              <ImageIcon v-if="attachment.kind === 'image'" class="size-5" aria-hidden="true" />
              <Film v-else class="size-5" aria-hidden="true" />
            </span>
            <span v-if="attachment.kind !== 'image'" class="absolute bottom-1 start-1 flex size-5 items-center justify-center rounded-full bg-black/60 text-white" aria-hidden="true">
              <Play class="size-2.5 fill-current" />
            </span>
            <span
                class="absolute end-1 top-1 flex size-[18px] items-center justify-center rounded-full transition-colors duration-[var(--nz-dur-fast,120ms)]"
                :class="isSelected(attachment.id) ? 'bg-primary text-primary-foreground' : 'border-[1.5px] border-white/90 bg-black/25'"
                aria-hidden="true"
            >
              <Check v-if="isSelected(attachment.id)" class="size-3" :stroke-width="3" />
            </span>
          </button>
        </tooltip>
      </li>
    </ul>
    <p v-if="selected.length === 0" class="mt-2 text-[12px] text-amber-foreground" role="status">{{ t('attachments.chooseOne') }}</p>
  </fieldset>
</template>

<script setup lang="ts">
import { useSettingsStore } from '../../stores/settings';
import { remoteImagesAllowed } from '../../helpers/remoteImages';
import Tooltip from '../ui/tooltip/Tooltip.vue';
import { computed, reactive, useId } from 'vue';
import { useI18n } from 'vue-i18n';
import { Check, Film, Image as ImageIcon, Play } from '@lucide/vue';
import Checkbox from '../ui/checkbox/Checkbox.vue';
import { useMediaOptionsStore } from '../../stores/media/options';
import type { Group } from '../../tauri/types/group';

type Attachment = NonNullable<Group['attachments']>[number];

const props = defineProps<{ group: Group }>();
const { t } = useI18n();
const settingsStore = useSettingsStore();
const optionsStore = useMediaOptionsStore();
const baseId = useId();
const countId = `${baseId}-count`;
const failed = reactive(new Set<string>());
const attachments = computed(() => props.group.attachments ?? []);
const selected = computed(() => optionsStore.getAttachmentSelection(props.group.id));
const allSelected = computed(() => selected.value.length === attachments.value.length
  ? true
  : selected.value.length > 0 ? 'indeterminate' : false);

const isSelected = (id: string) => selected.value.includes(id);
const preview = (attachment: Attachment) => {
  const url = attachment.thumbnail || (attachment.kind === 'image' ? attachment.url : undefined);
  return remoteImagesAllowed(settingsStore.settings.network, url) ? url : undefined;
};
const label = (kind: Attachment['kind'], index: number) => t(kind === 'image' ? 'attachments.photo' : 'attachments.video', { number: index + 1 });

function selectAll(checked: boolean) {
  optionsStore.setAttachmentSelection(props.group.id, checked ? attachments.value.map(attachment => attachment.id) : []);
}

function toggle(id: string, checked: boolean) {
  optionsStore.setAttachmentSelection(props.group.id, checked
    ? [...selected.value, id]
    : selected.value.filter(selectedId => selectedId !== id));
}
</script>
