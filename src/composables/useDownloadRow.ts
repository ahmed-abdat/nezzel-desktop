import { computed, onScopeDispose, ref, watch, type Ref } from 'vue';
import { useI18n } from 'vue-i18n';
import { MediaState, useMediaStateStore } from '../stores/media/state';
import { useMediaProgressStore } from '../stores/media/progress';
import { useMediaOptionsStore } from '../stores/media/options';
import { useMediaDiagnosticsStore } from '../stores/media/diagnostics';
import { useMediaDestinationStore } from '../stores/media/destination';
import { ProgressStage } from '../tauri/types/progress';
import { formatDuration } from '../helpers/units';
import { qualityLabel } from '../helpers/formats';
import { capitalizeFirstLetter } from '../helpers/progress';
import { choiceForOptions } from '../helpers/quickChoice';
import { groupFileKind, groupSite, phaseOf, savedAttachments, type Phase } from '../helpers/groupView';
import { useLocaleNumber } from './useLocaleNumber';
import type { Group } from '../tauri/types/group';
import { LOGIN_CODES } from '@/generated/contract';

/** After this long a row says it is still reading; after the second, it offers Retry. */
export const READING_SLOW_SECS = 8;
export const READING_STUCK_SECS = 60;

/** One 1 s ticker shared by every reading row; it runs only while at least one row holds it. */
const clockNow = ref(Date.now());
let clockHolders = 0;
let clockTimer: ReturnType<typeof setInterval> | undefined;
function holdClock() {
  clockHolders += 1;
  clockNow.value = Date.now();
  clockTimer ??= setInterval(() => {
    clockNow.value = Date.now();
  }, 1000);
}
function releaseClock() {
  clockHolders = Math.max(0, clockHolders - 1);
  if (clockHolders > 0 || !clockTimer) return;
  clearInterval(clockTimer);
  clockTimer = undefined;
}

/** Display model for one download row: phase, progress, meta line and the login hint. */
export function useDownloadRow(group: Ref<Group>) {
  const i18n = useI18n();
  const { t } = i18n;
  // These strings also appear inside translated metadata parts rendered by a single <bdi>.
  const isolate = (text: string) => i18n.locale.value.startsWith('ar') ? `\u2066${text}\u2069` : text;
  const { percent: formatPct, size: formatSizeLocal, speed: formatSpeedLocal, count: formatCount } = useLocaleNumber();
  const stateStore = useMediaStateStore();
  const progressStore = useMediaProgressStore();
  const optionsStore = useMediaOptionsStore();
  const diagnosticsStore = useMediaDiagnosticsStore();
  const destinationStore = useMediaDestinationStore();

  const state = computed(() => stateStore.getGroupState(group.value.id) ?? MediaState.fetching);
  const phase = computed<Phase>(() => phaseOf(state.value));
  const isList = computed(() => group.value.isCombined || state.value === MediaState.downloadingList || state.value === MediaState.pausedList);

  const site = computed(() => groupSite(group.value));

  const single = computed(() => progressStore.findDownloadProgress(group.value.id));
  const list = computed(() => (isList.value ? progressStore.findGroupProgress(group.value.id) : undefined));

  const queued = computed(() => {
    const pending = Object.values(group.value.items).filter((item) => {
      const state = stateStore.getState(item.id);
      return !item.entries && state !== MediaState.done && state !== MediaState.error;
    });
    return phase.value === 'downloading' && pending.length > 0
      && (!isList.value || (list.value?.done ?? 0) === 0)
      && pending.every(item => progressStore.progress[item.id]?.stage === ProgressStage.queued);
  });

  /** Keep the last percentage visible while the resumed job waits for its first update. */
  const resuming = ref(false);
  watch(phase, (now, before) => {
    resuming.value = before === 'paused' && now === 'downloading';
  });
  watch(() => [single.value?.stage, single.value?.percentage], ([stage, percentage]) => {
    if (stage === ProgressStage.downloading && percentage != null) resuming.value = false;
  });

  /** 0–100, or null when the engine has no number yet (preparing, merging…). */
  const percent = computed<number | null>(() => {
    if (phase.value === 'done') return 100;
    if (isList.value) {
      const p = list.value;
      if (!p || !p.total || queued.value) return null;
      return ((p.done ?? 0) / p.total) * 100;
    }
    const p = single.value;
    if (queued.value || resuming.value) return p?.percentage ?? null;
    if (!p || p.stage !== ProgressStage.downloading || p.percentage == null) return phase.value === 'paused' ? (p?.percentage ?? null) : null;
    return p.percentage;
  });

  const statusLocalization = computed<Record<string, string>>(() => i18n.tm('media.steps.download.status'));
  const stageLabel = computed(() => {
    let stage = single.value?.stage ?? ProgressStage.initializing;
    if (stage === ProgressStage.downloading && single.value?.percentage == null) stage = ProgressStage.initializing;
    return statusLocalization.value[stage] ?? capitalizeFirstLetter(stage);
  });

  const speed = computed(() => {
    const bps = isList.value ? list.value?.speedBps : single.value?.speedBps;
    return bps ? formatSpeedLocal(bps) : undefined;
  });
  /** After the bytes arrive (merging, converting, cleaning up): pausing would do nothing, only cancel can. */
  const processing = computed(() => phase.value === 'downloading' && !isList.value
    && [ProgressStage.merging, ProgressStage.remuxing, ProgressStage.reencoding, ProgressStage.finalizing].includes(single.value?.stage as ProgressStage));
  const eta = computed(() => (single.value?.etaSecs != null && !isList.value ? formatDuration(single.value.etaSecs) : undefined));

  const choice = computed(() => choiceForOptions(optionsStore.getOptions(group.value.id), group.value.formats ?? [], group.value.mediaKind));
  /** Photo posts have no formats; their label is what gets saved, never a video/audio choice. */
  const photoLabel = computed(() => {
    const saved = savedAttachments(group.value.mediaKind, group.value.attachments ?? [], optionsStore.getAttachmentSelection(group.value.id));
    return saved && t(`downloads.row.${saved.key}`, { n: saved.n }, saved.n);
  });
  const destination = computed(() => destinationStore.getPrimaryDestination(group.value.id)?.path);
  /** Category of a file download: from the saved file when it exists, else the link's title. */
  const fileKind = computed(() => groupFileKind(group.value, destination.value));
  const choiceLabel = computed(() => {
    if (fileKind.value) {
      const category = t(`files.category.${fileKind.value}`);
      return group.value.filesize ? t('files.withSize', { category, size: isolate(formatSizeLocal(group.value.filesize)) }) : category;
    }
    if (photoLabel.value) return photoLabel.value;
    const opts = optionsStore.getOptions(group.value.id);
    switch (choice.value) {
      case 'audio': return t('sheet.audio');
      case 'small': return opts?.height ? qualityLabel(opts.height, opts.fps) : t('sheet.small');
      case 'best': return opts?.height ? qualityLabel(opts.height, opts.fps) : t('sheet.best');
      default: return opts?.height ? qualityLabel(opts.height, opts.fps) : opts?.abr ? `${opts.abr}kbps` : t('sheet.custom');
    }
  });

  /** Seconds this row has been reading (slow sites get a hint, then Retry). One shared clock for all rows. */
  const readingSince = ref<number | null>(null);
  // A retry swaps the items while the row keeps reading: that restarts the clock too.
  watch(() => [phase.value, group.value.items] as const, ([now]) => {
    if (readingSince.value != null) releaseClock();
    readingSince.value = now === 'reading' ? Date.now() : null;
    if (readingSince.value != null) holdClock();
  }, { immediate: true });
  onScopeDispose(() => {
    if (readingSince.value != null) releaseClock();
  });
  const readingSecs = computed(() => (readingSince.value == null ? 0 : Math.max(0, Math.floor((clockNow.value - readingSince.value) / 1000))));
  const readingSlow = computed(() => readingSecs.value >= READING_SLOW_SECS);
  const readingStuck = computed(() => readingSecs.value >= READING_STUCK_SECS);

  const metaLine = computed<string[]>(() => {
    const parts: string[] = [];
    if (site.value) parts.push(site.value);
    switch (phase.value) {
      case 'reading':
        if (state.value === MediaState.fetchingList && group.value.total > 1) {
          parts.push(t('downloads.row.readingList', { done: group.value.processed, total: group.value.total }));
        } else if (readingStuck.value) {
          parts.push(t('downloads.row.readingStuck'));
        } else {
          parts.push(readingSlow.value ? t('downloads.row.stillReading', { s: formatCount(readingSecs.value) }) : t('downloads.row.reading'));
        }
        break;
      case 'ready':
        if (group.value.isCombined) parts.push(t('downloads.row.items', { n: group.value.total }, group.value.total));
        else parts.push(choiceLabel.value);
        break;
      case 'downloading':
        if (queued.value) parts.push(statusLocalization.value[ProgressStage.queued]);
        else if (resuming.value && !isList.value) parts.push(t('downloads.row.resuming'));
        else if (percent.value == null) parts.push(stageLabel.value);
        else {
          parts.push(isList.value ? `${list.value?.done ?? 0}/${list.value?.total ?? group.value.total}` : formatPct(percent.value));
          if (speed.value) parts.push(speed.value);
          if (eta.value) parts.push(t('downloads.row.left', { eta: isolate(eta.value) }));
        }
        break;
      case 'paused':
        parts.push(t('downloads.row.paused', { percent: formatPct(percent.value ?? 0) }));
        break;
      case 'done':
        parts.push(group.value.isCombined ? t('downloads.row.items', { n: group.value.total }, group.value.total) : choiceLabel.value);
        break;
      default:
        break;
    }
    return parts.filter(part => part && part.trim().length > 0);
  });

  /** Wide list columns: what was chosen, how big, how long. Empty string = unknown yet. */
  const columns = computed(() => ({
    quality: group.value.isCombined ? t('downloads.row.items', { n: group.value.total }, group.value.total) : (phase.value === 'reading' ? '' : choiceLabel.value),
    size: group.value.filesize ? formatSizeLocal(group.value.filesize) : '',
    length: group.value.duration ? formatDuration(group.value.duration) : '',
  }));

  const lastDiagnostic = computed(() => diagnosticsStore.findLastDiagnosticByGroupId(group.value.id));
  const lastFatal = computed(() => diagnosticsStore.findLastFatalByGroupId(group.value.id));
  const needsLogin = computed(() => phase.value === 'error' && (LOGIN_CODES as readonly string[]).includes(lastDiagnostic.value?.code ?? ''));

  return { state, phase, isList, site, percent, metaLine, columns, needsLogin, lastDiagnostic, lastFatal, destination, choice, fileKind, readingStuck, processing };
}
