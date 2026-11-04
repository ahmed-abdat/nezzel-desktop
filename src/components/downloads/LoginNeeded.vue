<template>
  <!-- Flat, inside the tinted row: one line, the fine print, then the choice. No box. -->
  <div v-if="card.kind === 'needed'" class="text-amber-foreground" data-testid="login-needed">
    <p class="flex items-start gap-1.5 text-[12.5px] font-medium">
      <key-round class="mt-0.5 size-3.5 shrink-0" aria-hidden="true" />
      <span>{{ card.request.changeOnly ? t('downloads.login.changeProfile') : siteLabel ? t('downloads.login.needs', { site: siteLabel }) : t('downloads.login.needsGeneric') }}</span>
    </p>
    <p class="mt-0.5 ps-5 text-[12px] leading-relaxed text-pretty">{{ t('downloads.login.privacy') }}</p>
    <p v-if="isMac" class="ps-5 text-[12px] leading-relaxed text-pretty">{{ t('downloads.login.keychain') }}</p>
    <p v-if="card.request.reason === 'browserAppBound'" class="mt-1 ps-5 text-[12px]">{{ t('downloads.login.windowsAppBound') }}</p>
    <div class="mt-2 flex flex-wrap items-center gap-2 ps-5">
      <template v-if="candidates.length > 1">
        <label :for="selectId" class="sr-only">{{ t('downloads.login.useFrom') }}</label>
        <select-field :id="selectId" v-model="browser" :options="browserOptions" class="min-w-40 border-amber-border" />
      </template>
      <span v-else-if="candidates.length === 1" class="text-[12.5px]">
        {{ t('downloads.login.useFrom') }} <bdi class="font-medium">{{ optionLabel(candidates[0]) }}</bdi>
      </span>
      <Button variant="amber" :disabled="busy || (!browser && candidates.length !== 1)" @click="use">{{ t('downloads.login.use') }}</Button>
      <Button variant="ghost" class="text-amber-foreground hover:bg-amber-border/50" :disabled="busy" @click="notNow">{{ t('downloads.login.notNow') }}</Button>
    </div>
  </div>

  <div v-else class="text-destructive-foreground" data-testid="login-failed">
    <p class="flex items-start gap-1.5 text-[12.5px] font-medium">
      <triangle-alert class="mt-0.5 size-3.5 shrink-0" aria-hidden="true" />
      <span class="text-pretty">{{ failedMessage }}</span>
    </p>
    <div v-if="card.failure.reason !== 'refused' || !usedBrowser" class="mt-1.5 flex flex-wrap items-center gap-x-3 gap-y-1 ps-5">
      <Button v-if="card.failure.reason === 'exhausted'" variant="outline" @click="emit('retry')"><rotate-cw />{{ t('downloads.login.tryAnother') }}</Button>
      <Button v-else-if="card.failure.reason === 'dismissed' || card.failure.reason === 'expired'" variant="outline" @click="emit('retry')"><rotate-cw />{{ t('downloads.login.again') }}</Button>
      <router-link :to="{ name: 'settings.logins' }" class="text-[12.5px] font-medium underline underline-offset-2 hover:no-underline">{{ t('downloads.login.settings') }}</router-link>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed, ref, useId, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { localeDirection } from '../../i18n';
import { KeyRound, RotateCw, TriangleAlert } from '@lucide/vue';
import { Button } from '@/components/ui/button';
import { SelectField } from '@/components/ui/select';
import { useLoginStore, type LoginCard } from '../../stores/login';
import { browserName, candidateLabel } from '../../helpers/cookieBrowsers';
import type { LoginCandidate } from '../../tauri/types/login';
import { siteName } from '../../helpers/siteName';
import { useSettingsStore } from '../../stores/settings';

const props = defineProps<{ card: LoginCard; site?: string }>();
const emit = defineEmits<{ retry: [] }>();

const { t, locale } = useI18n();
const loginStore = useLoginStore();
const settingsStore = useSettingsStore();
const selectId = useId();
const isMac = typeof navigator !== 'undefined' && /Mac/i.test(navigator.platform || navigator.userAgent);
const busy = ref(false);

const host = computed(() => (props.card.kind === 'needed' ? props.card.request.host : props.card.failure.host));
/** Consent names the host the engine will remember the browser for; the row's label is only a fallback. */
const siteLabel = computed(() => (host.value ? siteName(undefined, `https://${host.value}`) : props.site));
const candidates = computed(() => (props.card.kind === 'needed' ? props.card.request.candidates : []));

const browser = ref('');
const browserOptions = computed(() => candidates.value.map(c => ({ value: c.id, label: optionLabel(c) })));
watch(() => (props.card.kind === 'needed' ? `${props.card.request.requestId}|${props.card.preselect}` : ''), () => {
  if (props.card.kind === 'needed') browser.value = props.card.preselect;
}, { immediate: true });

/** Plain-text option labels can't hold <bdi>: in RTL, Unicode isolates keep "Chrome (Person 1)" and the host in order. */
const isolate = (text: string) => (localeDirection(locale.value) === 'rtl' ? `\u2068${text}\u2069` : text);
function optionLabel(candidate: LoginCandidate): string {
  const label = isolate(candidateLabel(candidate));
  return candidate.signedIn ? `${label} · ${t('downloads.login.signedIn', { host: isolate(host.value) })}` : label;
}

/** Browser the engine actually read the login from for this row, if any. */
const usedBrowser = computed(() => (props.card.kind === 'failed' ? loginStore.using[props.card.failure.groupId]?.candidate?.browser : undefined));

const failedMessage = computed(() => {
  if (props.card.kind !== 'failed') return '';
  const site = siteLabel.value ?? t('downloads.login.thisSite');
  const { reason, browser: triedBrowser } = props.card.failure;
  if (props.card.failure.hint === 'browserAppBound') return t('downloads.login.windowsAppBound');
  if (reason === 'exhausted') {
    return triedBrowser
      ? t('downloads.login.refusedBrowser', { site, browser: browserName(triedBrowser) })
      : t('downloads.login.noBrowser', { site });
  }
  if (reason === 'dismissed') return t('downloads.login.skipped', { site });
  if (reason === 'expired') return t('downloads.login.timedOut', { site });
  // Refused: name the browser that was really used, so the line never contradicts "Using Chrome · …" below it.
  if (usedBrowser.value) return t('downloads.login.stillFailing', { browser: browserName(usedBrowser.value) });
  const fixed = settingsStore.settings.auth.cookieBrowser;
  if (fixed && !['none', 'automatic'].includes(fixed)) return t('downloads.login.stillFailing', { browser: browserName(fixed) });
  return fixed === 'automatic' ? t('downloads.login.noBrowser', { site }) : t('downloads.login.turnedOff', { site });
});

async function use() {
  if (props.card.kind !== 'needed') return;
  const selected = browser.value || (candidates.value.length === 1 ? candidates.value[0].id : '');
  if (!selected) return;
  busy.value = true;
  try {
    await loginStore.accept(props.card.request.requestId, selected);
  } finally {
    busy.value = false;
  }
}

async function notNow() {
  if (props.card.kind !== 'needed') return;
  busy.value = true;
  try {
    await loginStore.dismiss(props.card.request.requestId);
  } finally {
    busy.value = false;
  }
}
</script>
