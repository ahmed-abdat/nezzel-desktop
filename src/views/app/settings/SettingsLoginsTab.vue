<template>
  <div>
    <p class="mb-5 text-[12.5px] leading-relaxed text-muted-foreground text-pretty">{{ t('prefs.logins.note') }}</p>

    <settings-row for-id="pref-cookie-browser" :label="t('prefs.logins.browser')" :help="t('prefs.logins.browserHint')">
      <SelectField id="pref-cookie-browser" v-model="browser" :disabled="neverUse" :options="browserOptions" />
    </settings-row>
    <settings-row for-id="pref-cookie-never" :label="t('prefs.logins.never')">
      <Switch id="pref-cookie-never" v-model="neverUse" />
    </settings-row>

    <settings-section :title="t('prefs.sections.otherLogins')" class="mt-8">
      <settings-row :label="t('prefs.logins.file')" :help="t('prefs.logins.fileHint')">
        <base-file-input
          v-model="settings.auth.cookieFile"
          :label="t('prefs.logins.file')"
          :filters="cookieFileFilters"
          :recent-values="recentFiles"
          display="name"
          show-recent
          keep-on-cancel
          @clear-recents="clearRecentFiles"
        />
      </settings-row>
      <settings-row for-id="pref-always-browser" :label="t('auth.cookies.alwaysUseBrowserLogins')" :help="t('accounts.alwaysHint')">
        <Switch id="pref-always-browser" v-model="settings.auth.alwaysUseBrowserLogins" :disabled="neverUse" />
      </settings-row>
      <saved-accounts />
    </settings-section>
  </div>
</template>

<script setup lang="ts">
import BaseFileInput from '../../../components/base/BaseFileInput.vue';
import { computed, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { SelectField } from '@/components/ui/select';
import Switch from '@/components/ui/switch/Switch.vue';
import SavedAccounts from '../../../components/authentication/SavedAccounts.vue';
import SettingsRow from '../../../components/settings/SettingsRow.vue';
import SettingsSection from '../../../components/settings/SettingsSection.vue';
import { Settings } from '../../../tauri/types/config.ts';
import { COOKIE_BROWSER_OPTIONS } from '../../../helpers/cookieBrowsers';
import { COOKIE_BROWSER_AUTOMATIC, COOKIE_BROWSER_NEVER } from '../../../generated/contract';
import { usePreferencesStore } from '../../../stores/preferences';

const settings = defineModel<Settings>({ required: true });
const { t } = useI18n();
const cookieFileFilters = [{ name: 'cookies.txt', extensions: ['txt'] }];

// Recently used cookies.txt files, so switching back is one click instead of a file dialog.
const COOKIE_FILE_RECENTS = 'cookieFile';
const preferencesStore = usePreferencesStore();
const recentFiles = computed(() => preferencesStore.getRecentPaths(COOKIE_FILE_RECENTS));
watch(() => settings.value.auth.cookieFile, (file, previous) => {
  if (file && file !== previous) void preferencesStore.addRecentPath(COOKIE_FILE_RECENTS, file);
});
const clearRecentFiles = () => void preferencesStore.clearRecentPaths(COOKIE_FILE_RECENTS);

// "automatic" detects a signed-in profile after a site refuses. Remember the last
// choice so the never-use switch can be flipped back without losing it.
const lastBrowser = ref(settings.value.auth.cookieBrowser === COOKIE_BROWSER_NEVER ? COOKIE_BROWSER_AUTOMATIC : settings.value.auth.cookieBrowser);
const neverUse = computed<boolean>({
  get: () => settings.value.auth.cookieBrowser === COOKIE_BROWSER_NEVER,
  set: (value) => { settings.value.auth.cookieBrowser = value ? COOKIE_BROWSER_NEVER : lastBrowser.value; },
});
const browserOptions = computed(() => [
  // Shown only while "Never" is on, so the disabled select says what actually happens.
  ...(neverUse.value ? [{ value: COOKIE_BROWSER_NEVER, label: t('prefs.logins.none') }] : []),
  { value: COOKIE_BROWSER_AUTOMATIC, label: t('prefs.logins.automatic') },
  ...COOKIE_BROWSER_OPTIONS,
]);
const browser = computed<string>({
  get: () => settings.value.auth.cookieBrowser,
  set: (value) => {
    lastBrowser.value = value;
    settings.value.auth.cookieBrowser = value;
  },
});
</script>
