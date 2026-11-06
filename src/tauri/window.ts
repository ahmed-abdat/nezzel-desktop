import { computed, ref, watch } from 'vue';
import { getCurrentWindow, ProgressBarStatus, UserAttentionType } from '@tauri-apps/api/window';
import { useMediaProgressStore } from '../stores/media/progress';
import { MediaState, useMediaStateStore } from '../stores/media/state';
import { notify } from './notifications';
import { NotificationKind, Platform } from './types/app';
import { currentPlatform } from '../composables/usePlatform';

export function startWindowWatcher(): void {
  const windowHandle = getCurrentWindow();
  const progressStore = useMediaProgressStore();
  const stateStore = useMediaStateStore();

  const previousActiveDownloadCount = ref(0);
  const attentionCooldownUntilMs = ref(0);
  let lastBadge: number | undefined;
  let lastBarKey = '';
  let sentOnce = false;

  function clampPercent0to100(value: number): number {
    return Math.max(0, Math.min(100, value));
  }

  function computeAggregatePercent(done: number, total: number): number | undefined {
    if (total <= 0) return undefined;
    return Math.round(clampPercent0to100((done / total) * 100));
  }

  function findActiveItemPercentIfSingleDownloading(): number | undefined {
    const entries = Object.entries(progressStore.progress);

    for (const [mediaId, itemProgress] of entries) {
      const mediaState = stateStore.getState(mediaId);

      if (mediaState === MediaState.downloading || mediaState === MediaState.downloadingList) {
        const percent = itemProgress.percentage;

        if (percent !== undefined && Number.isFinite(percent)) {
          return Math.round(clampPercent0to100(percent));
        }

        return undefined;
      }
    }

    return undefined;
  }

  type BarSignal
    = | { status: 'none' }
      | { status: 'indeterminate' }
      | { status: 'normal'; percent: number };

  function computeBar(activeDownloadCount: number, doneCount: number, totalCount: number): BarSignal {
    if (activeDownloadCount <= 0) return { status: 'none' };
    if (activeDownloadCount === 1) {
      const single = findActiveItemPercentIfSingleDownloading();
      if (typeof single === 'number') return { status: 'normal', percent: single };
    }
    const aggregate = computeAggregatePercent(doneCount, totalCount);
    if (typeof aggregate === 'number') return { status: 'normal', percent: aggregate };
    return { status: 'indeterminate' };
  }

  // Everything the OS shows, as one primitive: the watcher fires only when it changes.
  const signalKey = computed(() => {
    const counts = progressStore.findAllCounts();
    const bar = computeBar(counts.downloading, counts.done, counts.total);
    const percent = bar.status === 'normal' ? bar.percent : '';
    return `${counts.downloading}|${counts.ready}|${bar.status}|${percent}`;
  });

  async function setDockOrTaskbarBadge(count: number): Promise<void> {
    try {
      await windowHandle.setBadgeCount(count > 0 ? count : undefined);
    } catch {
      // ignore (some platforms may not support this)
    }
  }

  async function setProgressBarState(bar: BarSignal): Promise<void> {
    try {
      if (bar.status === 'none') {
        await windowHandle.setProgressBar({ status: ProgressBarStatus.None });
      } else if (bar.status === 'normal') {
        await windowHandle.setProgressBar({ status: ProgressBarStatus.Normal, progress: bar.percent });
      } else {
        await windowHandle.setProgressBar({ status: ProgressBarStatus.Indeterminate });
      }
    } catch {
      // ignore (some platforms may not support this)
    }
  }

  async function maybeRequestUserAttention(params: {
    activeDownloadCount: number;
    readyCount: number;
  }): Promise<void> {
    const { activeDownloadCount, readyCount } = params;

    const wasPreviouslyDownloading = previousActiveDownloadCount.value > 0;
    const queueIsFinished = activeDownloadCount === 0 && readyCount === 0;

    if (!wasPreviouslyDownloading || !queueIsFinished) return;

    const nowMs = Date.now();
    if (nowMs < attentionCooldownUntilMs.value) return;

    attentionCooldownUntilMs.value = nowMs + 2000;

    if (previousActiveDownloadCount.value > 1) {
      await notify(NotificationKind.QueueFinished, { n: previousActiveDownloadCount.value.toString() });
    }
    try {
      // macOS completion stays quiet: requesting attention bounces the Dock icon.
      if (currentPlatform() !== Platform.MacOS) {
        await windowHandle.requestUserAttention(UserAttentionType.Informational);
      }
    } catch {
      // ignore
    }
  }

  async function syncWindowState(): Promise<void> {
    const counts = progressStore.findAllCounts();
    const activeDownloadCount = counts.downloading;
    const readyCount = counts.ready;

    // Each OS call only when its own value changed (the key covers both together).
    const badge = activeDownloadCount + readyCount;
    const bar = computeBar(activeDownloadCount, counts.done, counts.total);
    const barKey = `${bar.status}|${bar.status === 'normal' ? bar.percent : ''}`;
    if (!sentOnce || badge !== lastBadge) await setDockOrTaskbarBadge(badge);
    if (!sentOnce || barKey !== lastBarKey) await setProgressBarState(bar);
    lastBadge = badge;
    lastBarKey = barKey;
    sentOnce = true;
    await maybeRequestUserAttention({ activeDownloadCount, readyCount });

    previousActiveDownloadCount.value = activeDownloadCount;
  }

  watch(
    signalKey,
    () => {
      void syncWindowState();
    },
    { immediate: true },
  );
}
