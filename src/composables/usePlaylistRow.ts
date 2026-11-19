import { computed, type Ref } from 'vue';
import { useI18n } from 'vue-i18n';
import type { Group } from '../tauri/types/group';
import { MediaState, useMediaStateStore } from '../stores/media/state';
import { useMediaProgressStore } from '../stores/media/progress';
import { useMediaStore } from '../stores/media/media';
import { useMediaOptionsStore } from '../stores/media/options';
import { useMediaDestinationStore } from '../stores/media/destination';
import { useLoginStore } from '../stores/login';
import { useToastStore } from '../stores/toast';
import { useSettingsStore } from '../stores/settings';
import { useUiPrefs } from './useUiPrefs';
import { useOpener } from './useOpener';
import { useListRemoval } from './useListRemoval';
import { phaseOf } from '../helpers/groupView';
import { playlistAggregate, type PlaylistChildFacts } from '../helpers/playlistAggregate';
import { ENGINE_FANOUT, runLimited } from '../helpers/runLimited';
import { defaultDownloadOptions } from '../helpers/quickChoice';
import { parentFolder } from '../helpers/folderPath';
import { ProgressStage } from '../tauri/types/progress';
import { countUsage } from '../tauri/usage';

/** Aggregate state and whole-playlist actions for one playlist row. Every action runs the per-download engine call. */
export function usePlaylistRow(parentId: Ref<string>, children: Ref<Group[]>) {
  const { t } = useI18n();
  const stateStore = useMediaStateStore();
  const progressStore = useMediaProgressStore();
  const mediaStore = useMediaStore();
  const optionsStore = useMediaOptionsStore();
  const destinationStore = useMediaDestinationStore();
  const loginStore = useLoginStore();
  const toastStore = useToastStore();
  const settingsStore = useSettingsStore();
  const { defaultChoice } = useUiPrefs();
  const { openPath, revealPath } = useOpener();
  const { removeRows } = useListRemoval();

  const phaseOfChild = (child: Group) => phaseOf(stateStore.getGroupState(child.id) ?? MediaState.fetching);

  const facts = computed<PlaylistChildFacts[]>(() => children.value.map((child) => {
    const phase = phaseOfChild(child);
    const progress = phase === 'downloading' || phase === 'paused' ? progressStore.findDownloadProgress(child.id) : undefined;
    return {
      phase,
      queued: phase === 'downloading' && (!progress || progress.stage === ProgressStage.queued),
      percent: progress?.percentage ?? null,
      size: child.filesize || progress?.totalBytes || 0,
      downloaded: progress?.downloadedBytes,
      speedBps: progress?.stage === ProgressStage.downloading ? progress?.speedBps : 0,
      needsLogin: loginStore.cardFor(child.id)?.kind === 'needed',
    };
  }));

  const aggregate = computed(() => {
    const a = playlistAggregate(facts.value);
    if (loginStore.cardFor(parentId.value)?.kind === 'needed') a.needsYou = true;
    return a;
  });

  const idsIn = (...phases: string[]) => children.value.filter(child => phases.includes(phaseOfChild(child)));

  function reportSome(failed: number, key: string) {
    if (failed > 0) toastStore.showToast(t(key, { n: failed }, failed), { style: 'error' });
  }

  async function pauseAll() {
    countUsage('pause');
    const results = await runLimited(idsIn('downloading'), ENGINE_FANOUT, child => mediaStore.pauseGroup(child.id));
    reportSome(results.filter(ok => ok !== true).length, 'downloads.playlist.pauseSomeFailed');
  }

  function optionsFor(child: Group) {
    const options = defaultDownloadOptions(child, optionsStore.getOptions(child.id), defaultChoice.value);
    optionsStore.setOptions(child.id, options);
    return options;
  }

  /** Paused, cancelled and never-started downloads; finished and failed ones stay as they are. */
  async function resumeAll() {
    const targets = idsIn('paused', 'cancelled', 'ready');
    if (targets.length > 1) countUsage('funnel.batch');
    const results = await runLimited(targets, ENGINE_FANOUT, child => mediaStore.downloadGroup(child.id, optionsFor(child), targets.length > 1 ? 'batch' : undefined));
    reportSome(results.filter(ok => ok === false).length, 'downloads.playlist.resumeSomeFailed');
  }

  /** Cancels every unfinished download; one Undo brings all of them back. Saved files stay. */
  function cancelAll() {
    const undos = idsIn('downloading', 'paused', 'ready').map(child => mediaStore.cancelDownload(child.id)).filter(Boolean) as (() => Promise<boolean | undefined>)[];
    if (!undos.length) return;
    toastStore.showToast(t('downloads.playlist.cancelledToast'), {
      style: 'info',
      duration: 5000,
      action: {
        label: t('downloads.list.undo'),
        run: () => void runLimited(undos, ENGINE_FANOUT, undo => undo()).then((results) => {
          reportSome(results.filter(ok => ok === false).length, 'downloads.playlist.resumeSomeFailed');
        }),
      },
    });
  }

  async function retryFailed() {
    countUsage('retry');
    await runLimited(idsIn('error'), ENGINE_FANOUT, child => mediaStore.retryGroup(child.id));
  }

  /** The folder a saved item is in; before anything is saved, the folder downloads go to. */
  async function openFolder() {
    const saved = children.value.map(child => destinationStore.getPrimaryDestination(child.id)?.path).find(Boolean);
    try {
      if (saved) {
        await revealPath(saved);
        return;
      }
      const first = children.value[0];
      const dir = (first && optionsStore.getOverrides(first.id)?.output?.downloadDir) || settingsStore.settings.output.downloadDir;
      if (!dir) throw new Error('no folder');
      try {
        await openPath(dir);
      } catch {
        // The default folder is created by the first download; until then open its parent.
        const parent = parentFolder(dir);
        if (!parent) throw new Error('no folder');
        await openPath(parent);
      }
    } catch {
      toastStore.showToast(t('feedback.folderFailed'), { style: 'error' });
    }
  }

  function removeFromList() {
    removeRows(children.value.map(child => child.id));
  }

  return { aggregate, pauseAll, resumeAll, cancelAll, retryFailed, openFolder, removeFromList, phaseOfChild };
}
