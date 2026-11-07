<template>
  <div>
    <settings-section :title="t('prefs.sections.downloads')">
      <settings-row :label="t('prefs.general.folder')" :help="t('prefs.general.folderHint')">
        <bdi class="block max-w-[14rem] truncate text-[12.5px] text-muted-foreground" :dir="folder ? 'ltr' : undefined" :title="folder">{{ folder ? compactFolderPath(folder) : t('prefs.general.folderDefault') }}</bdi>
        <Button variant="outline" @click="pickFolder">{{ t('prefs.general.change') }}</Button>
      </settings-row>
      <settings-row for-id="pref-choice" :label="t('prefs.general.choice')" :help="t('prefs.general.choiceHint')">
        <SelectField id="pref-choice" v-model="defaultChoice" :options="choiceOptions" />
      </settings-row>
      <settings-row for-id="pref-ask" :label="t('prefs.general.askEach')" :help="t('prefs.general.askEachHint')">
        <Switch id="pref-ask" v-model="askEach" />
      </settings-row>
    </settings-section>

    <settings-section :title="t('prefs.sections.quick')">
      <settings-row for-id="pref-clip" :label="t('prefs.general.clip')" :help="t('prefs.general.clipHint')">
        <Switch id="pref-clip" v-model="settings.input.autoFillClipboard" />
      </settings-row>
      <settings-row for-id="pref-card" :label="t('prefs.general.card')" :help="capturePlatform.backgroundClipboard ? t('prefs.general.cardHint') : t('prefs.general.cardFocusHint')">
        <Switch id="pref-card" v-model="settings.input.watchClipboard" />
      </settings-row>
      <settings-row for-id="pref-tray" :label="t('prefs.general.tray')" :help="t('prefs.general.trayHint')">
        <Switch id="pref-tray" v-model="settings.system.trayEnabled" />
      </settings-row>
    </settings-section>

    <settings-section :title="t('prefs.sections.appearance')">
      <settings-row for-id="pref-language" :label="t('prefs.general.language')">
        <SelectField id="pref-language" v-model="settings.appearance.language" :options="languageSelectOptions" />
      </settings-row>
      <settings-row for-id="pref-theme" :label="t('prefs.general.theme')">
        <SelectField id="pref-theme" v-model="settings.appearance.theme" :options="themeOptions" />
      </settings-row>
    </settings-section>
  </div>
</template>

<script setup lang="ts">
import { computed, unref } from 'vue';
import { useI18n } from 'vue-i18n';
import { Button } from '@/components/ui/button';
import { SelectField } from '@/components/ui/select';
import Switch from '@/components/ui/switch/Switch.vue';
import SettingsRow from '../../../components/settings/SettingsRow.vue';
import SettingsSection from '../../../components/settings/SettingsSection.vue';
import { Settings } from '../../../tauri/types/config.ts';
import { useCapturePlatform } from '../../../composables/useCapturePlatform';
import { useUiPrefs } from '../../../composables/useUiPrefs';
import { useTheme } from '../../../composables/useTheme';
import { useDialog } from '../../../composables/useDialog';
import { appLanguageOptions } from '../../../helpers/languageOptions';
import { getDefaultLocale } from '../../../i18n';
import { languageOptionsLookup } from '../../../helpers/subtitles/languages';
import { compactFolderPath } from '../../../helpers/folderPath';

const settings = defineModel<Settings>({ required: true });
const { t } = useI18n();
const capturePlatform = useCapturePlatform();
const { skipOptionsSheet, defaultChoice } = useUiPrefs();
const { systemTheme } = useTheme();
const { openPathDialog } = useDialog();

const askEach = computed({
  get: () => !skipOptionsSheet.value,
  set: (value: boolean) => { skipOptionsSheet.value = !value; },
});

const folder = computed(() => settings.value.output.downloadDir ?? '');
const languages = appLanguageOptions();
const systemLanguage = computed(() => languageOptionsLookup.get(getDefaultLocale())?.nativeName ?? getDefaultLocale());
const choiceOptions = computed(() => [
  { value: 'best', label: t('sheet.best') },
  { value: 'small', label: t('sheet.small') },
  { value: 'audio', label: t('sheet.audio') },
]);
const languageSelectOptions = computed(() => [
  { value: 'system', label: t('settings.appearance.language.options.system', { language: systemLanguage.value }) },
  ...languages.map(option => ({ value: option.value, label: option.label })),
]);
const themeOptions = computed(() => [
  { value: 'system', label: t('settings.appearance.theme.options.system', { theme: unref(systemTheme) === 'dark' ? t('settings.appearance.theme.options.dark') : t('settings.appearance.theme.options.light') }) },
  { value: 'light', label: t('settings.appearance.theme.options.light') },
  { value: 'dark', label: t('settings.appearance.theme.options.dark') },
]);

async function pickFolder() {
  const picked = await openPathDialog(false, true);
  if (typeof picked === 'string' && picked.length > 0) {
    settings.value.output.downloadDir = picked;
  }
}
</script>
