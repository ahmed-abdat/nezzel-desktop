<template>
  <!-- Category list + one column, anchored at the inline start next to the app sidebar; spare width stays empty
       at the end (System Settings / Finder), never centred. -->
  <div class="flex w-full items-start gap-8 px-6 pb-12 pt-6">
    <nav :aria-label="t('prefs.tabs.label')" class="sticky top-6 flex w-[176px] shrink-0 flex-col gap-0.5 self-start">
      <router-link
        v-for="tab in tabs"
        :key="tab.label"
        :to="tab.to"
        class="flex h-8 items-center gap-2.5 rounded-[6px] px-2.5 text-[13px] text-foreground/80 outline-none transition-colors duration-100 hover:bg-foreground/[0.04] focus-visible:outline-solid focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--nz-focus)]"
        :class="{ 'bg-foreground/[0.07] font-medium text-foreground hover:bg-foreground/[0.07]': tab.active }"
        :aria-current="tab.active ? 'page' : undefined"
        @click="searchQuery = ''"
      >
        <component :is="tab.icon" class="size-4 shrink-0" :class="tab.active ? 'text-primary' : 'text-muted-foreground'" aria-hidden="true" />
        <span class="truncate">{{ tab.label }}</span>
      </router-link>
    </nav>
    <div ref="content" class="min-w-0 max-w-[640px] flex-1">
      <settings-search v-model="searchQuery" @select="goToSetting" />
      <div v-show="!searchQuery.trim()" class="relative mb-3 flex h-8 items-center">
        <h2 class="text-[18px] font-semibold rtl:text-[20px]">{{ activeTab?.label }}</h2>
        <Transition
          enter-active-class="transition-opacity duration-150"
          leave-active-class="transition-opacity duration-150"
          enter-from-class="opacity-0"
          leave-to-class="opacity-0"
        >
          <span v-if="justSaved" class="absolute end-0 top-1/2 inline-flex -translate-y-1/2 items-center gap-1 text-[12px] font-medium text-primary" aria-hidden="true">
            <Check class="size-3.5" aria-hidden="true" />{{ t('prefs.saved') }}
          </span>
        </Transition>
        <!-- Always mounted, so screen readers hear the text change (a live region inserted with its text is often missed). -->
        <span class="sr-only" role="status" aria-live="polite">{{ justSaved ? t('prefs.saved') : '' }}</span>
      </div>
      <div v-show="!searchQuery.trim()">
        <router-view v-slot="{ Component }">
          <component :is="Component" v-model="draft" @reset="resetSettings" />
        </router-view>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, toRaw, useTemplateRef, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { useRoute, useRouter } from 'vue-router';
import { Check, Info, KeyRound, Settings2, SlidersHorizontal } from '@lucide/vue';
import { useSettingsStore } from '../../stores/settings';
import { useToastStore } from '../../stores/toast';
import { useTheme } from '../../composables/useTheme.ts';
import { Settings } from '../../tauri/types/config.ts';
import SettingsSearch from '../../components/settings/SettingsSearch.vue';
import { SETTINGS_TABS, type SettingsIndexEntry } from '../../components/settings/settingsIndex';
import { revealSetting } from '../../composables/useSettingsReveal';

const settingsStore = useSettingsStore();
const toastStore = useToastStore();
const { setTheme } = useTheme();
const { t } = useI18n();
const route = useRoute();
const router = useRouter();

const searchQuery = ref('');
const content = useTemplateRef<HTMLElement>('content');
// Picking a category in the list leaves the search.
watch(() => route.name, () => {
  searchQuery.value = '';
});

/** A search result: open its tab, expand what hides it, scroll to it and highlight it. */
async function goToSetting(entry: SettingsIndexEntry) {
  const tab = SETTINGS_TABS.find(item => item.tab === entry.tab);
  if (tab && route.name !== tab.route) await router.push({ name: tab.route });
  await nextTick();
  if (content.value) await revealSetting(content.value, entry.id, entry.parent);
}

const activeTab = computed(() => tabs.value.find(tab => tab.active));
// While searching, results span every tab: no category reads as selected.
const isTab = (name: string) => route.name === name && !searchQuery.value.trim();
const tabs = computed(() => [
  { icon: Settings2, label: t('prefs.tabs.general'), to: { name: 'settings.general' }, active: isTab('settings.general') },
  { icon: KeyRound, label: t('prefs.tabs.logins'), to: { name: 'settings.logins' }, active: isTab('settings.logins') },
  { icon: SlidersHorizontal, label: t('prefs.tabs.advanced'), to: { name: 'settings.advanced' }, active: isTab('settings.advanced') },
  { icon: Info, label: t('about.title'), to: { name: 'settings.about' }, active: isTab('settings.about') },
]);

const draft = ref<Settings>(structuredClone<Settings>(toRaw(settingsStore.settings)));
const justSaved = ref(false);
const SAVE_DELAY_MS = 450;
let saveTimer: ReturnType<typeof setTimeout> | undefined;
let savedTimer: ReturnType<typeof setTimeout> | undefined;
let suppressNextSave = false;
let saveChain = Promise.resolve();

const hasChanges = () => JSON.stringify(draft.value) !== JSON.stringify(settingsStore.settings);

function save() {
  clearTimeout(saveTimer);
  saveTimer = undefined;
  saveChain = saveChain.then(async () => {
    if (!hasChanges()) return;
    const snapshot = structuredClone(toRaw(draft.value));
    const themeChanged = snapshot.appearance.theme !== settingsStore.settings.appearance.theme;
    try {
      await settingsStore.patch(snapshot);
      if (themeChanged && snapshot.appearance.theme) setTheme(snapshot.appearance.theme);
      // A response for an earlier draft must not claim newer edits were saved.
      if (JSON.stringify(snapshot) !== JSON.stringify(draft.value)) return;
      justSaved.value = true;
      clearTimeout(savedTimer);
      savedTimer = setTimeout(() => {
        justSaved.value = false;
      }, 1600);
    } catch {
      justSaved.value = false;
      toastStore.showToast(t('feedback.saveFailed'), { style: 'error' });
    }
  });
  return saveChain;
}

// Settings save themselves shortly after each change; no Save button to forget.
watch(draft, () => {
  justSaved.value = false;
  clearTimeout(savedTimer);
  if (suppressNextSave) {
    suppressNextSave = false;
    return;
  }
  clearTimeout(saveTimer);
  saveTimer = setTimeout(() => void save(), SAVE_DELAY_MS);
}, { deep: true });

async function resetSettings() {
  clearTimeout(saveTimer);
  const next = await settingsStore.reset();
  suppressNextSave = true;
  draft.value = structuredClone<Settings>(toRaw(next));
  if (next.appearance.theme) setTheme(next.appearance.theme);
  toastStore.showToast(t('settings.toasts.reset'), { style: 'success' });
}

onBeforeUnmount(() => {
  if (saveTimer) {
    clearTimeout(saveTimer);
    void save();
  }
  clearTimeout(savedTimer);
});

defineExpose({ flush: save });
</script>
