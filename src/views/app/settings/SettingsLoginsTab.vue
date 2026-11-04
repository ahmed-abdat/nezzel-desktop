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
        <bdi v-if="settings.auth.cookieFile" class="block max-w-[10rem] truncate font-mono text-[12px] text-muted-foreground" dir="ltr" :title="settings.auth.cookieFile">{{ fileName }}</bdi>
        <tooltip v-if="settings.auth.cookieFile" :content="t('components.base.fileInput.clear')">
          <Button
            variant="ghost"
            size="icon-xs"
            :aria-label="t('components.base.fileInput.clear')"
            @click="settings.auth.cookieFile = null"
          >
            <X />
          </Button>
        </tooltip>
        <Button variant="outline" @click="pickFile">{{ settings.auth.cookieFile ? t('prefs.general.change') : t('prefs.general.choose') }}</Button>
        <DropdownMenu v-if="recentFiles.length > 0">
          <DropdownMenuTrigger as-child>
            <Button
              variant="ghost"
              size="icon-xs"
              class="text-muted-foreground"
              :aria-label="t('components.base.fileInput.recents.open')"
            >
              <History aria-hidden="true" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent class="max-w-96">
            <DropdownMenuItem v-for="recent in recentFiles" :key="recent" :title="recent" @select="settings.auth.cookieFile = recent">
              <bdi class="min-w-0 truncate" dir="ltr">{{ baseName(recent) }}</bdi>
            </DropdownMenuItem>
            <DropdownMenuSeparator />
            <DropdownMenuItem danger @select="clearRecentFiles">
              <Trash2 aria-hidden="true" />
              {{ t('components.base.fileInput.recents.clear') }}
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </settings-row>
      <settings-row for-id="pref-always-browser" :label="t('auth.cookies.alwaysUseBrowserLogins')" :help="t('accounts.alwaysHint')">
        <Switch id="pref-always-browser" v-model="settings.auth.alwaysUseBrowserLogins" :disabled="neverUse" />
      </settings-row>
      <saved-accounts />
    </settings-section>
  </div>
</template>

<script setup lang="ts">
import Tooltip from '../../../components/ui/tooltip/Tooltip.vue';
import { computed, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { History, Trash2, X } from '@lucide/vue';
import { Button } from '@/components/ui/button';
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger } from '@/components/ui/dropdown-menu';
import { SelectField } from '@/components/ui/select';
import Switch from '@/components/ui/switch/Switch.vue';
import SavedAccounts from '../../../components/authentication/SavedAccounts.vue';
import SettingsRow from '../../../components/settings/SettingsRow.vue';
import SettingsSection from '../../../components/settings/SettingsSection.vue';
import { Settings } from '../../../tauri/types/config.ts';
import { COOKIE_BROWSERS } from '../../../helpers/cookieBrowsers';
import { useDialog } from '../../../composables/useDialog';
import { usePreferencesStore } from '../../../stores/preferences';

const settings = defineModel<Settings>({ required: true });
const { t } = useI18n();
const { openPathDialog } = useDialog();
const browsers = COOKIE_BROWSERS;
const baseName = (path: string) => path.split(/[\\/]/).pop() ?? path;
const fileName = computed(() => (settings.value.auth.cookieFile ? baseName(settings.value.auth.cookieFile) : ''));

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
const lastBrowser = ref(settings.value.auth.cookieBrowser === 'none' ? 'automatic' : settings.value.auth.cookieBrowser);
const neverUse = computed<boolean>({
  get: () => settings.value.auth.cookieBrowser === 'none',
  set: (value) => { settings.value.auth.cookieBrowser = value ? 'none' : lastBrowser.value; },
});
const browserOptions = computed(() => [
  // Shown only while "Never" is on, so the disabled select says what actually happens.
  ...(neverUse.value ? [{ value: 'none', label: t('prefs.logins.none') }] : []),
  { value: 'automatic', label: t('prefs.logins.automatic') },
  ...browsers.map(name => ({ value: name.toLowerCase(), label: name })),
]);
const browser = computed<string>({
  get: () => settings.value.auth.cookieBrowser,
  set: (value) => {
    lastBrowser.value = value;
    settings.value.auth.cookieBrowser = value;
  },
});

async function pickFile() {
  const picked = await openPathDialog(false, false, [{ name: 'cookies.txt', extensions: ['txt'] }]);
  if (typeof picked === 'string' && picked.length > 0) settings.value.auth.cookieFile = picked;
}
</script>
