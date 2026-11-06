import { flushPromises, mount } from '@vue/test-utils';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { defineComponent, ref } from 'vue';
import { createMemoryHistory, createRouter } from 'vue-router';
import { i18n } from '../../src/i18n';
import { installTauriMock } from '../utils/tauriMock';
import { useDownloadActions } from '../../src/composables/useDownloadActions';
import { useLinkIntake } from '../../src/composables/useLinkIntake';
import { useMediaStore } from '../../src/stores/media/media';
import { useMediaGroupStore } from '../../src/stores/media/group';
import { MediaState, useMediaStateStore } from '../../src/stores/media/state';
import { useReportStore } from '../../src/stores/report';
import { useSettingsStore } from '../../src/stores/settings';
import SettingsApp from '../../src/components/settings/SettingsApp.vue';
import { TrackType } from '../../src/tauri/types/media';
import type { Group } from '../../src/tauri/types/group';
import type { Settings } from '../../src/tauri/types/config';

/** Every UI usage key fires from its action (Android's clip.accepted never arrived); Rust owns the allow-list. */
function recordIpc() {
  const counted: string[] = [];
  const downloads: Record<string, unknown>[] = [];
  const patches: unknown[] = [];
  installTauriMock({
    usage_count: (_cmd, args) => {
      counted.push((args as { key: string }).key);
      return null;
    },
    media_download: (_cmd, args) => {
      downloads.push(args as Record<string, unknown>);
      return 'g';
    },
    media_info: (_cmd, args) => (args as { groupId: string }).groupId,
    config_set: (_cmd, args) => {
      patches.push((args as { patch: unknown }).patch);
      return useSettingsStore().settings;
    },
  });
  return { counted, downloads, patches };
}

function group(id: string, items = 1): Group {
  const entries: Record<string, Record<string, unknown>> = Object.fromEntries(Array.from({ length: items }, (_, i) => [`${id}-${i}`, {
    id: `${id}-${i}`, isLeader: items === 1, url: `https://example.com/${i}`, audioCodecs: [], formats: [], filesize: 0,
  }]));
  // A list's leader holds its entries and is never downloaded itself.
  if (items > 1) entries[id] = { id, isLeader: true, entries: [], url: 'https://example.com/list', audioCodecs: [], formats: [], filesize: 0 };
  return { id, url: 'https://example.com/v', total: items, processed: 0, errored: 0, isCombined: false, audioCodecs: [], formats: [], filesize: 0, items: entries } as unknown as Group;
}

const options = { trackType: TrackType.both } as never;

function harness<T>(use: () => T) {
  let out!: T;
  const router = createRouter({ history: createMemoryHistory(), routes: [{ path: '/', name: 'home', component: { template: '<div />' } }] });
  const wrapper = mount(defineComponent({
    setup() {
      out = use();
      return () => null;
    },
  }), { global: { plugins: [i18n, router] } });
  return { out, wrapper };
}

describe('usage counts from the UI', () => {
  afterEach(() => vi.restoreAllMocks());

  it('counts pause, pause all, retry, report opened and a pasted link', async () => {
    const ipc = recordIpc();
    useMediaGroupStore().createGroup(group('g'));
    vi.spyOn(useMediaStore(), 'pauseGroup').mockResolvedValue(true as never);
    vi.spyOn(useMediaStore(), 'retryGroup').mockResolvedValue(undefined);
    const { out: actions, wrapper } = harness(() => useDownloadActions(ref(useMediaGroupStore().findGroupById('g'))));
    const { out: intake, wrapper: intakeWrapper } = harness(() => useLinkIntake());
    try {
      await actions.pause();
      await actions.retry();
      useMediaStore().pauseAllGroups();
      useReportStore().openReport();
      intake.url.value = 'https://www.youtube.com/watch?v=abc';
      intake.submit();
      await flushPromises();
      expect(ipc.counted).toEqual(['pause', 'retry', 'pause_all', 'report.opened', 'paste']);
    } finally {
      wrapper.unmount();
      intakeWrapper.unmount();
    }
  });

  it('tells Rust whether a download is a tap, a batch or a resume', async () => {
    const ipc = recordIpc();
    const groups = useMediaGroupStore();
    const states = useMediaStateStore();
    const media = useMediaStore();
    groups.createGroup(group('one'));
    groups.createGroup(group('list', 3));
    groups.createGroup(group('paused'));
    states.setGroupState('paused', MediaState.paused);

    await media.downloadGroup('one', options);
    await media.downloadGroup('list', options, 'batch');
    await media.downloadGroup('paused', options);

    expect(ipc.downloads.map(d => [d.groupId, d.origin])).toEqual([
      ['one', 'tap'],
      ['list', 'batch'],
      ['paused', 'resume'],
    ]);
  });

  it('counts funnel.batch once for one Download on several links', async () => {
    const ipc = recordIpc();
    const groups = useMediaGroupStore();
    const states = useMediaStateStore();
    for (const id of ['a', 'b', 'c']) {
      groups.createGroup(group(id));
      states.setGroupState(id, MediaState.configure);
    }
    await useMediaStore().downloadAllGroups();
    await flushPromises();
    expect(ipc.counted).toEqual(['funnel.batch']);
    expect(ipc.downloads.map(d => d.origin)).toEqual(['batch', 'batch', 'batch']);
  });

  it('crash reports are on by default and the switch saves off at once', async () => {
    const ipc = recordIpc();
    const settings = ref<Settings>(JSON.parse(JSON.stringify(useSettingsStore().settings)) as Settings);
    expect(settings.value.privacy.sendReports).toBe(true);
    const wrapper = mount(SettingsApp, {
      props: { 'modelValue': settings.value, 'onUpdate:modelValue': (v: Settings) => (settings.value = v) },
      global: { plugins: [i18n] },
    });
    try {
      await wrapper.find('[data-testid="settings-send-reports"]').trigger('click');
      await flushPromises();
      expect(settings.value.privacy.sendReports).toBe(false);
      expect(ipc.patches).toContainEqual({ privacy: { sendReports: false } });
    } finally {
      wrapper.unmount();
    }
  });

  it('Help improve Nezzel is on by default and saves off at once', async () => {
    const ipc = recordIpc();
    const settings = ref<Settings>(JSON.parse(JSON.stringify(useSettingsStore().settings)) as Settings);
    expect(settings.value.privacy.sendUsage).toBe(true);
    const wrapper = mount(SettingsApp, {
      props: { 'modelValue': settings.value, 'onUpdate:modelValue': (v: Settings) => (settings.value = v) },
      global: { plugins: [i18n] },
    });
    try {
      const row = wrapper.find('[data-testid="settings-send-usage"]');
      expect(row.exists()).toBe(true);
      expect(wrapper.text()).toContain('Help improve Nezzel');
      await row.trigger('click');
      await flushPromises();
      expect(settings.value.privacy.sendUsage).toBe(false);
      expect(ipc.patches).toContainEqual({ privacy: { sendUsage: false } });
    } finally {
      wrapper.unmount();
    }
  });
});
