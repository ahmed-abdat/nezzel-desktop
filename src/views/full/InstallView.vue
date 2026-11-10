<template>
  <base-sub-nav :backButton="false">
    <template v-slot:default>
      <base-button v-if="installPartialFail" class="btn-primary" @click="ensureBinaries">{{ t('common.retry') }}</base-button>
      <base-button v-if="counting" class="btn-subtle" @click="stopCountdown">{{ t('install.stay') }}</base-button>
      <base-button :disabled="isInstalling" :class="{ 'btn-primary': !installPartialFail, 'btn-subtle': installPartialFail }" to="/">
        <template v-if="isInstalling">
          {{ t('common.pleaseWait') }}
        </template>
        <template v-else-if="!installPartialFail && counting">
          {{ t('common.continue') }} <span class="tabular-nums text-primary-foreground/80">{{ t('common.parenthesis', { content: waitTime }) }}</span> <ArrowRight class="rtl:rotate-180" aria-hidden="true" />
        </template>
        <template v-else-if="installPartialFail">
          {{ t('install.continueWithout') }} <ArrowRight class="rtl:rotate-180" aria-hidden="true" />
        </template>
        <template v-else>
          {{ t('common.continue') }} <ArrowRight class="rtl:rotate-180" aria-hidden="true" />
        </template>
      </base-button>
    </template>
    <template v-slot:title>
      <div class="min-w-0">
        <h1 v-if="isInstalling">{{ t('install.title.installing') }}</h1>
        <h1 v-else-if="checkFailed">{{ t('feedback.toolsCheckFailed') }}</h1>
        <h1 v-else-if="installPartialFail">{{ t('install.title.failed') }}</h1>
        <h1 v-else>{{ t('install.title.installed') }}</h1>
        <p v-if="isInstalling">{{ t('install.subtitle.installing') }}</p>
        <p v-else-if="installPartialFail">{{ t('install.subtitle.failed') }}</p>
        <p v-else>{{ t('install.subtitle.installed') }}</p>
        <!-- Announces the outcome once; the per-tool bars stay silent. -->
        <span class="sr-only" role="status" aria-live="polite">{{ isInstalling ? '' : installPartialFail ? t('install.title.failed') : t('install.title.installed') }}</span>
      </div>
    </template>
  </base-sub-nav>
  <div class="mx-auto w-full max-w-[600px] px-4 py-6">
    <ul class="border-t border-hairline" :aria-busy="isInstalling || undefined">
      <tool-card v-for="[name, tool] in Object.entries(tools)" :key="name" :tool="tool" :name="name"/>
    </ul>
  </div>
</template>

<script setup lang="ts">
import { useBinariesStore } from '../../stores/binaries';
import { ArrowRight } from '@lucide/vue';
import { computed, onBeforeUnmount, onMounted, ref } from 'vue';
import BaseSubNav from '../../components/base/BaseSubNav.vue';
import BaseButton from '../../components/base/BaseButton.vue';
import { useRoute, useRouter } from 'vue-router';
import ToolCard from '../../components/tool-card/ToolCard.vue';
import { useI18n } from 'vue-i18n';

const { t } = useI18n();
const binariesStore = useBinariesStore();
const router = useRouter();

const checkFailed = ref(useRoute().query.checkFailed === '1');
const tools = computed(() => binariesStore.tools);
const isInstalling = ref(!checkFailed.value);
const waitTime = ref(5);
const installPartialFail = ref(checkFailed.value);

onMounted(async () => {
  if (!checkFailed.value) await ensureBinaries();
});

const ensureBinaries = async () => {
  isInstalling.value = true;
  installPartialFail.value = false;
  try {
    if (checkFailed.value || Object.keys(tools.value).length === 0) {
      checkFailed.value = true;
      await binariesStore.check();
      checkFailed.value = false;
    }
    const toEnsure = Object.keys(tools.value).filter(name => tools.value[name].error || tools.value[name].percent < 100);
    for (const name of toEnsure) {
      delete tools.value[name].error;
      tools.value[name].percent = 0;
      tools.value[name].received = 0;
    }
    await binariesStore.ensure(toEnsure);
    if (toEnsure.some(name => tools.value[name].error)) throw new Error('Tool install failed');
    isInstalling.value = false;
    startCountdown();
  } catch (e) {
    installPartialFail.value = true;
    console.error(e);
  } finally {
    isInstalling.value = false;
  }
};

/** 5 s to the app, unless the user stays; never fires after leaving this page. */
const counting = ref(false);
let countdown: ReturnType<typeof setInterval> | undefined;
const startCountdown = () => {
  stopCountdown();
  waitTime.value = 5;
  counting.value = true;
  countdown = setInterval(() => {
    waitTime.value -= 1;
    if (waitTime.value > 0) return;
    stopCountdown();
    void router.push('/');
  }, 1000);
};
function stopCountdown() {
  clearInterval(countdown);
  countdown = undefined;
  counting.value = false;
}
onBeforeUnmount(stopCountdown);
</script>
