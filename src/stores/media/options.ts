import { defineStore } from 'pinia';
import { markRaw, ref, toRaw } from 'vue';
import { DownloadOptions, DownloadOverrides, EncodingOptions, TrackOptions } from '../../tauri/types/media';
import { useMediaGroupStore } from './group';
import { Group } from '../../tauri/types/group';

export const useMediaOptionsStore = defineStore('media-options', () => {
  const groupOptions = ref<Record<string, DownloadOptions>>({});
  const groupEncodings = ref<Record<string, EncodingOptions>>({});
  const groupTracks = ref<Record<string, TrackOptions>>({});
  const groupOverrides = ref<Record<string, DownloadOverrides>>({});
  const groupPlaylistSelections = ref<Record<string, { signature: string; positions: number[] }>>({});
  const groupAttachmentSelections = ref<Record<string, string[]>>({});
  const globalOptions = ref<DownloadOptions>();
  const globalEncodings = ref<EncodingOptions>();
  const globalTracks = ref<TrackOptions>();

  const groupStore = useMediaGroupStore();

  function setGroupValue<T>(target: { value: Record<string, T> }, groupId: string, value: T) {
    target.value[groupId] = value;
  }

  function getGroupValue<T>(target: { value: Record<string, T> }, groupId: string): T | undefined {
    return target.value[groupId];
  }

  function removeGroupValue<T>(target: { value: Record<string, T> }, groupId: string) {
    delete target.value[groupId];
  }

  function applyGlobalValue<T>(
    globalRef: { value: T | undefined },
    groupRef: { value: Record<string, T> },
    value: T,
  ) {
    globalRef.value = value;
    for (const group of Object.values(groupStore.groups)) {
      groupRef.value[group.id] = value;
    }
  }

  const setOptions = (groupId: string, options: DownloadOptions) => setGroupValue(groupOptions, groupId, options);
  const getOptions = (groupId: string): DownloadOptions | undefined => getGroupValue(groupOptions, groupId);
  const getGlobalOptions = (): DownloadOptions | undefined => globalOptions.value;

  function playlistSignature(group: Group): string {
    return JSON.stringify((group.entries ?? []).map(entry => [entry.index, entry.videoUrl]));
  }

  function getPlaylistSelection(group: Group): number[] {
    const stored = groupPlaylistSelections.value[group.id];
    return stored?.signature === playlistSignature(group)
      ? stored.positions
      : (group.entries ?? []).map((_, position) => position);
  }

  function setPlaylistSelection(group: Group, positions: number[]) {
    const wanted = new Set(positions);
    groupPlaylistSelections.value[group.id] = {
      signature: playlistSignature(group),
      positions: (group.entries ?? []).map((_, position) => position).filter(position => wanted.has(position)),
    };
  }

  function attachmentIds(groupId: string): string[] {
    return [...new Set((groupStore.findGroupById(groupId)?.attachments ?? []).map(attachment => attachment.id))];
  }

  function getAttachmentSelection(groupId: string): string[] {
    const selected = groupAttachmentSelections.value[groupId];
    const wanted = selected === undefined ? undefined : new Set(selected);
    return attachmentIds(groupId).filter(id => wanted === undefined || wanted.has(id));
  }

  function setAttachmentSelection(groupId: string, selected: string[]) {
    const wanted = new Set(selected);
    groupAttachmentSelections.value[groupId] = attachmentIds(groupId).filter(id => wanted.has(id));
  }

  const setEncodings = (groupId: string, encodings: EncodingOptions) => setGroupValue(groupEncodings, groupId, encodings);
  const getEncodings = (groupId: string): EncodingOptions | undefined => getGroupValue(groupEncodings, groupId);
  const getGlobalEncodings = (): EncodingOptions | undefined => globalEncodings.value;

  const setTracks = (groupId: string, tracks: TrackOptions) => setGroupValue(groupTracks, groupId, tracks);
  const getTracks = (groupId: string): TrackOptions | undefined => getGroupValue(groupTracks, groupId);
  const getGlobalTracks = (): TrackOptions | undefined => globalTracks.value;

  const setOverrides = (groupId: string, overrides: DownloadOverrides) => {
    const clonedOverrides = structuredClone(toRaw(overrides));
    setGroupValue(groupOverrides, groupId, markRaw(clonedOverrides));
  };
  const getOverrides = (groupId: string): DownloadOverrides | undefined => getGroupValue(groupOverrides, groupId);

  const removeOptions = (groupId: string) => {
    removeGroupValue(groupOptions, groupId);
    removeGroupValue(groupEncodings, groupId);
    removeGroupValue(groupTracks, groupId);
    removeGroupValue(groupOverrides, groupId);
    removeGroupValue(groupAttachmentSelections, groupId);
    removeGroupValue(groupPlaylistSelections, groupId);
  };

  const removeEncodings = (groupId: string) => removeGroupValue(groupEncodings, groupId);
  const removeTracks = (groupId: string) => removeGroupValue(groupTracks, groupId);
  const removeOverrides = (groupId: string) => removeGroupValue(groupOverrides, groupId);

  function applyOptionsToGroup(
    group: Group,
    options: DownloadOptions,
  ) {
    setOptions(group.id, options);
  }

  function applyEncodingsToGroup(
    group: Group,
    encodings: EncodingOptions,
  ) {
    setEncodings(group.id, encodings);
  }

  function applyTracksToGroup(
    group: Group,
    tracks: TrackOptions,
  ) {
    setTracks(group.id, tracks);
  }

  function applyGlobalOptions(
    options: DownloadOptions,
  ) {
    applyGlobalValue(globalOptions, groupOptions, options);
  }

  function applyGlobalEncodings(
    encodings: EncodingOptions,
  ) {
    applyGlobalValue(globalEncodings, groupEncodings, encodings);
  }

  function applyGlobalTracks(
    tracks: TrackOptions,
  ) {
    applyGlobalValue(globalTracks, groupTracks, tracks);
  }

  return {
    groupOptions,
    groupEncodings,
    groupTracks,
    groupOverrides,
    groupAttachmentSelections,
    groupPlaylistSelections,
    getPlaylistSelection,
    setPlaylistSelection,
    globalOptions,
    globalEncodings,
    globalTracks,
    setOptions,
    getOptions,
    getGlobalOptions,
    getAttachmentSelection,
    setAttachmentSelection,
    setEncodings,
    getEncodings,
    getGlobalEncodings,
    setTracks,
    getTracks,
    getGlobalTracks,
    setOverrides,
    getOverrides,
    removeOptions,
    removeEncodings,
    removeTracks,
    removeOverrides,
    applyGlobalOptions,
    applyGlobalEncodings,
    applyGlobalTracks,
    applyOptionsToGroup,
    applyEncodingsToGroup,
    applyTracksToGroup,
  };
});
