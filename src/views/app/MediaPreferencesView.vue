<template>
  <article class="pb-12">
    <!-- Same header as the download detail: Back, what this is for, one Download button. -->
    <header class="sticky top-0 z-10 flex h-12 items-center gap-2 border-b border-hairline bg-background/90 px-4 backdrop-blur">
      <Button variant="ghost" size="sm" class="-ms-2 shrink-0" @click="goBack">
        <ChevronLeft class="rtl:rotate-180" aria-hidden="true" />{{ t('common.back') }}
      </Button>
      <h1 class="min-w-0 flex-1 truncate text-start text-nz-meta font-medium" :title="title">
        <bdi dir="auto">{{ title }}</bdi>
      </h1>
      <Button size="sm" class="shrink-0" @click="downloadItem">
        <ArrowDownToLine aria-hidden="true" />{{ isPaused ? t('common.resume') : t('common.download') }}
      </Button>
    </header>

    <div class="mx-auto w-full max-w-[600px] px-6 pt-5">
      <!-- Same segmented control as Settings. -->
      <nav :aria-label="t('perDownload.tabsLabel')" class="mb-6 flex justify-center">
        <div class="inline-flex max-w-full gap-0.5 overflow-x-auto rounded-lg bg-muted p-[3px]">
          <router-link
            v-for="tab in tabs"
            :key="tab.name"
            :to="{ name: tab.name, params: { groupId } }"
            class="shrink-0 rounded-md px-3.5 py-1 text-nz-meta font-medium text-muted-foreground outline-none transition-colors hover:text-foreground focus-visible:outline-solid focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--nz-focus)]"
            :class="{ 'bg-card text-foreground shadow-xs': route.name === tab.name }"
            :aria-current="route.name === tab.name ? 'page' : undefined"
          >{{ tab.label }}</router-link>
        </div>
      </nav>
      <router-view />
    </div>
  </article>
</template>

<script setup lang="ts">
import { computed } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import { useI18n } from 'vue-i18n';
import { ArrowDownToLine, ChevronLeft } from '@lucide/vue';
import { Button } from '../../components/ui/button';
import { useMediaStore } from '../../stores/media/media';
import { useMediaGroupStore } from '../../stores/media/group';
import { useMediaOptionsStore } from '../../stores/media/options.ts';
import { MediaState, useMediaStateStore } from '../../stores/media/state.ts';

const { groupId } = defineProps({
  groupId: {
    type: String,
    required: true,
  },
});

const { t } = useI18n();
const route = useRoute();
const router = useRouter();
const mediaStore = useMediaStore();
const groupStore = useMediaGroupStore();
const stateStore = useMediaStateStore();
const optionsStore = useMediaOptionsStore();

const tabs = computed(() => [
  { name: 'preferences.quality', label: t('media.preferences.tabs.quality') },
  { name: 'preferences.output', label: t('perDownload.tabs.output') },
  { name: 'preferences.subtitles', label: t('media.preferences.tabs.subtitles') },
  { name: 'preferences.network', label: t('media.preferences.tabs.network') },
]);

const title = computed(() => {
  const group = groupStore.findGroupById(groupId);
  return group?.title ?? group?.url ?? '';
});

const isPaused = computed(() => {
  const state = stateStore.getGroupState(groupId);
  return state === MediaState.paused || state === MediaState.pausedList;
});

function goBack() {
  const state = window.history.state as { back?: string | null } | null;
  if (state?.back) router.back();
  else void router.push({ name: 'home' });
}

const downloadItem = (): void => {
  const options = optionsStore.getOptions(groupId);
  if (!options) {
    console.warn(`No options found for group: ${groupId}, cannot download.`);
    return;
  }
  void mediaStore.downloadGroup(groupId, options);
  void router.push(`/`);
};
</script>
