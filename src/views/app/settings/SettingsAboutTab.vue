<template>
  <settings-section :title="t('about.title')">
    <settings-row>
      <template #label>
        <a href="https://nezzel.hululsoft.mr" class="text-primary hover:underline" target="_blank" rel="noopener">{{ t('about.links.website') }}</a>
      </template>
    </settings-row>
    <settings-row>
      <template #label>
        <a href="https://github.com/ahmed-abdat/nezzel-desktop" class="text-primary hover:underline" target="_blank" rel="noopener">{{ t('about.links.github') }}</a>
      </template>
    </settings-row>
    <settings-row :label="t('report.title')" :help="t('report.aboutHelp')">
      <Button variant="outline" data-testid="about-report" @click="report.openReport()">
        <MessageSquareWarning aria-hidden="true" />{{ t('report.open') }}
      </Button>
    </settings-row>
    <settings-row :label="t('about.poweredBy')" :help="t('about.credit')">
      <Button variant="outline" @click="openLicenses()">{{ t('about.viewLicenses') }}</Button>
    </settings-row>
    <settings-row :label="t('about.version', { version: `\u2066${appVersion}\u2069` })" :help="checkHelp">
      <Button variant="outline" data-testid="about-check" :disabled="checking" :aria-busy="checking || undefined" @click="checkNow">
        <LoaderCircle v-if="checking" class="animate-spin" aria-hidden="true" />{{ checking ? t('about.update.checking') : t('about.update.check') }}
      </Button>
    </settings-row>
    <span class="sr-only" role="status" aria-live="polite">{{ checkHelp }}</span>
  </settings-section>
</template>

<script setup lang="ts">
import { computed, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import { LoaderCircle, MessageSquareWarning } from '@lucide/vue';
import { useReportStore } from '../../../stores/report';
import { useUpdaterStore } from '../../../stores/updater';
import { Button } from '@/components/ui/button';
import SettingsRow from '../../../components/settings/SettingsRow.vue';
import SettingsSection from '../../../components/settings/SettingsSection.vue';
import { useOpener } from '../../../composables/useOpener.ts';

defineOptions({
  inheritAttrs: false,
});

const { t } = useI18n();
const { openInternalPath } = useOpener();
const appVersion = typeof __APP_VERSION__ === 'undefined' ? '' : __APP_VERSION__;

const updater = useUpdaterStore();
const report = useReportStore();
const checking = ref(false);
const checkResult = ref<'latest' | 'available' | 'failed'>();
const checkHelp = computed(() => (checkResult.value ? t(`about.update.${checkResult.value}`) : undefined));
async function checkNow() {
  checking.value = true;
  checkResult.value = undefined;
  try {
    checkResult.value = (await updater.check(true))?.available ? 'available' : 'latest';
  } catch {
    checkResult.value = 'failed';
  } finally {
    checking.value = false;
  }
}

const openLicenses = async () => {
  await openInternalPath('licenses/3rdpartylicenses.txt');
};
</script>
