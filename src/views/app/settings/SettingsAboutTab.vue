<template>
  <!-- The app: icon, name, what it does, version. -->
  <settings-section>
    <div class="flex items-center gap-3 border-b border-hairline pb-4">
      <!-- The icon file carries its own margin: pull it to the column edge. -->
      <img :src="nezzelIcon" alt="" class="-ms-1.5 size-16 shrink-0" />
      <div class="min-w-0">
        <p class="text-[15px] font-semibold rtl:text-[16px]">{{ t('shell.brand') }}</p>
        <p class="mt-0.5 text-[12.5px] leading-relaxed text-muted-foreground text-pretty rtl:text-[13px]">{{ t('welcome.title') }}</p>
      </div>
    </div>
    <settings-row :label="t('about.version', { version: `\u2066${appVersion}\u2069` })" :help="checkHelp">
      <Button variant="outline" data-testid="about-check" :disabled="checking" :aria-busy="checking || undefined" @click="checkNow">
        <LoaderCircle v-if="checking" class="animate-spin" aria-hidden="true" />{{ checking ? t('about.update.checking') : t('about.update.check') }}
      </Button>
    </settings-row>
    <span class="sr-only" role="status" aria-live="polite">{{ checkHelp }}</span>
  </settings-section>

  <!-- Who built it. Flat block, no card: mark + name, tagline, one line, two ways out. -->
  <settings-section :title="t('about.hululsoft.madeBy')">
    <div class="pb-1 pt-2" data-testid="about-hululsoft">
      <div class="flex items-center gap-3">
        <img :src="hululsoftMark" alt="" class="size-9 shrink-0" />
        <div class="min-w-0">
          <p class="text-[15px] font-semibold leading-tight rtl:text-[16px]">{{ t('about.hululsoft.name') }}</p>
          <p class="mt-0.5 text-[12.5px] text-muted-foreground rtl:text-[13px]">{{ t('about.hululsoft.tagline') }}</p>
        </div>
      </div>
      <p class="mt-3 max-w-[62ch] text-[13px] leading-relaxed text-pretty rtl:text-[14px]">{{ t('about.hululsoft.whoWeAre') }}</p>
      <div class="mt-3.5 flex flex-wrap gap-2">
        <Button as="a" :href="HULULSOFT_URL" target="_blank" rel="noopener" variant="outline">
          {{ t('about.hululsoft.visit') }}<ExternalLink class="text-muted-foreground" aria-hidden="true" />
        </Button>
        <Button as="a" :href="`${HULULSOFT_URL}/contact`" target="_blank" rel="noopener" variant="outline">
          {{ t('about.hululsoft.workWithUs') }}<ExternalLink class="text-muted-foreground" aria-hidden="true" />
        </Button>
      </div>
    </div>
  </settings-section>

  <settings-section :title="t('about.hululsoft.more')">
    <a :href="NEZZEL_URL" :class="linkRowCls" target="_blank" rel="noopener">
      <span>{{ t('about.hululsoft.nezzelAndroid') }}</span><ExternalLink class="size-4 shrink-0 text-muted-foreground group-hover:text-primary" aria-hidden="true" />
    </a>
    <ul class="flex flex-wrap items-center gap-x-2 gap-y-1 pt-3 text-[12.5px] text-muted-foreground rtl:text-[13px]">
      <template v-for="(product, i) in OTHER_PRODUCTS" :key="product">
        <li v-if="i" class="nz-sep" aria-hidden="true" />
        <li><bdi>{{ product }}</bdi></li>
      </template>
    </ul>
  </settings-section>

  <settings-section>
    <a v-for="link in links" :key="link.href" :href="link.href" :class="linkRowCls" target="_blank" rel="noopener">
      <span>{{ link.label }}</span><ExternalLink class="size-4 shrink-0 text-muted-foreground group-hover:text-primary" aria-hidden="true" />
    </a>
    <settings-row :label="t('report.title')" :help="t('report.aboutHelp')">
      <Button variant="outline" data-testid="about-report" @click="report.openReport()">
        <MessageSquareWarning aria-hidden="true" />{{ t('report.open') }}
      </Button>
    </settings-row>
    <settings-row :label="t('about.poweredBy')" :help="t('about.credit')">
      <Button variant="outline" @click="openLicenses()">{{ t('about.viewLicenses') }}</Button>
    </settings-row>
  </settings-section>
</template>

<script setup lang="ts">
import { computed, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import { ExternalLink, LoaderCircle, MessageSquareWarning } from '@lucide/vue';
import { useReportStore } from '../../../stores/report';
import { useUpdaterStore } from '../../../stores/updater';
import { Button } from '@/components/ui/button';
import SettingsRow from '../../../components/settings/SettingsRow.vue';
import SettingsSection from '../../../components/settings/SettingsSection.vue';
import { useOpener } from '../../../composables/useOpener.ts';
import nezzelIcon from '../../../assets/brand/nezzel-icon.svg';
import hululsoftMark from '../../../assets/brand/hululsoft-mark.svg';

defineOptions({
  inheritAttrs: false,
});

const { t } = useI18n();
// Product names are proper nouns: Latin script in every language, like "Nezzel" and "Hululsoft".
const HULULSOFT_URL = 'https://hululsoft.mr';
const NEZZEL_URL = 'https://nezzel.hululsoft.mr';
const OTHER_PRODUCTS = ['Najahi', 'Souki', 'Bgalti', 'cvite', 'Sorety', 'Mahdarat'];
const linkRowCls = 'group flex min-h-[52px] items-center justify-between gap-8 border-b border-hairline py-3 text-[13px] hover:text-primary';
const links = computed(() => [
  { href: NEZZEL_URL, label: t('about.links.website') },
  { href: 'https://github.com/ahmed-abdat/nezzel-desktop', label: t('about.links.github') },
]);
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
