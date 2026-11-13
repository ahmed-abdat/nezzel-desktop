import { defineStore } from 'pinia';
import { ProgressStage } from '../../tauri/types/progress';
import type { MediaDestinationPayload } from '../../tauri/types/destination';
import type { LibraryDisplay, LibraryRecord } from '../../tauri/types/library';
import { libraryPathKey } from '../../helpers/folderPath';
import { normalizePastedUrl } from '../../helpers/url';
import { invoke } from '@tauri-apps/api/core';
import { v4 as uuidv4 } from 'uuid';
import { MediaState, useMediaStateStore } from './state';
import { useMediaGroupStore } from './group';
import { useMediaDestinationStore } from './destination';
import { useMediaProgressStore } from './progress';
import { useMediaOptionsStore } from './options';
import { DownloadOptions, DownloadOverrides, MediaAddPayload, MediaItem, TrackType } from '../../tauri/types/media';
import { useMediaSizeStore } from './size.ts';
import { useMediaDiagnosticsStore } from './diagnostics.ts';
import { useSettingsStore } from '../settings.ts';
import { useListVisibilityStore } from '../listVisibility';
import { useLoginStore } from '../login';
import { Group } from '../../tauri/types/group.ts';
import { notify, notifyGroup } from '../../tauri/notifications';
import { NotificationKind } from '../../tauri/types/app';
import { resolvePlaylistIndex } from '../../helpers/playlistNumbering';
import { optionsForCaptureChoice } from '../../helpers/captureChoices';
import { defaultDownloadOptions } from '../../helpers/quickChoice';
import { useUiPrefs } from '../../composables/useUiPrefs';
import type { CaptureChoice } from '@/generated/contract';
import { countUsage } from '../../tauri/usage';
import { settingsToInputFilterOverride } from '../../helpers/inputFilters.ts';
import {
  applyPlaylistSelectionToEntries,
  buildPlaylistItemsSpec,
  type PlaylistSelection,
} from '../../helpers/playlistSelection.ts';

type PendingReadyGroup = {
  resolve: (groupIds: string[]) => void;
  stopAtPlaylist?: boolean;
  reject: (reason?: unknown) => void;
};

/** Media/options are JSON contracts. A roundtrip also unwraps nested Vue proxies for isolation IPC. */
function cloneMediaData<T>(value: T): T {
  return value == null ? value : JSON.parse(JSON.stringify(value));
}

export const useMediaStore = defineStore('media', () => {
  const groupStore = useMediaGroupStore();
  const stateStore = useMediaStateStore();
  const progressStore = useMediaProgressStore();
  const destinationStore = useMediaDestinationStore();
  const optionsStore = useMediaOptionsStore();
  const sizeStore = useMediaSizeStore();
  const diagnosticsStore = useMediaDiagnosticsStore();
  const settingsStore = useSettingsStore();
  const { defaultChoice } = useUiPrefs();
  const removedLibraryPaths = new Set<string>();
  let libraryLoad: Promise<void> | undefined;
  const libraryGroupsByPath = new Map<string, Set<string>>();
  const pendingLibraryRemoval = new Set<string>();
  let removeTimer: ReturnType<typeof setTimeout> | undefined;
  const removedFailures = new Set<string>();
  const historyIds = new Map<string, string>();
  let failureWrites = Promise.resolve();

  function historyRecordId(groupId: string, itemId: string) {
    if (groupId.startsWith('library:')) return groupId.slice('library:'.length);
    const key = `${groupId}:${itemId}`;
    if (!historyIds.has(key)) historyIds.set(key, uuidv4());
    return historyIds.get(key)!;
  }

  function libraryMetadata(item: MediaItem | Group, groupId: string) {
    const options = optionsStore.getOptions(groupId);
    const height = options?.trackType === TrackType.audio ? undefined : (options?.height ?? Math.max(0, ...(item.formats ?? []).map(f => f.height ?? 0)));
    const display: LibraryDisplay = {
      duration: item.duration, height: height || undefined, fps: options?.fps, trackType: options?.trackType,
    };
    if (item.attachments?.length) {
      const ids = optionsStore.getAttachmentSelection(groupId);
      const selected = item.attachments.filter(a => ids.includes(a.id));
      display.attachmentSummary = {
        photos: selected.filter(a => a.kind === 'image').length,
        videos: selected.filter(a => a.kind === 'video').length,
      };
    }
    const itemId = 'items' in item ? groupStore.findGroupLeader(groupId)?.id ?? item.id : item.id;
    return { historyId: historyRecordId(groupId, itemId), title: item.title, site: item.extractor, thumbnail: item.thumbnail, display };
  }

  function failureRecordId(groupId: string) {
    const itemId = groupStore.findGroupLeader(groupId)?.id ?? groupId;
    // Capture group and item counters restart with the app; history has its own UUID.
    return historyRecordId(groupId, itemId);
  }

  function rememberFailedGroup(groupId: string) {
    const group = groupStore.findGroupById(groupId);
    if (!group || stateStore.getGroupState(groupId) !== MediaState.error) return;
    // Match the success record's identity, so a late failure cannot replace a saved file.
    const id = failureRecordId(groupId);
    removedFailures.delete(id);
    const row = cloneMediaData({
      id, url: group.url, kind: group.mediaKind ?? 'video', ...libraryMetadata(group, groupId),
      errorCode: diagnosticsStore.findLastDiagnosticByGroupId(groupId)?.code ?? 'unknown',
    });
    failureWrites = failureWrites.then(() => invoke<void>('library_record_failure', { row }))
      .catch(() => console.warn('Unable to save local download history'));
  }

  function forgetFailedGroup(groupId: string) {
    const id = failureRecordId(groupId);
    removedFailures.add(id);
    failureWrites = failureWrites.then(() => invoke<void>('library_remove', { paths: [], ids: [id] }))
      .catch(() => console.warn('Unable to remove local download history'));
  }

  function queueLibraryRemoval(paths: string[]) {
    for (const path of paths) pendingLibraryRemoval.add(path);
    clearTimeout(removeTimer);
    removeTimer = setTimeout(() => {
      const paths = [...pendingLibraryRemoval];
      pendingLibraryRemoval.clear();
      removeTimer = undefined;
      void invoke('library_remove', { paths }).catch(() => console.warn('Unable to remove local download history'));
    }, 0);
  }

  function forgetLibraryPath(id: string, path: string) {
    const key = libraryPathKey(path);
    const ids = libraryGroupsByPath.get(key);
    ids?.delete(id);
    if (ids?.size === 0) libraryGroupsByPath.delete(key);
  }

  function mergeLibrary(rows: LibraryRecord[]) {
    const knownPaths = new Set(Object.values(destinationStore.destinations).flat().map(d => libraryPathKey(d.path)));
    const sessionItems = new Set(historyIds.values());
    for (const row of rows) {
      const paths = row.paths.filter(p => !knownPaths.has(libraryPathKey(p)) && !removedLibraryPaths.has(libraryPathKey(p)));
      const id = `library:${row.id}`;
      if ((!row.errorCode && paths.length === 0) || removedFailures.has(row.id) || groupStore.findGroupById(id)
        || (row.errorCode && (groupStore.findGroupById(row.id) || sessionItems.has(row.id)))) continue;
      const summary = row.display?.attachmentSummary ?? (row.kind === 'image' ? { photos: paths.length, videos: 0 } : undefined);
      const kind = summary?.photos ? (summary.videos ? 'mixed' : summary.photos > 1 ? 'carousel' : 'image') : row.kind;
      const item: MediaItem = {
        id, groupId: id, isLeader: true, url: row.url, title: row.title,
        extractor: row.site ?? undefined, thumbnail: row.thumbnail ?? undefined,
        mediaKind: kind, filesize: row.size, audioCodecs: [], videoCodecs: [], formats: [],
        duration: row.display?.duration ?? undefined,
      };
      groupStore.groups[id] = {
        ...item, total: 1, processed: 1, errored: row.errorCode ? 1 : 0, isCombined: false, items: { [id]: item },
        savedAttachmentSummary: summary,
      };
      groupStore.groupOrder.push(id);
      stateStore.setState(id, row.errorCode ? MediaState.error : MediaState.done);
      optionsStore.setOptions(id, {
        trackType: row.display?.trackType ?? (row.kind === 'audio' ? TrackType.audio : TrackType.both),
        height: row.display?.height ?? undefined, fps: row.display?.fps ?? undefined,
      });
      if (row.errorCode) diagnosticsStore.processMediaDiagnosticPayload({
        id, groupId: id, code: row.errorCode, level: 'error', component: 'library', message: row.errorCode, raw: '', timestamp: row.completedAt,
      });
      for (const path of paths) {
        destinationStore.addDestination(id, { path, confidence: 100 });
        const key = libraryPathKey(path);
        knownPaths.add(key);
        const ids = libraryGroupsByPath.get(key) ?? new Set<string>();
        ids.add(id);
        libraryGroupsByPath.set(key, ids);
      }
    }
  }

  function loadLibrary(): Promise<void> {
    libraryLoad ??= (async () => {
      mergeLibrary(await invoke<LibraryRecord[]>('library_list', { scan: false }) ?? []);
      mergeLibrary(await invoke<LibraryRecord[]>('library_list', { scan: true }) ?? []);
    })();
    return libraryLoad;
  }

  // Only the arriving destination can become a duplicate; no traversal on unrelated state changes.
  function processMediaDestinationPayload(payload: MediaDestinationPayload) {
    const group = groupStore.findGroupById(payload.groupId);
    if (!group) return;
    destinationStore.processMediaDestinationPayload(payload);
    const item = group.items[payload.id];
    if (item && !item.title?.trim() && payload.destination.confidence === 100) {
      const name = payload.destination.path.split(/[\\/]/).pop();
      if (name) {
        item.title = name;
        if (item.isLeader && !group.title?.trim()) group.title = name;
      }
    }
    const key = libraryPathKey(payload.destination.path);
    const duplicates = libraryGroupsByPath.get(key);
    if (!duplicates) return;
    for (const id of [...duplicates]) {
      if (id === payload.groupId) continue;
      const remaining = destinationStore.getDestinations(id).filter(d => libraryPathKey(d.path) !== key);
      forgetLibraryPath(id, payload.destination.path);
      if (remaining.length) destinationStore.destinations[id] = remaining;
      else {
        destinationStore.deleteDestinations(id);
        stateStore.removeState(id);
        useLoginStore().forgetGroup(id);
        groupStore.deleteGroup(id);
      }
    }
  }

  const pendingReadyGroups = new Map<string, PendingReadyGroup[]>();
  const recentCaptures = new Map<string, { groupId: string; createdAt: number }>();

  function resolvePendingReadyGroup(groupId: string, resolvedIds: string[], playlistOnly = false) {
    const pending = pendingReadyGroups.get(groupId) ?? [];
    const remaining = pending.filter(waiter => playlistOnly && !waiter.stopAtPlaylist);
    if (remaining.length) pendingReadyGroups.set(groupId, remaining);
    else pendingReadyGroups.delete(groupId);
    for (const waiter of pending) {
      if (!playlistOnly || waiter.stopAtPlaylist) waiter.resolve(resolvedIds);
    }
  }

  function rejectPendingReadyGroup(groupId: string, reason?: unknown) {
    const pending = pendingReadyGroups.get(groupId) ?? [];
    pendingReadyGroups.delete(groupId);
    for (const waiter of pending) waiter.reject(reason);
  }

  function finalizePlaylistGroup(group: Group) {
    const splitThreshold = settingsStore.settings.performance.splitPlaylistThreshold;
    const overrides = optionsStore.getOverrides(group.id);
    if (group.total < splitThreshold) {
      const selectedOptions = optionsStore.getOptions(group.id);
      const newGroups = groupStore.splitGroup(group);
      if (selectedOptions) {
        for (const next of newGroups) optionsStore.setOptions(next.id, { ...selectedOptions });
      }
      // splitGroup removed the playlist row: its pending login card goes with it.
      useLoginStore().forgetGroup(group.id);
      migrateOverrides(group.id, newGroups.map(newGroup => newGroup.id), overrides);
      void notifyGroup(NotificationKind.PlaylistReady, group, {}, newGroups.length);
      resolvePendingReadyGroup(group.id, newGroups.map(newGroup => newGroup.id));
      if (newGroups.length === 0) {
        stateStore.setGroupState(group.id, MediaState.configure);
        return;
      }
      for (const newGroup of newGroups) {
        stateStore.setGroupState(newGroup.id, MediaState.configure);
      }
    } else {
      groupStore.consolidateGroup(group);
      void notifyGroup(NotificationKind.PlaylistReady, group, {}, group.entries?.length ?? 1);
      stateStore.setGroupState(group.id, MediaState.configure);
      resolvePendingReadyGroup(group.id, [group.id]);
    }
  }

  function processMediaAddPayload(payload: MediaAddPayload) {
    const { item, groupId, total } = payload;
    item.groupId = groupId;

    const group = groupStore.findGroupById(groupId);
    if (!group) throw new Error('Orphaned media item found.');
    if (!item.title?.trim()) {
      const destination = destinationStore.getPrimaryDestination(groupId);
      if (destination?.confidence === 100) item.title = destination.path.split(/[\\/]/).pop() ?? item.title;
    }

    if (item.entries) {
      const existingId = Object.keys(group.items)[0];
      if (existingId && existingId !== item.id) {
        delete group.items[existingId];
      }

      item.isLeader = true;
      group.items[item.id] = item;
      const { id, groupId: gid, isLeader, ...meta } = item;
      void id;
      void gid;
      void isLeader;
      Object.assign(group, meta);
      group.total = total;
      group.processed = 0;
      if (pendingReadyGroups.get(groupId)?.some(waiter => waiter.stopAtPlaylist)) {
        group.skipPlaylistSelection = false;
        stateStore.setState(item.id, MediaState.playlistSelection);
        resolvePendingReadyGroup(groupId, [groupId], true);
      } else if (group.skipPlaylistSelection) {
        void expandPlaylistGroup(groupId, { rows: [] }).catch((error) => {
          console.error(error);
          stateStore.setState(item.id, MediaState.playlistSelection);
        });
      } else {
        stateStore.setState(item.id, MediaState.playlistSelection);
      }
      return;
    }

    const leader = groupStore.findGroupLeader(groupId);
    const hasPlaylistLeader = !!leader?.entries;
    const isFirst = group.total === 1 && !hasPlaylistLeader;
    if (isFirst) {
      const existingId = Object.keys(group.items)[0];
      if (existingId) delete group.items[existingId];
    }
    item.isLeader = isFirst;
    group.items[item.id] = item;

    if (isFirst) {
      const { id, groupId: gid, isLeader, ...meta } = item;
      void id;
      void gid;
      void isLeader;
      Object.assign(group, meta);
    }

    group.total = total;
    if (!isFirst) group.processed++;

    if (group.processed === total) {
      finalizePlaylistGroup(group);
    } else if (group.total === 1 && !hasPlaylistLeader) {
      void notifyGroup(NotificationKind.VideoReady, group);
      resolvePendingReadyGroup(group.id, [group.id]);
    }

    const next = total > 1 && isFirst
      ? MediaState.fetchingList
      : MediaState.configure;
    stateStore.setState(item.id, next);
  }

  async function expandPlaylistGroup(groupId: string, selection: PlaylistSelection) {
    const group = groupStore.findGroupById(groupId);
    const leader = groupStore.findGroupLeader(groupId);
    const entries = group?.entries;
    if (!group || !leader?.entries || !entries) {
      throw new Error(`Playlist group ${groupId} is missing entry metadata.`);
    }

    const selectedEntries = applyPlaylistSelectionToEntries(entries, selection);
    if (selectedEntries.length === 0) {
      throw new Error('No playlist entries match the selected range.');
    }
    const spec = buildPlaylistItemsSpec(selection, entries);
    const previousOverrides = optionsStore.getOverrides(groupId);
    const previousTotal = group.total;
    const previousProcessed = group.processed;
    const nextOverrides = cloneMediaData(previousOverrides) ?? {};

    if (spec) {
      nextOverrides.inputFilters = {
        ...(nextOverrides.inputFilters ?? {}),
        playlistItems: spec,
      };
    } else if (nextOverrides.inputFilters) {
      delete nextOverrides.inputFilters.playlistItems;
      if (Object.keys(nextOverrides.inputFilters).length === 0) {
        delete nextOverrides.inputFilters;
      }
    }

    if (Object.keys(nextOverrides).length > 0) {
      optionsStore.setOverrides(groupId, nextOverrides);
    } else {
      optionsStore.removeOverrides(groupId);
    }

    group.entries = [...selectedEntries];
    leader.entries = [...selectedEntries];
    group.total = selectedEntries.length;
    group.processed = 0;
    stateStore.setGroupState(groupId, MediaState.fetchingList);

    try {
      await invoke<string>('media_playlist_expand', {
        groupId,
        entries: selectedEntries,
        overrides: cloneMediaData(optionsStore.getOverrides(groupId)),
      });
    } catch (error) {
      group.entries = entries;
      leader.entries = entries;
      group.total = previousTotal;
      group.processed = previousProcessed;
      if (previousOverrides) optionsStore.setOverrides(groupId, previousOverrides);
      else optionsStore.removeOverrides(groupId);
      stateStore.setGroupState(groupId, MediaState.playlistSelection);
      throw error;
    }
  }

  /** Selection is the user's download confirmation; read only those entries, then queue them. */
  async function downloadPlaylistSelection(groupId: string, selection: PlaylistSelection) {
    const ready = waitForGroupReady(groupId);
    const expansion = expandPlaylistGroup(groupId, selection).catch((error) => {
      rejectPendingReadyGroup(groupId, error);
      throw error;
    });
    const [, ids] = await Promise.all([expansion, ready]);
    await Promise.all(ids.map(id => downloadGroup(id, getDownloadOptions(id), ids.length > 1 ? 'batch' : undefined)));
  }

  function createMediaInfoGroup(
    url: string,
    fromShortcut: boolean = false,
    skipPlaylistSelection: boolean = false,
    groupId: string = uuidv4(),
  ) {
    url = normalizePastedUrl(url);
    const id = uuidv4();

    stateStore.setState(id, MediaState.fetching);
    const newGroup: Group = {
      id: groupId,
      total: 1,
      processed: 0,
      errored: 0,
      isCombined: false,
      url,
      audioCodecs: [],
      videoCodecs: [],
      audioTracks: [],
      videoTracks: [],
      formats: [],
      filesize: 0,
      fromShortcut,
      skipPlaylistSelection,
      items: {
        [id]: {
          id,
          isLeader: true,
          url,
          audioCodecs: [],
          videoCodecs: [],
          audioTracks: [],
          videoTracks: [],
          formats: [],
          filesize: 0,
        },
      },
    };
    groupStore.createGroup(newGroup);

    const inputFiltersOverride = settingsToInputFilterOverride(settingsStore.settings);
    if (inputFiltersOverride) {
      optionsStore.setOverrides(groupId, { inputFilters: inputFiltersOverride });
    }

    return newGroup;
  }

  /** A captured link's row; `groupId` comes from its capture id, so a later claim on it finds the same row. */
  function createCaptureGroup(groupId: string, url: string) {
    const group = createMediaInfoGroup(url, true, false, groupId);
    recentCaptures.set(url, { groupId: group.id, createdAt: Date.now() });
    void notifyGroup(NotificationKind.QueueAdded, group);
    return group.id;
  }

  function hasRecentCapture(url: string) {
    const capture = recentCaptures.get(url);
    if (!capture) return false;
    if (Date.now() - capture.createdAt < 5000 && groupStore.findGroupById(capture.groupId)) return true;
    recentCaptures.delete(url);
    return false;
  }

  async function fetchMediaInfoForGroup(groupId: string) {
    const group = groupStore.findGroupById(groupId);
    const leader = groupStore.findGroupLeader(groupId);
    if (!group || !leader) return;
    try {
      await invoke('media_info', {
        url: group.url,
        id: leader.id,
        groupId,
        overrides: cloneMediaData(optionsStore.getOverrides(groupId)),
      });
    } catch (error) {
      stateStore.setState(leader.id, MediaState.error);
      rememberFailedGroup(groupId);
      rejectPendingReadyGroup(groupId, error);
      throw error;
    }
  }

  async function retryGroup(groupId: string) {
    const group = groupStore.findGroupById(groupId);
    const leader = groupStore.findGroupLeader(groupId);
    if (!group || !leader) throw new Error('Missing download');
    forgetFailedGroup(groupId);
    delete group.savedAttachmentSummary;
    for (const itemId of Object.keys(group.items)) {
      stateStore.removeState(itemId);
      progressStore.deleteProgress(itemId);
      sizeStore.removeSizes(itemId);
      diagnosticsStore.removeDiagnostics(itemId);
    }
    diagnosticsStore.removeDiagnostics(groupId);
    destinationStore.deleteDestinations(groupId);
    group.items = { [leader.id]: {
      id: leader.id, isLeader: true, url: group.url, audioCodecs: [], formats: [], filesize: 0,
    } };
    group.total = 1;
    group.processed = 0;
    group.errored = 0;
    group.isCombined = false;
    delete group.entries;
    stateStore.setGroupState(groupId, MediaState.fetching);
    await fetchMediaInfoForGroup(groupId);
  }

  async function dispatchMediaInfoFetch(
    url: string,
    fromShortcut: boolean = false,
    skipPlaylistSelection: boolean = false,
    preparedItem?: MediaItem,
  ) {
    const group = createMediaInfoGroup(url, fromShortcut, skipPlaylistSelection);
    const groupId = group.id;

    if (preparedItem) {
      processMediaAddPayload({ groupId, total: preparedItem.entries?.length ?? 1, item: preparedItem });
    } else {
      await fetchMediaInfoForGroup(groupId);
    }
    void notifyGroup(NotificationKind.QueueAdded, group);
    return groupId;
  }

  function migrateOverrides(
    previousGroupId: string,
    nextGroupIds: string[],
    overrides?: DownloadOverrides,
  ) {
    if (!overrides) return;
    for (const nextGroupId of nextGroupIds) {
      const clonedOverrides = cloneMediaData(overrides);
      if (clonedOverrides) {
        optionsStore.setOverrides(nextGroupId, clonedOverrides);
      }
    }
    optionsStore.removeOverrides(previousGroupId);
  }

  function waitForGroupReady(groupId: string, stopAtPlaylist = false): Promise<string[]> {
    const group = groupStore.findGroupById(groupId);
    const state = stateStore.getGroupState(groupId);

    if (group && (state === MediaState.configure || (stopAtPlaylist && state === MediaState.playlistSelection))) {
      return Promise.resolve([groupId]);
    }

    if (!group || state === MediaState.error) return Promise.reject(new Error('Download is unavailable'));
    return new Promise<string[]>((resolve, reject) => {
      const pending = pendingReadyGroups.get(groupId) ?? [];
      pending.push({ resolve, reject, stopAtPlaylist });
      pendingReadyGroups.set(groupId, pending);
    });
  }

  function getDownloadOptions(groupId: string): DownloadOptions {
    return optionsStore.getOptions(groupId) ?? optionsStore.getGlobalOptions() ?? { trackType: TrackType.both };
  }

  async function addAndDownload(url: string, fromShortcut: boolean = false) {
    await addUrlBatchAndDownload([url], fromShortcut, true);
  }

  async function addUrlBatch(
    urls: string[],
    fromShortcut: boolean = false,
    skipPlaylistSelection: boolean = false,
  ): Promise<string[]> {
    const results = await Promise.allSettled(
      urls.map(url => dispatchMediaInfoFetch(url, fromShortcut, skipPlaylistSelection)),
    );
    return results.flatMap(result => result.status === 'fulfilled' ? [result.value] : []);
  }

  async function addUrlBatchAndDownload(
    urls: string[],
    fromShortcut: boolean = false,
    skipPlaylistSelection: boolean = false,
  ): Promise<string[]> {
    const initialGroupIds = await addUrlBatch(urls, fromShortcut, skipPlaylistSelection);
    const readyGroupIds = [...new Set(
      (await Promise.all(initialGroupIds.map(groupId => waitForGroupReady(groupId)))).flat(),
    )];

    await notify(NotificationKind.QueueDownloading, { n: readyGroupIds.length.toString() }, fromShortcut);
    const origin = readyGroupIds.length > 1 ? 'batch' : undefined;
    if (origin) countUsage('funnel.batch');
    await Promise.all(
      readyGroupIds.map(groupId => downloadGroup(groupId, getDownloadOptions(groupId), origin)),
    );

    return readyGroupIds;
  }

  /** Reuse the capture read, stopping before playlist expansion for every capture choice. */
  async function prepareCaptureGroup(groupId: string, choice?: CaptureChoice) {
    const ready = await waitForGroupReady(groupId, true);
    for (const id of ready) {
      const group = groupStore.findGroupById(id);
      if (!group) continue;
      const sourceKind = group.entries ? 'video' : group.mediaKind;
      optionsStore.setOptions(id, choice
        ? optionsForCaptureChoice(choice, group.formats ?? [], sourceKind)
        : defaultDownloadOptions({ ...group, mediaKind: sourceKind }, optionsStore.getOptions(id), defaultChoice.value));
    }
    return ready;
  }

  /** `choice`: a quick pick or a copied-link card format row; none = the user's default. */
  async function downloadPreparedGroup(groupId: string, choice?: CaptureChoice) {
    const ready = await prepareCaptureGroup(groupId, choice);
    if (stateStore.getGroupState(groupId) === MediaState.playlistSelection) return 'playlist';
    await Promise.all(ready.map(id => downloadGroup(id, getDownloadOptions(id))));
    return 'downloaded';
  }

  /** `origin`: 'batch' when one action starts several links (the caller counts funnel.batch once); a paused group
   * is a resume (usage counts only). */
  async function downloadGroup(
    groupId: string,
    options: DownloadOptions,
    origin?: 'batch',
  ) {
    const group = groupStore.findGroupById(groupId);
    if (!group) return;
    const prior = stateStore.getGroupState(groupId);
    const resuming = prior === MediaState.paused || prior === MediaState.pausedList;
    const usageOrigin = resuming ? 'resume' : origin ?? 'tap';

    const items: MediaItem[] = Object.values(group.items);
    const itemsWithoutLeader: MediaItem[] = items.filter(item => !item.entries && stateStore.getState(item.id) !== MediaState.done);
    const encodings = optionsStore.getEncodings(groupId);
    const tracks = optionsStore.getTracks(groupId);
    const overrides = optionsStore.getOverrides(groupId);
    const resolvedOptions: DownloadOptions = {
      ...options,
      audioEncoding: encodings?.audio,
      videoEncoding: encodings?.video,
      audioTrack: tracks?.audio,
      videoTrack: tracks?.video,
    };

    if (itemsWithoutLeader.length === 0) return;
    const hasAttachments = itemsWithoutLeader.some(item => !!item.attachments?.length);
    const attachmentIds = hasAttachments ? optionsStore.getAttachmentSelection(groupId) : undefined;
    if (attachmentIds?.length === 0) return;

    // Keep the submitted choice even when it came straight from a global default or capture.
    optionsStore.setOptions(groupId, { ...options });
    const newState = group.isCombined ? MediaState.downloadingList : MediaState.downloading;
    items
      .filter(item => stateStore.getState(item.id) !== MediaState.done)
      .forEach((item) => {
        stateStore.setState(item.id, newState);
        if (!resuming) progressStore.deleteProgress(item.id);
        if (!item.entries) progressStore.processMediaProgressStagePayload({ id: item.id, groupId, stage: ProgressStage.queued });
      });

    try {
      await invoke<string>('media_download', cloneMediaData({
        groupId,
        origin: usageOrigin,
        metadata: Object.fromEntries(itemsWithoutLeader.map(item => [item.id, libraryMetadata(item, groupId)])),
        items: itemsWithoutLeader.map(item => ({
          id: item.id,
          url: item.url,
          format: resolvedOptions,
          fetchEngine: item.fetchEngine,
          resolvedUrl: item.resolvedUrl,
          mediaKind: item.mediaKind,
          ...(item.attachments?.length ? { attachmentIds, attachments: item.attachments } : {}),
          subtitleInventory: item.subtitleInventory,
          overrides,
          templateContext: {
            values: {
              ...buildTemplateContext(item, group, overrides),
              enriched_title: item.title ?? undefined,
              ...(item.attachments?.length || (item.resolvedUrl && item.resolvedUrl !== item.url)
                ? { title: item.title ?? undefined, fulltitle: item.title ?? undefined, uploader: item.uploader ?? undefined }
                : {}),
            },
          },
        })),
      }));
    } catch (e) {
      diagnosticsStore.processMediaFatalPayload({
        groupId,
        id: groupId,
        exitCode: 1,
        internal: true,
        message: `${e}`,
        details: null,
        timestamp: Date.now(),
      });
      console.error(e);
      return false;
    }
    return true;
  }

  async function pauseGroup(groupId: string) {
    const group = groupStore.findGroupById(groupId);
    if (!group) return;

    const items: MediaItem[] = Object.values(group.items);
    const itemsWithoutLeader: MediaItem[] = items.filter(item => !item.entries);

    if (itemsWithoutLeader.length === 0) return;

    const newState = group.isCombined ? MediaState.pausedList : MediaState.paused;
    items
      .filter(item => stateStore.getState(item.id) !== MediaState.done)
      .forEach(item => stateStore.setState(item.id, newState));

    try {
      await groupStore.cancelGroup(groupId);
      return true;
    } catch {
      // Cancellation did not reach the engine; the work is still running.
      items.filter(item => stateStore.getState(item.id) === newState)
        .forEach(item => stateStore.setState(item.id, group.isCombined ? MediaState.downloadingList : MediaState.downloading));
      return false;
    }
  }

  function buildTemplateContext(
    item: MediaItem,
    group: Group,
    overrides?: DownloadOverrides,
  ): Record<string, string | undefined> {
    const reversePlaylistNumbering = overrides?.output?.reversePlaylistNumbering === true;
    const playlistIndex = resolvePlaylistIndex(item.playlistIndex, group.playlistCount, reversePlaylistNumbering);

    return {
      playlist_index: playlistIndex?.toString(),
      playlist_id: group.playlistId ?? undefined,
      playlist_title: group.playlistTitle ?? undefined,
      playlist: group.playlistTitle ?? undefined,
      playlist_uploader: group.uploader ?? undefined,
      playlist_uploader_id: group.uploaderId ?? undefined,
      playlist_count: group.playlistCount?.toString(),
      n_entries: group.entries?.length?.toString(),
    };
  }

  async function downloadAllGroups(fromShortcut: boolean = false) {
    const group_ids = groupStore.groupOrder
      .filter(gid => stateStore.getGroupState(gid) === MediaState.configure);

    const origin = group_ids.length > 1 ? 'batch' : undefined;
    if (origin) countUsage('funnel.batch');
    await Promise.all(group_ids.map((gid) => {
      const opts = getDownloadOptions(gid);
      return downloadGroup(gid, opts, origin);
    }));
    await notify(NotificationKind.QueueDownloading, { n: group_ids.length.toString() }, fromShortcut);
  }

  function pauseAllGroups() {
    countUsage('pause_all');
    for (const groupId of groupStore.groupOrder) {
      const state = stateStore.getGroupState(groupId);
      if (state === MediaState.downloading || state === MediaState.downloadingList) {
        void pauseGroup(groupId);
      }
    }
  }

  function resumeAllGroups() {
    for (const groupId of groupStore.groupOrder) {
      const state = stateStore.getGroupState(groupId);
      if (state !== MediaState.paused && state !== MediaState.pausedList) continue;
      const options = optionsStore.getOptions(groupId);
      if (!options) {
        console.warn(`No options found for group: ${groupId}, cannot resume.`);
        continue;
      }
      void downloadGroup(groupId, options);
    }
  }

  /** Stop through the same removal path as the list; Undo starts a fresh engine generation. */
  function cancelDownload(id: string) {
    const original = groupStore.findGroupById(id);
    if (!original) return;
    const group = cloneMediaData(original);
    const itemStates = new Map(Object.values(group.items).map(item => [item.id, stateStore.getState(item.id)]));
    const destinations = cloneMediaData(destinationStore.getDestinations(id));
    const options = cloneMediaData(optionsStore.getOptions(id));
    const encodings = cloneMediaData(optionsStore.getEncodings(id));
    const tracks = cloneMediaData(optionsStore.getTracks(id));
    const overrides = cloneMediaData(optionsStore.getOverrides(id));
    const attachments = optionsStore.getAttachmentSelection(id);
    const cancellation = deleteGroup(id);
    let restored = false;
    return async () => {
      if (restored) return false;
      restored = true;
      await cancellation;
      // Old jobs may still emit events while stopping. Never reuse their IDs.
      group.id = uuidv4();
      group.items = Object.fromEntries(Object.values(group.items).map((item) => {
        const itemId = uuidv4();
        const state = itemStates.get(item.id);
        if (state) stateStore.setState(itemId, state);
        return [itemId, { ...item, id: itemId, groupId: group.id }];
      }));
      groupStore.createGroup(group);
      for (const destination of destinations) destinationStore.addDestination(group.id, destination);
      if (options) optionsStore.setOptions(group.id, options);
      if (encodings) optionsStore.setEncodings(group.id, encodings);
      if (tracks) optionsStore.setTracks(group.id, tracks);
      if (overrides) optionsStore.setOverrides(group.id, overrides);
      optionsStore.setAttachmentSelection(group.id, attachments);
      return downloadGroup(group.id, options ?? getDownloadOptions(group.id));
    };
  }

  function deleteGroup(id: string, removeHistory = true) {
    rejectPendingReadyGroup(id, new Error('Download was removed'));
    // Even when the row is already gone, a login card may still wait for it.
    useLoginStore().forgetGroup(id);
    const group = groupStore.findGroupById(id);
    if (!group) return;
    if (removeHistory && stateStore.getGroupState(id) === MediaState.error) forgetFailedGroup(id);
    const paths = destinationStore.getDestinations(id).map(d => d.path);
    if (removeHistory && paths.length && stateStore.getGroupState(id) === MediaState.done) {
      for (const path of paths) removedLibraryPaths.add(libraryPathKey(path));
      queueLibraryRemoval(paths);
      // History is gone, so is the "removed from list" mark: a later download to this path must show.
      useListVisibilityStore().forget(paths);
    }
    for (const path of paths) forgetLibraryPath(id, path);
    for (const itemId of Object.keys(group.items)) {
      historyIds.delete(`${id}:${itemId}`);
      stateStore.removeState(itemId);
      progressStore.deleteProgress(itemId);
      sizeStore.removeSizes(itemId);
      diagnosticsStore.removeDiagnostics(itemId);
    }
    diagnosticsStore.removeDiagnostics(group.id);
    destinationStore.deleteDestinations(group.id);
    optionsStore.removeOptions(group.id);
    const cancellation = groupStore.cancelGroup(group.id, true).catch(console.error);
    groupStore.deleteGroup(group.id);
    return cancellation;
  }

  function deleteAllGroups(removeHistory = true) {
    for (const groupId of [...groupStore.groupOrder]) {
      deleteGroup(groupId, removeHistory);
    }
  }

  function deleteGroupsByState(states: MediaState[]) {
    for (const groupId of [...groupStore.groupOrder]) {
      const state = stateStore.getGroupState(groupId);
      if (state && states.includes(state)) {
        deleteGroup(groupId);
      }
    }
  }

  return {
    loadLibrary,
    processMediaDestinationPayload,
    processMediaAddPayload,
    finalizePlaylistGroup,
    dispatchMediaInfoFetch,
    createCaptureGroup,
    hasRecentCapture,
    fetchMediaInfoForGroup,
    retryGroup,
    expandPlaylistGroup,
    downloadPlaylistSelection,
    rejectPendingReadyGroup,
    addAndDownload,
    addUrlBatch,
    addUrlBatchAndDownload,
    downloadPreparedGroup,
    prepareCaptureGroup,
    downloadGroup,
    downloadAllGroups,
    pauseAllGroups,
    pauseGroup,
    resumeAllGroups,
    deleteGroup,
    cancelDownload,
    deleteAllGroups,
    deleteGroupsByState,
    rememberFailedGroup,
  };
});
