<template>
  <form @submit.prevent="save">
    <base-sub-nav>
      <template #default>
        <base-button type="submit" class="btn-primary" :disabled="!hasChanges" :loading="isSaving">
          {{ t('common.save') }}
        </base-button>
      </template>
      <template #title>
        <h1>{{ t('prefs.adv.files.folders') }}</h1>
      </template>
    </base-sub-nav>

    <div class="mx-auto w-full max-w-[600px] px-6 pb-12 pt-5">
      <settings-section :title="t('folders.where')">
        <settings-row :label="t('prefs.general.folder')" :help="t('prefs.general.folderHint')">
          <base-file-input
            v-model="settingsDraft.output.downloadDir"
            :label="t('prefs.general.folder')"
            :placeholder="t('prefs.general.folderDefault')"
            :recent-values="recentValues.downloadDir"
            :clearable="false"
            show-recent
            directory
            @clear-recents="clearRecents('downloadDir')"
          />
        </settings-row>
        <preferences-directory
          v-model="preferencesDraft"
          :recent-values="recentValues"
          @clear-recents="clearRecents"
        />
      </settings-section>
      <settings-section :title="t('folders.names')">
        <preferences-filename v-model="settingsDraft" />
      </settings-section>
    </div>
  </form>
</template>
<script setup lang="ts">
import BaseSubNav from '../../components/base/BaseSubNav.vue';
import BaseButton from '../../components/base/BaseButton.vue';
import { useSettingsStore } from '../../stores/settings';
import { computed, ref, toRaw, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { Settings } from '../../tauri/types/config.ts';
import BaseFileInput from '../../components/base/BaseFileInput.vue';
import SettingsRow from '../../components/settings/SettingsRow.vue';
import SettingsSection from '../../components/settings/SettingsSection.vue';
import { usePreferencesStore } from '../../stores/preferences';
import { Preferences } from '../../tauri/types/preferences';
import PreferencesFilename from '../../components/preferences/PreferencesFilename.vue';
import PreferencesDirectory from '../../components/preferences/PreferencesDirectory.vue';
import { useToastStore } from '../../stores/toast';

const settingsStore = useSettingsStore();
const preferencesStore = usePreferencesStore();
const toastStore = useToastStore();
const { t } = useI18n();

const settingsDraft = ref<Settings>(structuredClone<Settings>(toRaw(settingsStore.settings)));
const preferencesDraft = ref<Preferences>(structuredClone<Preferences>(toRaw(preferencesStore.preferences)));

const isSaving = ref(false);

const hasSettingsChanges = computed(() => {
  return JSON.stringify(settingsDraft.value) !== JSON.stringify(settingsStore.settings);
});
const hasPathPreferencesChanges = computed(() => {
  return JSON.stringify(preferencesDraft.value.paths) !== JSON.stringify(preferencesStore.preferences.paths);
});
const hasChanges = computed(() => {
  return hasSettingsChanges.value || hasPathPreferencesChanges.value;
});

const recentValues = computed(() => ({
  downloadDir: preferencesStore.getRecentPaths('downloadDir'),
  videoDir: preferencesStore.getRecentPaths('videoDir'),
  audioDir: preferencesStore.getRecentPaths('audioDir'),
}));

watch(
  () => settingsDraft.value.output.downloadDir,
  (newFile, oldFile) => {
    if (newFile && newFile !== oldFile) {
      void preferencesStore.addRecentPath('downloadDir', newFile);
    }
  },
);

const clearRecents = (label: string) => {
  void preferencesStore.clearRecentPaths(label);
};

const save = async () => {
  isSaving.value = true;
  try {
    await settingsStore.patch(settingsDraft.value);
    await preferencesStore.patch({ paths: preferencesDraft.value.paths });
    toastStore.showToast(t('location.toasts.saved'), { style: 'success' });
  } catch (e) {
    console.error(e);
    toastStore.showToast(t('location.toasts.error'), { style: 'error' });
  } finally {
    isSaving.value = false;
  }
};
</script>
