<template>
  <div class="contents" :inert="onboarding.showWelcome || undefined">
    <router-view/>
  </div>
  <welcome-sheet v-if="onboarding.showWelcome"/>
  <report-dialog/>
  <the-toaster/>
</template>

<script setup lang="ts">
import TheToaster from './components/TheToaster.vue';
import ReportDialog from './components/ReportDialog.vue';
import { useReportStore } from './stores/report';
import WelcomeSheet from './components/welcome/WelcomeSheet.vue';
import { useOnboardingStore } from './stores/onboarding';
import { useBinariesStore } from './stores/binaries';
import { useRouter } from 'vue-router';
import { watch } from 'vue';
import { useUpdaterStore } from './stores/updater';
import { useStrongholdStore } from './stores/stronghold';
import { useDragDrop } from './composables/useDragDrop.ts';

const router = useRouter();
const binariesStore = useBinariesStore();
const updaterStore = useUpdaterStore();
const strongholdStore = useStrongholdStore();
const onboarding = useOnboardingStore();

useDragDrop();

const checkTools = async () => {
  const toolsToEnsure = await binariesStore.check();
  if (toolsToEnsure.length > 0) {
    await router.push('/install');
  }
};

const checkUpdates = async () => {
  try {
    await updaterStore.check();
  } catch (e) {
    console.warn('Unable to check for updates:', e);
  }
};

void checkTools().catch(() => router.push({ path: '/install', query: { checkFailed: '1' } }));

try {
  void strongholdStore.loadStatus();
} catch (e) {
  console.error(e);
}

void checkUpdates();

// Crash check and the one-time "reports are now on" notice wait for the welcome sheet.
const reportStore = useReportStore();
watch(() => onboarding.showWelcome, (welcome) => {
  if (!welcome) void reportStore.startupChecks();
}, { immediate: true });
</script>
