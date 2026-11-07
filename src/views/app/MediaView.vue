<template>
  <article v-if="group" class="pb-10">
    <!-- Header: Back + one Download button, sized like the toolbar controls. -->
    <header class="sticky top-0 z-10 flex h-12 items-center gap-2 border-b border-hairline bg-background/90 px-4 backdrop-blur">
      <Button variant="ghost" size="sm" class="-ms-2" @click="goBack">
        <ChevronLeft class="rtl:rotate-180" aria-hidden="true" />{{ t('common.back') }}
      </Button>
      <div class="flex-1" />
      <Button v-if="canDownload" size="sm" @click="downloadItem">
        <ArrowDownToLine aria-hidden="true" />{{ isPaused ? t('common.resume') : t('common.download') }}
      </Button>
    </header>

    <div class="mx-auto max-w-[760px] px-6 pt-5">
      <!-- Failed: one plain-language card with one action; the technical text lives in the disclosure below. -->
      <section v-if="phase === 'error'" class="mb-5 flex items-start gap-3 rounded-lg border border-destructive-border bg-destructive-soft px-4 py-3" role="alert">
        <CircleAlert class="mt-0.5 size-4 shrink-0 text-destructive" aria-hidden="true" />
        <div class="min-w-0 flex-1">
          <p class="text-[13px] font-semibold text-destructive-foreground">{{ t('media.detail.failed') }}</p>
          <i18n-t v-if="errorDisplay.action" keypath="errors.rowLine" tag="p" scope="global" class="mt-0.5 text-[12.5px] text-destructive-foreground/90 text-pretty">
            <template #what>{{ errorDisplay.shortMessage }}</template>
            <template #action><span class="text-foreground/85">{{ errorDisplay.action }}</span></template>
          </i18n-t>
          <p v-else class="mt-0.5 text-[12.5px] text-destructive-foreground/90 text-pretty">{{ errorDisplay.shortMessage }}</p>
        </div>
        <Button v-if="needsLogin" variant="outline" size="sm" @click="router.push({ name: 'settings.logins' })">{{ t('media.detail.useLogin') }}</Button>
        <Button v-else-if="group.url" variant="outline" size="sm" @click="recover">
          <RotateCw aria-hidden="true" />{{ recoveryLabel }}
        </Button>
      </section>

      <!-- Media: thumbnail, title, one muted line, small stats, link. -->
      <section class="flex flex-col gap-5 sm:flex-row">
        <div v-if="fileKind" class="flex aspect-video w-full shrink-0 items-center justify-center rounded-lg bg-muted sm:w-[320px]">
          <FileTypeIcon :category="fileKind" class="size-16 text-muted-foreground" />
        </div>
        <img v-else
          :src="thumbnail"
          :alt="t('media.view.thumbnailAlt', { title: group.title ?? group.url ?? t('common.video') })"
          class="aspect-video w-full shrink-0 rounded-lg bg-muted object-cover sm:w-[320px]"
          @error="thumbFailed = true"
        />
        <div class="min-w-0 flex-1">
          <h1 class="text-start text-[17px] font-semibold leading-snug text-pretty [overflow-wrap:anywhere]"><bdi dir="auto">{{ group.title ?? group.url }}</bdi></h1>
          <p class="mt-1.5 flex flex-wrap items-center gap-x-1.5 gap-y-1 text-[12.5px] text-muted-foreground">
            <template v-for="(part, index) in metaParts" :key="part.key">
              <span v-if="index > 0" class="nz-sep" aria-hidden="true" />
              <span class="inline-flex items-center gap-1.5">
                <img v-if="part.key === 'site' && favicon && !faviconFailed" :src="favicon" alt="" class="size-3.5 rounded-sm" @error="faviconFailed = true" />
                <bdi :class="part.key === 'duration' && 'tnum'">{{ part.text }}</bdi>
              </span>
            </template>
          </p>
          <ul v-if="stats.length" class="mt-2.5 flex flex-wrap items-center gap-x-4 gap-y-1 text-[12px] text-muted-foreground" :aria-label="t('media.detail.stats')">
            <li v-for="stat in stats" :key="stat.key" class="inline-flex items-center gap-1.5" :title="stat.label">
              <component :is="stat.icon" class="size-3.5" aria-hidden="true" />
              <span class="tnum">{{ stat.value }}</span>
              <span class="sr-only">{{ stat.label }}</span>
            </li>
          </ul>
          <div v-if="group.url" class="mt-3 flex min-w-0 items-center gap-1">
            <Link2 class="size-3.5 shrink-0 text-muted-foreground rtl:-scale-x-100" aria-hidden="true" />
            <a :href="group.url" target="_blank" rel="noopener" class="min-w-0 truncate text-[12.5px] text-primary hover:underline" dir="ltr" :title="group.url">{{ displayUrl }}</a>
            <tooltip :content="t('media.detail.copyLink')">
              <Button variant="ghost" size="icon-xs" class="text-muted-foreground" :aria-label="t('media.detail.copyLink')" @click="copy(group.url, 'link')">
                <Check v-if="copied === 'link'" class="text-primary" aria-hidden="true" />
                <Copy v-else aria-hidden="true" />
              </Button>
            </tooltip>
          </div>
        </div>
      </section>

      <section v-if="group.description" class="mt-8">
        <h2 class="mb-1.5 text-[12px] font-semibold text-foreground/65">{{ t('media.view.metadata.description') }}</h2>
        <p class="max-h-48 overflow-y-auto whitespace-pre-line text-[13px] leading-relaxed text-foreground/85 [overflow-wrap:anywhere]" dir="auto">{{ group.description }}</p>
      </section>

      <section v-if="chapters.length" class="mt-8">
        <h2 class="mb-1 text-[12px] font-semibold text-foreground/65">{{ t('media.view.metadata.chapters') }} <span class="tnum font-normal text-muted-foreground">{{ `(${n(chapters.length)})` }}</span></h2>
        <ol class="max-h-64 overflow-y-auto">
          <li v-for="chapter in chapters" :key="chapter.key" class="flex items-baseline gap-3 border-b border-hairline py-1.5 text-[13px] last:border-b-0">
            <bdi class="tnum w-16 shrink-0 text-[12px] text-muted-foreground" dir="ltr">{{ chapter.start }}</bdi>
            <span class="min-w-0 flex-1 truncate" dir="auto">{{ chapter.title || t('common.unknown') }}</span>
          </li>
        </ol>
      </section>

      <!-- Developer output, collapsed. Empty lists never show. -->
      <Collapsible class="group mt-10 border-t border-hairline pt-3" :unmount-on-hide="false">
        <CollapsibleTrigger class="inline-flex h-7 cursor-pointer items-center gap-1 rounded-[6px] text-[12px] font-semibold text-foreground/65 outline-none focus-visible:outline-solid focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--nz-focus)]">
          <ChevronRight class="size-3.5 transition-transform duration-150 group-data-[state=open]:rotate-90 rtl:rotate-180 rtl:group-data-[state=open]:rotate-90" aria-hidden="true" />
          {{ t('media.detail.technical') }}
        </CollapsibleTrigger>
        <CollapsibleContent>
          <div class="mt-2 flex flex-col gap-3">
            <ul v-if="problems.length" class="flex flex-col gap-1.5">
              <li v-for="problem in problems" :key="problem.key" class="flex items-start gap-2 text-[12.5px]">
                <span class="mt-1.5 size-1.5 shrink-0 rounded-full" :class="problem.level === 'error' ? 'bg-destructive' : 'bg-amber-foreground/60'" aria-hidden="true" />
                <span class="min-w-0 flex-1 [overflow-wrap:anywhere]">{{ problem.text }}</span>
              </li>
            </ul>
            <pre v-if="logText" role="log" aria-live="off" dir="ltr" class="max-h-64 overflow-auto whitespace-pre-wrap rounded-md bg-muted px-3 py-2 font-mono text-[11.5px] leading-5 text-foreground/80">{{ logText }}</pre>
            <p v-else class="text-[12.5px] text-muted-foreground">{{ t('media.detail.noLog') }}</p>
            <div>
              <Button variant="outline" size="sm" @click="copy(details, 'details')">
                <Check v-if="copied === 'details'" class="text-primary" aria-hidden="true" />
                <Copy v-else aria-hidden="true" />
                {{ copied === 'details' ? t('media.detail.copied') : t('media.detail.copyDetails') }}
              </Button>
            </div>
          </div>
        </CollapsibleContent>
      </Collapsible>
    </div>
  </article>
</template>

<script setup lang="ts">
import { useSettingsStore } from '../../stores/settings';
import { remoteImagesAllowed } from '../../helpers/remoteImages';
import Tooltip from '../../components/ui/tooltip/Tooltip.vue';
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from '../../components/ui/collapsible';
import { computed, onBeforeUnmount, ref, toRef } from 'vue';
import { useI18n } from 'vue-i18n';
import { useRouter } from 'vue-router';
import { copyText } from '../../tauri/clipboard';
import { ArrowDownToLine, Check, ChevronLeft, ChevronRight, CircleAlert, Copy, Eye, Link2, MessageCircle, RotateCw, ThumbsUp } from '@lucide/vue';
import { Button } from '@/components/ui/button';
import placeholderUrl from '../../assets/placeholder.png';
import { useMediaGroupStore } from '../../stores/media/group';
import { useMediaDiagnosticsStore } from '../../stores/media/diagnostics';
import FileTypeIcon from '../../components/downloads/FileTypeIcon.vue';
import { useDownloadRow } from '../../composables/useDownloadRow';
import { useDownloadActions } from '../../composables/useDownloadActions';
import { useDiagnostic } from '../../composables/useDiagnostic';
import { useGroupLog } from '../../composables/useGroupLog';
import { useLocaleNumber } from '../../composables/useLocaleNumber';
import { formatDuration } from '../../helpers/units';
import { groupSkippedDiagnostics, isSkippedDiagnostic } from '../../helpers/skippedDiagnostics';
import type { Group } from '../../tauri/types/group';

const props = defineProps<{ groupId: string }>();
const { t } = useI18n();
const settingsStore = useSettingsStore();
const router = useRouter();
const { count: n, size: formatSize } = useLocaleNumber();
const groupStore = useMediaGroupStore();
const diagnosticsStore = useMediaDiagnosticsStore();

const group = computed(() => groupStore.findGroupById(props.groupId));
// The layout redirects home when the group is gone; until then the composables need a group to read.
const safeGroup = computed(() => group.value ?? ({ id: props.groupId, items: {} } as unknown as Group));
const { phase, site, fileKind, needsLogin, lastDiagnostic, lastFatal } = useDownloadRow(safeGroup);
const { download, recover, recoveryLabel } = useDownloadActions(safeGroup);
const groupLog = useGroupLog(toRef(props, 'groupId'));

const isPaused = computed(() => phase.value === 'paused');
const canDownload = computed(() => phase.value === 'ready' || phase.value === 'paused');

function goBack() {
  const state = window.history.state as { back?: string | null } | null;
  if (state?.back) router.back();
  else void router.push({ name: 'home' });
}
function downloadItem() {
  void download();
  void router.push({ name: 'home' });
}

const thumbFailed = ref(false);
const thumbnail = computed(() => (!thumbFailed.value && remoteImagesAllowed(settingsStore.settings.network, group.value?.thumbnail) && group.value?.thumbnail) || placeholderUrl);

/** The site's own favicon (same host the video comes from; nothing is sent to a third party). */
const faviconFailed = ref(false);
const favicon = computed(() => {
  if (!remoteImagesAllowed(settingsStore.settings.network)) return null;
  try {
    const url = new URL(group.value?.url ?? '');
    return url.protocol === 'https:' ? `${url.origin}/favicon.ico` : null;
  } catch {
    return null;
  }
});
const displayUrl = computed(() => (group.value?.url ?? '').replace(/^https?:\/\/(www\.)?/, ''));

const metaParts = computed(() => {
  const parts: { key: string; text: string }[] = [];
  if (group.value?.uploader) parts.push({ key: 'uploader', text: group.value.uploader });
  if (site.value) parts.push({ key: 'site', text: site.value });
  if (fileKind.value) {
    parts.push({ key: 'type', text: t(`files.category.${fileKind.value}`) });
    if (group.value?.filesize) parts.push({ key: 'size', text: formatSize(group.value.filesize) });
  }
  if (group.value?.duration) parts.push({ key: 'duration', text: formatDuration(group.value.duration, { pad: true }) });
  return parts;
});

const stats = computed(() => {
  const g = group.value;
  if (!g) return [];
  return [
    g.views ? { key: 'views', icon: Eye, value: n(g.views), label: t('media.view.stats.views') } : null,
    g.likes ? { key: 'likes', icon: ThumbsUp, value: n(g.likes), label: t('media.view.stats.likes') } : null,
    g.comments ? { key: 'comments', icon: MessageCircle, value: n(g.comments), label: t('media.view.stats.comments') } : null,
  ].filter((stat): stat is NonNullable<typeof stat> => stat !== null);
});

const chapters = computed(() => (group.value?.chapters ?? []).map((chapter, index) => ({
  key: `${chapter.startTime}-${index}`,
  start: formatDuration(chapter.startTime, { pad: true }),
  title: chapter.title,
})));

const diagnostic = computed<MediaDiagnostic>(() => {
  const base: MediaDiagnostic = { id: 'unknown', code: 'unknown', groupId: props.groupId, raw: '', message: 'unknown', level: 'error', component: null, timestamp: 0 };
  const fatal = lastFatal.value;
  if (fatal?.internal) return { ...base, id: fatal.id, message: fatal.message, component: 'runner', raw: fatal.details ?? '', timestamp: fatal.timestamp };
  return lastDiagnostic.value ?? base;
});
const { diagnosticDisplay: errorDisplay } = useDiagnostic(diagnostic, true);

const problems = computed(() => {
  const diagnostics = diagnosticsStore.findDiagnosticsByGroupId(props.groupId);
  const fatals = diagnosticsStore.findFatalsByGroupId(props.groupId).map(fatal => ({ key: `f-${fatal.id}-${fatal.timestamp}`, level: 'error', text: fatal.details || fatal.message }));
  const listed = diagnostics
    .filter(item => item.level === 'error' || (item.level === 'warning' && !isSkippedDiagnostic(item)))
    .map(item => ({ key: `d-${item.id}-${item.code}-${item.timestamp}`, level: item.level, text: item.message }));
  const skipped = groupSkippedDiagnostics(diagnostics).map(item => ({ key: `s-${item.message}`, level: 'warning', text: `${item.message} (${n(item.count)})` }));
  return [...fatals, ...listed, ...skipped];
});
const logText = computed(() => groupLog.logText.value.trim());

/** Plain text for a bug report: what failed, the problems list, then the raw engine log. */
const details = computed(() => [
  `Nezzel ${typeof __APP_VERSION__ === 'undefined' ? '' : __APP_VERSION__}`.trim(),
  site.value ? `Site: ${site.value}` : '',
  ...problems.value.map(problem => `[${problem.level}] ${problem.text}`),
  logText.value,
].filter(Boolean).join('\n'));

const copied = ref<'link' | 'details' | null>(null);
let copiedTimer: ReturnType<typeof setTimeout> | undefined;
async function copy(text: string, what: 'link' | 'details') {
  try {
    await copyText(text);
  } catch {
    // Same as before: a failed copy still shows the "copied" tick.
  }
  copied.value = what;
  clearTimeout(copiedTimer);
  copiedTimer = setTimeout(() => {
    copied.value = null;
  }, 1500);
}
onBeforeUnmount(() => clearTimeout(copiedTimer));
</script>
