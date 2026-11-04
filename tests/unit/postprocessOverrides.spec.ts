import { mount } from '@vue/test-utils';
import { nextTick } from 'vue';
import { describe, expect, it } from 'vitest';
import TheOutputPreferences from '../../src/components/media-view/TheOutputPreferences.vue';
import { i18n } from '../../src/i18n';
import { useMediaGroupStore } from '../../src/stores/media/group';
import { useMediaOptionsStore } from '../../src/stores/media/options';
import { useSettingsStore } from '../../src/stores/settings';
import { defaultSettings } from '../../src/tauri/types/config';
import {
  AudioPostprocessPreset,
  TranscodePolicy,
  VideoPostprocessPreset,
} from '../../src/tauri/types/media';
import type { MediaItem } from '../../src/tauri/types/media';
import { pickOption } from '../utils/selectField';

function createGroupItem(id: string): MediaItem {
  return {
    id,
    url: `https://example.com/${id}`,
    audioCodecs: [],
    formats: [],
    filesize: 0,
    isLeader: true,
  };
}

describe('post-processing overrides', () => {
  it('stores per-group postprocess overrides and clears them when reset', async () => {
    const groupId = 'group-postprocess';
    const settingsStore = useSettingsStore();
    settingsStore.settings.output = structuredClone(defaultSettings.output);

    const groupStore = useMediaGroupStore();
    groupStore.createGroup({
      id: groupId,
      url: 'https://example.com',
      total: 1,
      processed: 1,
      errored: 0,
      isCombined: false,
      audioCodecs: [],
      formats: [],
      filesize: 0,
      items: {
        leader: createGroupItem('leader'),
      },
    });

    const optionsStore = useMediaOptionsStore();
    const wrapper = mount(TheOutputPreferences, {
      props: {
        groupId,
      },
      global: {
        plugins: [i18n],
        stubs: {
          RouterLink: true,
        },
      },
    });

    await nextTick();
    await pickOption(wrapper, 'override-video-after', VideoPostprocessPreset.custom);
    await nextTick();
    await wrapper.find('#override-video-args').setValue('-vf fps=30');
    await nextTick();

    expect(optionsStore.getOverrides(groupId)?.output?.video).toEqual({
      postprocessPreset: VideoPostprocessPreset.custom,
      postprocessArgs: '-vf fps=30',
    });

    await pickOption(wrapper, 'override-video-after', VideoPostprocessPreset.none);
    await nextTick();
    expect(wrapper.find('#override-video-args').exists()).toBe(false);

    expect(optionsStore.getOverrides(groupId)?.output).toBeUndefined();
  });

  it('stores per-group keep original streams overrides and clears them when reset', async () => {
    const groupId = 'group-policy';
    const settingsStore = useSettingsStore();
    settingsStore.settings.output = structuredClone(defaultSettings.output);

    const groupStore = useMediaGroupStore();
    groupStore.createGroup({
      id: groupId,
      url: 'https://example.com',
      total: 1,
      processed: 1,
      errored: 0,
      isCombined: false,
      audioCodecs: [],
      formats: [],
      filesize: 0,
      items: {
        leader: createGroupItem('leader'),
      },
    });

    const optionsStore = useMediaOptionsStore();
    const wrapper = mount(TheOutputPreferences, {
      props: {
        groupId,
      },
      global: {
        plugins: [i18n],
        stubs: {
          RouterLink: true,
        },
      },
    });

    await nextTick();
    await wrapper.get('#override-keep-video').trigger('click');
    await nextTick();

    expect(optionsStore.getOverrides(groupId)?.output?.video?.policy).toBe(TranscodePolicy.never);

    await wrapper.get('#override-keep-video').trigger('click');
    await nextTick();

    expect(optionsStore.getOverrides(groupId)?.output).toBeUndefined();
  });

  it('stores per-group audio postprocess overrides and clears them when reset', async () => {
    const groupId = 'group-audio-postprocess';
    const settingsStore = useSettingsStore();
    settingsStore.settings.output = structuredClone(defaultSettings.output);

    const groupStore = useMediaGroupStore();
    groupStore.createGroup({
      id: groupId,
      url: 'https://example.com',
      total: 1,
      processed: 1,
      errored: 0,
      isCombined: false,
      audioCodecs: [],
      formats: [],
      filesize: 0,
      items: {
        leader: createGroupItem('leader'),
      },
    });

    const optionsStore = useMediaOptionsStore();
    const wrapper = mount(TheOutputPreferences, {
      props: {
        groupId,
      },
      global: {
        plugins: [i18n],
        stubs: {
          RouterLink: true,
        },
      },
    });

    await nextTick();
    await pickOption(wrapper, 'override-audio-after', AudioPostprocessPreset.custom);
    await nextTick();
    await wrapper.get('#override-audio-args').setValue('-metadata artist=Example');
    await nextTick();

    expect(optionsStore.getOverrides(groupId)?.output?.audio).toEqual({
      postprocessPreset: AudioPostprocessPreset.custom,
      postprocessArgs: '-metadata artist=Example',
    });

    await pickOption(wrapper, 'override-audio-after', AudioPostprocessPreset.none);
    await nextTick();

    expect(wrapper.find('#override-audio-args').exists()).toBe(false);
    expect(optionsStore.getOverrides(groupId)?.output).toBeUndefined();
  });
});
