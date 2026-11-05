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
      >
        <component :is="tab.icon" class="size-4 shrink-0" :class="tab.active ? 'text-primary' : 'text-muted-foreground'" aria-hidden="true" />
        <span class="truncate">{{ tab.label }}</span>
      </router-link>
    </nav>
    <div class="min-w-0 max-w-[640px] flex-1">
      <div class="relative mb-3 flex h-8 items-center">
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
      <router-view v-slot="{ Component }">
        <component :is="Component" v-model="draft" @reset="resetSettings" />
      </router-view>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed, onBeforeUnmount, ref, toRaw, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { useRoute } from 'vue-router';
import { Check, KeyRound, Settings2, SlidersHorizontal } from '@lucide/vue';
import { useSettingsStore } from '../../stores/settings';
import { useToastStore } from '../../stores/toast';
import { useTheme } from '../../composables/useTheme.ts';
import { Settings } from '../../tauri/types/config.ts';

const settingsStore = useSettingsStore();
const toastStore = useToastStore();
const { setTheme } = useTheme();
const { t } = useI18n();
const route = useRoute();

const activeTab = computed(() => tabs.value.find(tab => tab.active));
const tabs = computed(() => [
  { icon: Settings2, label: t('prefs.tabs.general'), to: { name: 'settings.general' }, active: route.name === 'settings.general' },
  { icon: KeyRound, label: t('prefs.tabs.logins'), to: { name: 'settings.logins' }, active: route.name === 'settings.logins' },
  { icon: SlidersHorizontal, label: t('prefs.tabs.advanced'), to: { name: 'settings.advanced' }, active: route.name === 'settings.advanced' },
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
