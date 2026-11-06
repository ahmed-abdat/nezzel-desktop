import { mount } from '@vue/test-utils';
import { defineComponent, ref } from 'vue';
import { createMemoryHistory, createRouter } from 'vue-router';
import { describe, expect, it } from 'vitest';
import { useDownloadRow } from '../../src/composables/useDownloadRow';
import { phaseOf } from '../../src/helpers/groupView';
import { useMediaDiagnosticsStore } from '../../src/stores/media/diagnostics';
import DownloadRow from '../../src/components/downloads/DownloadRow.vue';
import { i18n } from '../../src/i18n';
import { useMediaGroupStore } from '../../src/stores/media/group';
import { useMediaOptionsStore } from '../../src/stores/media/options';
import { MediaState, useMediaStateStore } from '../../src/stores/media/state';
import { TrackType, type MediaFormat } from '../../src/tauri/types/media';

const f = (id: string, height: number, fps: number): MediaFormat => ({ id, height, fps, audioCodecs: ['mp4a'], videoCodecs: ['h264'] });

describe('row quality label', () => {
  it.each([
    ['best', { height: 1080, fps: 60 }, '1080p60'],
    ['small', { height: 720, fps: 60 }, '720p60'],
    ['best', { height: 1080, fps: 30 }, '1080p'],
  ] as const)('a %s pick names the frame rate it saves (%j -> %s)', async (_choice, picked, label) => {
    const groups = useMediaGroupStore();
    const id = `q-${label}`;
    const formats = [f('v1080', 1080, picked.height === 1080 ? picked.fps : 30), f('v720', 720, picked.height === 720 ? picked.fps : 30)];
    groups.createGroup({ id, url: 'https://example.com/v', title: 'Clip', mediaKind: 'video', total: 1, processed: 1, errored: 0, isCombined: false,
      audioCodecs: [], formats, filesize: 0, items: { item: { id: `${id}-item`, isLeader: true, url: 'https://example.com/v', audioCodecs: [], formats: [], filesize: 0 } } });
    useMediaOptionsStore().setOptions(id, { trackType: TrackType.both, ...picked });
    useMediaStateStore().setGroupState(id, MediaState.done);
    const router = createRouter({ history: createMemoryHistory(), routes: [{ path: '/', component: { template: '<div />' } }] });
    await router.push('/');
    const row = mount(DownloadRow, { props: { group: groups.findGroupById(id)! }, global: { plugins: [router, i18n] } });
    try {
      expect(row.text()).toContain(label);
      if (label === '1080p') expect(row.text()).not.toContain('1080p30');
    } finally {
      row.unmount();
    }
  });
});

describe('row phase mapping', () => {
  it.each([
    [MediaState.fetching, 'reading'], [MediaState.fetchingList, 'reading'], [MediaState.playlistSelection, 'playlist'],
    [MediaState.configure, 'ready'], [MediaState.downloading, 'downloading'], [MediaState.downloadingList, 'downloading'],
    [MediaState.paused, 'paused'], [MediaState.pausedList, 'paused'], [MediaState.done, 'done'], [MediaState.error, 'error'],
  ] as const)('%s -> %s', (state, phase) => {
    expect(phaseOf(state)).toBe(phase);
  });
});

describe('row login hint', () => {
  function needsLogin(id: string, code: string | undefined, state: MediaState) {
    const groups = useMediaGroupStore();
    groups.createGroup({ id, url: 'https://example.com/v', title: 'Clip', mediaKind: 'video', total: 1, processed: 1, errored: 0, isCombined: false,
      audioCodecs: [], formats: [], filesize: 0, items: { item: { id: `${id}-item`, isLeader: true, url: 'https://example.com/v', audioCodecs: [], formats: [], filesize: 0 } } });
    useMediaStateStore().setGroupState(id, state);
    if (code) useMediaDiagnosticsStore().processMediaDiagnosticPayload({ id: `${id}-item`, groupId: id, level: 'error', code, component: null, message: '', raw: '', timestamp: 1 });
    const host = mount(defineComponent({ setup: () => useDownloadRow(ref(groups.findGroupById(id)!)), render: () => null }), { global: { plugins: [i18n] } });
    try {
      return (host.vm as unknown as { needsLogin: boolean }).needsLogin;
    } finally {
      host.unmount();
    }
  }

  it.each([
    ['signInRequired', MediaState.error, true],
    ['signInRequiredForBotDetection', MediaState.error, true],
    ['signInRequired', MediaState.done, false],
    ['geoBlocked', MediaState.error, false],
    [undefined, MediaState.error, false],
    // a login fixes these too: the row agrees with the capture card's "login" reason
    ['membersOnly', MediaState.error, true],
    ['videoPasswordRequired', MediaState.error, true],
  ] as const)('%s while %s -> %s', (code, state, expected) => {
    expect(needsLogin(`login-${code}-${state}`, code, state)).toBe(expected);
  });
});
