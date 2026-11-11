<template>
  <div>
    <!-- Flat, inside the tinted row: one line, the fine print, then the choice. No box. -->
    <div v-if="card.kind === 'needed'" class="text-amber-foreground" data-testid="login-needed">
      <p class="flex items-start gap-1.5 text-[12.5px] font-medium">
        <key-round class="mt-0.5 size-3.5 shrink-0" aria-hidden="true" />
        <span>{{ card.request.changeOnly ? t('downloads.login.changeProfile') : siteLabel ? t('downloads.login.needs', { site: siteLabel }) : t('downloads.login.needsGeneric') }}</span>
      </p>
      <p class="mt-0.5 ps-5 text-[12px] leading-relaxed text-pretty">{{ t('downloads.login.privacy') }}</p>
      <p v-if="isMac" class="ps-5 text-[12px] leading-relaxed text-pretty">{{ t('downloads.login.keychain') }}</p>
      <p v-if="card.request.reason === LOGIN_APP_BOUND_REASON" class="mt-1 ps-5 text-[12px]">{{ appBoundMessage }}</p>
      <div class="mt-2 flex flex-wrap items-center gap-2 ps-5">
        <template v-if="candidates.length > 1">
          <label :for="selectId" class="sr-only">{{ t('downloads.login.useFrom') }}</label>
          <select-field :id="selectId" v-model="browser" :options="browserOptions" :value-dir="localeDirection(locale)" class="min-w-40 border-amber-border">
            <template #value><LoginCandidateLabel :candidate="candidates.find(c => c.id === browser)" :host="host" /></template>
            <template #option="{ option }"><LoginCandidateLabel :candidate="candidates.find(c => c.id === option.value)" :host="host" /></template>
          </select-field>
        </template>
        <span v-else-if="candidates.length === 1" class="text-[12.5px]">
          {{ t('downloads.login.useFrom') }} <span class="font-medium"><LoginCandidateLabel :candidate="candidates[0]" :host="host" /></span>
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
        <!-- Retrying can't help until the login is fixed (refused, exhausted): settings first, retry second.
             Dismissed or expired: "Log in again" reopens the prompt, so it leads. -->
        <template v-if="card.failure.reason === 'dismissed' || card.failure.reason === 'expired'">
          <Button @click="emit('retry')"><rotate-cw />{{ t('downloads.login.again') }}</Button>
          <router-link :to="{ name: 'settings.logins' }" class="text-[12.5px] font-medium underline underline-offset-2 hover:no-underline">{{ t('downloads.login.settings') }}</router-link>
        </template>
        <template v-else>
          <Button as-child data-testid="login-settings"><router-link :to="{ name: 'settings.logins' }">{{ t('downloads.login.settings') }}</router-link></Button>
          <Button v-if="card.failure.reason === 'exhausted'" variant="outline" @click="emit('retry')"><rotate-cw />{{ t('downloads.login.tryAnother') }}</Button>
        </template>
      </div>
    </div>
    <Collapsible v-if="details" class="mt-1 ps-5">
      <CollapsibleTrigger class="min-h-7 cursor-pointer text-[12px] text-muted-foreground underline underline-offset-2 hover:no-underline">{{ t('downloads.row.details') }}</CollapsibleTrigger>
      <CollapsibleContent>
        <pre dir="ltr" class="mt-1 whitespace-pre-wrap break-words text-start text-[12px] text-muted-foreground" data-testid="login-details">{{ details }}</pre>
      </CollapsibleContent>
    </Collapsible>
  </div>
</template>

<script setup lang="ts">
import { computed, ref, useId, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { localeDirection } from '../../i18n';
import LoginCandidateLabel from './LoginCandidateLabel.vue';
import { KeyRound, RotateCw, TriangleAlert } from '@lucide/vue';
import { Button } from '@/components/ui/button';
import { SelectField } from '@/components/ui/select';
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from '@/components/ui/collapsible';
import { useLoginStore, type LoginCard } from '../../stores/login';
import { browserName, candidateLabel } from '../../helpers/cookieBrowsers';
import type { LoginCandidate } from '../../tauri/types/login';
import { siteName } from '../../helpers/siteName';
import { useSettingsStore } from '../../stores/settings';
import { usePlatform } from '../../composables/usePlatform';
import { COOKIE_BROWSER_AUTOMATIC, COOKIE_BROWSER_NEVER, LOGIN_APP_BOUND_REASON } from '@/generated/contract';

const props = defineProps<{ card: LoginCard; site?: string; details?: string }>();
const emit = defineEmits<{ retry: [] }>();

const { t, locale } = useI18n();
const loginStore = useLoginStore();
const settingsStore = useSettingsStore();
const selectId = useId();
const { isMac, isWindows } = usePlatform();
/** Windows Edge/Chrome logins are unreadable (app-bound), so only there point at Firefox. */
const noBrowserKey = computed(() => (isWindows.value ? 'downloads.login.noBrowserWindows' : 'downloads.login.noBrowser'));
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

/** Typeahead uses plain text; the rendered value and options use LoginCandidateLabel's <bdi> runs. */
const isolate = (text: string) => (localeDirection(locale.value) === 'rtl' ? `\u2068${text}\u2069` : text);
function optionLabel(candidate: LoginCandidate): string {
  const label = isolate(candidateLabel(candidate));
  return candidate.signedIn ? t('downloads.login.profileStatus', { profile: label, status: t('downloads.login.signedIn', { host: isolate(host.value) }) }) : label;
}

/** Browser the engine actually read the login from for this row, if any. */
const usedBrowser = computed(() => (props.card.kind === 'failed' ? loginStore.using[props.card.failure.groupId]?.candidate?.browser : undefined));

const appBoundMessage = computed(() => {
  const blocked = props.card.kind === 'needed' ? props.card.request.blockedBrowser : props.card.failure.blockedBrowser;
  return blocked
    ? t('downloads.login.windowsAppBoundBrowser', { browser: isolate(browserName(blocked)) })
    : t('downloads.login.windowsAppBound');
});

const failedMessage = computed(() => {
  if (props.card.kind !== 'failed') return '';
  const site = siteLabel.value ?? t('downloads.login.thisSite');
  const { reason, browser: triedBrowser } = props.card.failure;
  if (props.card.failure.hint === LOGIN_APP_BOUND_REASON) return appBoundMessage.value;
  if (reason === 'exhausted') {
    return triedBrowser
      ? t('downloads.login.refusedBrowser', { site, browser: browserName(triedBrowser) })
      : t(noBrowserKey.value);
  }
  if (reason === 'dismissed') return t('downloads.login.skipped', { site });
  if (reason === 'expired') return t('downloads.login.timedOut', { site });
  // Refused: name the browser that was really used, so the line never contradicts "Using Chrome · …" below it.
  if (usedBrowser.value) return t('downloads.login.stillFailing', { browser: browserName(usedBrowser.value) });
  const fixed = settingsStore.settings.auth.cookieBrowser;
  if (fixed && fixed !== COOKIE_BROWSER_NEVER && fixed !== COOKIE_BROWSER_AUTOMATIC) return t('downloads.login.stillFailing', { browser: browserName(fixed) });
  return fixed === COOKIE_BROWSER_AUTOMATIC ? t(noBrowserKey.value) : t('downloads.login.turnedOff', { site });
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
