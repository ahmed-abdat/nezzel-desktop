<template>
  <div>
    <p class="mb-5 text-[12.5px] leading-relaxed text-muted-foreground text-pretty">{{ t('prefs.logins.note') }}</p>

    <settings-row for-id="pref-login-policy" :label="t('prefs.logins.policy')">
      <SelectField id="pref-login-policy" v-model="policy" :options="policyOptions" />
    </settings-row>
    <settings-row for-id="pref-cookie-browser" :label="t('prefs.logins.browser')" :help="t('prefs.logins.browserHint')">
      <SelectField id="pref-cookie-browser" v-model="browser" :disabled="policy === 'never'" :options="browserOptions" />
    </settings-row>

    <saved-accounts class="mt-8" />
    <Collapsible class="mt-8">
      <CollapsibleTrigger as-child>
        <Button variant="ghost" class="group gap-1.5">{{ t('prefs.tabs.advanced') }}<ChevronDown class="size-4 transition-transform group-data-[state=open]:rotate-180" aria-hidden="true" /></Button>
      </CollapsibleTrigger>
      <CollapsibleContent>
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
        </settings-section>
      </CollapsibleContent>
    </Collapsible>
  </div>
</template>

<script setup lang="ts">
import { ChevronDown } from '@lucide/vue';
import BaseFileInput from '../../../components/base/BaseFileInput.vue';
import { computed, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { SelectField } from '@/components/ui/select';
import { Button } from '@/components/ui/button';
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from '@/components/ui/collapsible';
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

// Adapt legacy values without changing the backend contract; Never wins over a stale Always flag.
// An empty stored browser means Never in Rust (BrowserSetting::of), so read it the same way here.
const isNever = (value: string) => value === COOKIE_BROWSER_NEVER || value === '';
const lastBrowser = ref(isNever(settings.value.auth.cookieBrowser) ? COOKIE_BROWSER_AUTOMATIC : settings.value.auth.cookieBrowser);
const policy = computed<string>({
  get: () => isNever(settings.value.auth.cookieBrowser) ? 'never' : settings.value.auth.alwaysUseBrowserLogins ? 'always' : 'ask',
  set: (value) => {
    if (!isNever(settings.value.auth.cookieBrowser)) lastBrowser.value = settings.value.auth.cookieBrowser;
    settings.value.auth.cookieBrowser = value === 'never' ? COOKIE_BROWSER_NEVER : lastBrowser.value;
    settings.value.auth.alwaysUseBrowserLogins = value === 'always';
  },
});
const policyOptions = computed(() => ['ask', 'never', 'always'].map(value => ({ value, label: t(`prefs.logins.${value}Choice`) })));
const browserOptions = computed(() => [
  { value: COOKIE_BROWSER_AUTOMATIC, label: t('prefs.logins.automatic') },
  ...COOKIE_BROWSER_OPTIONS,
]);
const browser = computed<string>({
  get: () => policy.value === 'never' ? lastBrowser.value : settings.value.auth.cookieBrowser,
  set: (value) => {
    lastBrowser.value = value;
    if (policy.value !== 'never') settings.value.auth.cookieBrowser = value;
  },
});
</script>
