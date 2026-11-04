<template>
  <settings-section :title="t('settings.sponsorBlock.legend')" :note="t('prefs.adv.sb.note')">
    <settings-row for-id="sb-remove" :label="t('prefs.adv.sb.remove')" :help="t('prefs.adv.sb.removeHint')">
      <Switch id="sb-remove" v-model="removeOn" />
    </settings-row>
    <settings-row :label="t('prefs.adv.sb.choose')" :help="summary">
      <dropdown-menu>
        <dropdown-menu-trigger as-child>
          <Button variant="outline">{{ t('prefs.adv.sb.chooseButton') }}</Button>
        </dropdown-menu-trigger>
        <dropdown-menu-content align="end" class="max-h-[60vh] w-72 overflow-y-auto">
          <dropdown-menu-label>{{ t('prefs.adv.sb.removeGroup') }}</dropdown-menu-label>
          <dropdown-menu-checkbox-item
            v-for="part in removable"
            :key="`r-${part.value}`"
            :model-value="settings.sponsorBlock.removeParts.includes(part.value)"
            @update:model-value="toggle('removeParts', part.value, $event)"
          >{{ part.label }}</dropdown-menu-checkbox-item>
          <dropdown-menu-separator />
          <dropdown-menu-label>{{ t('prefs.adv.sb.markGroup') }}</dropdown-menu-label>
          <dropdown-menu-checkbox-item
            v-for="part in markable"
            :key="`m-${part.value}`"
            :model-value="settings.sponsorBlock.markParts.includes(part.value)"
            @update:model-value="toggle('markParts', part.value, $event)"
          >{{ part.label }}</dropdown-menu-checkbox-item>
        </dropdown-menu-content>
      </dropdown-menu>
    </settings-row>

    <template #more>
      <settings-row for-id="sponsorblockPreciseCuts" :label="t('prefs.adv.sb.precise')" :help="t('prefs.adv.sb.preciseHint')">
        <Switch id="sponsorblockPreciseCuts" v-model="settings.output.preciseCuts" />
      </settings-row>
      <settings-row for-id="sponsorblockApiUrl" :label="t('prefs.adv.sb.server')" :help="t('prefs.adv.sb.serverHint')">
        <Input
          id="sponsorblockApiUrl"
          v-model="settings.sponsorBlock.apiUrl"
          type="text"
          dir="ltr"
          spellcheck="false"
          class="w-56 font-mono text-[12px]"
          placeholder="https://sponsor.ajay.app" />
      </settings-row>
    </template>
  </settings-section>
</template>

<script setup lang="ts">
import { Input } from '../ui/input';
import { computed, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import { Button } from '@/components/ui/button';
import { DropdownMenu, DropdownMenuCheckboxItem, DropdownMenuContent, DropdownMenuLabel, DropdownMenuSeparator, DropdownMenuTrigger } from '@/components/ui/dropdown-menu';
import Switch from '@/components/ui/switch/Switch.vue';
import SettingsRow from './SettingsRow.vue';
import SettingsSection from './SettingsSection.vue';
import { Settings } from '../../tauri/types/config.ts';

const i18n = useI18n();
const t = i18n.t;
const settings = defineModel<Settings>({ required: true });

const PARTS = ['sponsor', 'selfpromo', 'interaction', 'intro', 'outro', 'music_offtopic', 'preview', 'filler', 'chapter', 'hook', 'poi_highlight'];
const localized = computed(() => {
  const parts: Record<string, { label: string; description: string }> = i18n.tm('settings.sponsorBlock.parts');
  return parts;
});
const markable = computed(() => PARTS.map(part => ({ value: part, label: localized.value[part]?.label ?? part })));
const removable = computed(() => markable.value.filter(part => !['chapter', 'poi_highlight'].includes(part.value)));

// The switch is "remove anything at all"; turning it back on restores the last choice (sponsors by default).
const lastRemove = ref<string[]>(settings.value.sponsorBlock.removeParts.length ? [...settings.value.sponsorBlock.removeParts] : ['sponsor']);
const removeOn = computed<boolean>({
  get: () => settings.value.sponsorBlock.removeParts.length > 0,
  set: (on) => {
    if (on) settings.value.sponsorBlock.removeParts = [...lastRemove.value];
    else {
      lastRemove.value = [...settings.value.sponsorBlock.removeParts];
      settings.value.sponsorBlock.removeParts = [];
    }
  },
});

function toggle(list: 'removeParts' | 'markParts', part: string, on: boolean) {
  const current = settings.value.sponsorBlock[list].filter(value => value !== part);
  settings.value.sponsorBlock[list] = on === true ? [...current, part] : current;
}

const summary = computed(() => {
  const removed = settings.value.sponsorBlock.removeParts.length;
  const marked = settings.value.sponsorBlock.markParts.length;
  return t('prefs.adv.sb.summary', { removed, marked });
});
</script>
