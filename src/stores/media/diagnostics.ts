import { defineStore } from 'pinia';
import { ref } from 'vue';
import { MediaState, useMediaStateStore } from './state.ts';
import { useMediaGroupStore } from './group.ts';
import { notifyGroup } from '../../tauri/notifications';
import { NotificationKind } from '../../tauri/types/app';
import { useMediaStore } from './media.ts';
import { reportFailedDownload } from '../report';
import { downloadReportContext } from '../../helpers/reportContext';

export interface MediaAttempts {
  id: string;
  groupId: string;
  attempts: { stage: string; login: boolean; outcome: string; code: string | null; ms: number }[];
  winner: number | null;
  finalCode: string | null;
}

export const useMediaDiagnosticsStore = defineStore('media-diagnostics', () => {
  const diagnostics = ref<Record<string, MediaDiagnostic[]>>({});
  const attempts = ref<Record<string, MediaAttempts>>({});
  const fatals = ref<Record<string, MediaFatal>>({});

  const stateStore = useMediaStateStore();
  const groupStore = useMediaGroupStore();
  const mediaStore = useMediaStore();

  function processMediaAttemptsPayload(payload: MediaAttempts) {
    attempts.value[payload.id] = payload;
  }

  function findAttemptsByGroupId(groupId: string): MediaAttempts[] {
    return Object.values(attempts.value).filter(report => report.groupId === groupId);
  }

  function processMediaDiagnosticPayload(payload: MediaDiagnostic) {
    diagnostics.value[payload.id] = diagnostics.value[payload.id] ?? [];
    diagnostics.value[payload.id].push(payload);
  }

  function processMediaFatalPayload(payload: MediaFatal) {
    fatals.value[payload.id] = payload;
    const { groupId } = payload;
    const currentState = stateStore.getGroupState(groupId);
    const group = groupStore.findGroupById(groupId);
    if (!group) throw new Error('Orphaned media item found during error handling.');
    // Automatic report (Rust checks the Settings switch and sends the link shape only).
    reportFailedDownload({
      context: downloadReportContext(findLastDiagnosticByGroupId(groupId)?.code, payload.exitCode, findAttemptsByGroupId(groupId)),
      url: group.url ?? null,
      extractor: group.extractor ?? null,
    });
    if (currentState === MediaState.fetching || currentState === MediaState.fetchingList) {
      mediaStore.rejectPendingReadyGroup(groupId, payload.message);
    }
    group.processed++;
    group.errored++;
    const leader = groupStore.findGroupLeader(groupId);
    if (leader && leader.entries) {
      if (stateStore.getGroupState(groupId) === MediaState.fetchingList) {
        return;
      }
      // We are combined, so we only set one item to error.
      stateStore.setState(payload.id, MediaState.error);
      const itemsWithoutLeader = Object.values(group.items).filter(item => !item.isLeader);
      const allItemsAreTerminal = itemsWithoutLeader.every((item) => {
        const state = stateStore.getState(item.id);
        return state === MediaState.done || state === MediaState.error;
      });
      if (allItemsAreTerminal) {
        const allItemsFailed = itemsWithoutLeader.every(
          item => stateStore.getState(item.id) === MediaState.error,
        );
        stateStore.setGroupState(
          groupId,
          allItemsFailed ? MediaState.error : MediaState.done,
        );
        if (allItemsFailed) {
          void notifyGroup(NotificationKind.DownloadFailed, group, { message: payload.message });
        }
      }
    } else {
      // We are not combined, so we set all items to error.
      if (group.processed === group.total && group.total > 1) {
        mediaStore.finalizePlaylistGroup(group);
      }
      stateStore.setGroupState(groupId, MediaState.error);
      void notifyGroup(NotificationKind.DownloadFailed, group, { message: payload.message });
    }
  }

  function findFatalsByGroupId(groupId: string): MediaFatal[] {
    const group = groupStore.findGroupById(groupId);
    if (!group) return [];
    return Object.values(fatals.value).reduce(
      (acc, fatal) => {
        if (fatal.groupId === group.id) {
          acc.push(fatal);
        }
        return acc;
      },
      [] as MediaFatal[],
    );
  }

  function findDiagnosticsByGroupId(groupId: string): MediaDiagnostic[] {
    const group = groupStore.findGroupById(groupId);
    if (!group) return [];
    return Object.values(diagnostics.value).reduce(
      (acc, items) => {
        if (items.some(i => i.groupId === group.id)) {
          acc.push(...items);
        }
        return acc;
      },
      [] as MediaDiagnostic[],
    );
  }

  function findLastFatalByGroupId(groupId: string): MediaFatal | undefined {
    const fatals = findFatalsByGroupId(groupId);
    return fatals[fatals.length - 1];
  }

  function findLastDiagnosticByGroupId(groupId: string): MediaDiagnostic | undefined {
    const diagnostics = findDiagnosticsByGroupId(groupId);
    return diagnostics[diagnostics.length - 1];
  }

  function removeDiagnostics(id: string) {
    delete diagnostics.value[id];
    delete fatals.value[id];
    delete attempts.value[id];
  }

  return { diagnostics, fatals, attempts, processMediaAttemptsPayload, findAttemptsByGroupId, findLastFatalByGroupId, findLastDiagnosticByGroupId, findFatalsByGroupId, findDiagnosticsByGroupId, processMediaDiagnosticPayload, processMediaFatalPayload, removeDiagnostics };
});
