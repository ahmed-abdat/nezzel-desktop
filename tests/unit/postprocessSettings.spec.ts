import { mount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import { nextTick } from 'vue';
import SettingsOutput from '../../src/components/settings/SettingsOutput.vue';
import { i18n } from '../../src/i18n';
import { defaultSettings } from '../../src/tauri/types/config';
import {
  AudioPostprocessPreset,
  TranscodePolicy,
  VideoContainer,
  VideoPostprocessMode,
  VideoPostprocessPreset,
} from '../../src/tauri/types/media';
import { pickOption as pick } from '../utils/selectField';

const SLOW_WARNING = i18n.global.t('prefs.adv.files.slowWarning');
const MP42_HINT = i18n.global.t('settings.output.postprocess.mp42RequiresMp4');
const KEEP_CONFLICT = i18n.global.t('prefs.adv.files.keepConflict');

function mountOutput() {
  const settings = structuredClone(defaultSettings);
  const wrapper = mount(SettingsOutput, {
    props: { modelValue: settings },
    global: { plugins: [i18n], stubs: { RouterLink: true } },
  });
  return { settings, wrapper };
}

describe('post-processing settings', () => {
  it('shows the slow warning for fps30 without rendering custom args', async () => {
    const { settings, wrapper } = mountOutput();
    await pick(wrapper, 'out-video-after', VideoPostprocessPreset.fps30);
    await nextTick();

    expect(settings.output.video.postprocessPreset).toBe(VideoPostprocessPreset.fps30);
    expect(wrapper.find('#out-video-args').exists()).toBe(false);
    expect(wrapper.text()).toContain(SLOW_WARNING);
  });

  it('shows and binds video ffmpeg args only for the custom preset', async () => {
    const { settings, wrapper } = mountOutput();
    expect(wrapper.find('#out-video-args').exists()).toBe(false);

    await pick(wrapper, 'out-video-after', VideoPostprocessPreset.custom);
    await nextTick();
    await wrapper.get('#out-video-args').setValue('-movflags +faststart');

    expect(settings.output.video.postprocessPreset).toBe(VideoPostprocessPreset.custom);
    expect(settings.output.video.postprocessArgs).toBe('-movflags +faststart');
  });

  it('maps the keep-as-sent switches to the output policies', async () => {
    const { settings, wrapper } = mountOutput();

    await wrapper.get('#out-keep-video').trigger('click');
    expect(settings.output.video.policy).toBe(TranscodePolicy.never);
    await wrapper.get('#out-keep-video').trigger('click');
    expect(settings.output.video.policy).toBe(TranscodePolicy.allowReencode);

    await wrapper.get('#out-keep-audio').trigger('click');
    expect(settings.output.audio.policy).toBe(TranscodePolicy.never);
    await wrapper.get('#out-keep-audio').trigger('click');
    expect(settings.output.audio.policy).toBe(TranscodePolicy.allowReencode);
  });

  it('shows the MP4-only alert when mp42 is selected without mp4 output', async () => {
    const { wrapper } = mountOutput();
    await pick(wrapper, 'out-video-format', VideoContainer.mkv);
    await pick(wrapper, 'out-video-after', VideoPostprocessPreset.mp42);
    await nextTick();

    expect(wrapper.text()).toContain(MP42_HINT);
  });

  it('shows the keep-as-sent conflict for custom re-encode mode immediately', async () => {
    const { wrapper } = mountOutput();
    await pick(wrapper, 'out-video-after', VideoPostprocessPreset.custom);
    await nextTick();
    await wrapper.get('#out-keep-video').trigger('click');
    await pick(wrapper, 'out-video-mode', VideoPostprocessMode.reencode);
    await nextTick();

    expect(wrapper.text()).toContain(KEEP_CONFLICT);
  });

  it('shows and binds audio ffmpeg args only for the custom preset', async () => {
    const { settings, wrapper } = mountOutput();
    expect(wrapper.find('#out-audio-args').exists()).toBe(false);

    await pick(wrapper, 'out-audio-after', AudioPostprocessPreset.custom);
    await nextTick();
    await wrapper.get('#out-audio-args').setValue('-metadata artist=Example');

    expect(settings.output.audio.postprocessPreset).toBe(AudioPostprocessPreset.custom);
    expect(settings.output.audio.postprocessArgs).toBe('-metadata artist=Example');
  });
});
