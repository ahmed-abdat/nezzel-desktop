import { mount, flushPromises, type VueWrapper } from '@vue/test-utils';
import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';
import PlaylistSelectionStep from '../../src/components/media-card/steps/PlaylistSelectionStep.vue';
import { i18n, applyDocumentLocale, localeDirection } from '../../src/i18n';
import { ConfigProvider } from 'reka-ui';
import { defineComponent, h, reactive } from 'vue';
import { useMediaOptionsStore } from '../../src/stores/media/options';
import { TrackType } from '../../src/tauri/types/media';
import { SelectField } from '../../src/components/ui/select';
import type { Group } from '../../src/tauri/types/group';

const expandPlaylistGroup = vi.fn();
const showToast = vi.fn();
vi.mock('../../src/stores/media/media.ts', () => ({ useMediaStore: () => ({ downloadPlaylistSelection: expandPlaylistGroup }) }));
vi.mock('../../src/stores/toast.ts', () => ({ useToastStore: () => ({ showToast }) }));
let wrapper: ReturnType<typeof mount>;
afterEach(() => wrapper?.unmount());
function createGroup(): Group {
  return {
    id: 'group-1',
    url: 'https://example.com/playlist',
    title: 'Playlist',
    total: 2,
    processed: 0,
    errored: 0,
    isCombined: false,
    audioCodecs: [],
    formats: [],
    filesize: 0,
    entries: [
      { index: 0, title: 'First video', duration: 65, thumbnail: 'https://example.com/one.jpg', videoUrl: 'https://example.com/playlist?v=1' },
      { index: 1, videoUrl: 'https://example.com/playlist?v=2' },
    ],
    items: {
      leader: {
        id: 'leader',
        isLeader: true,
        url: 'https://example.com/playlist',
        audioCodecs: [],
        formats: [],
        filesize: 0,
        entries: [],
      },
    },
  };
}

describe('PlaylistSelectionStep', () => {
  beforeEach(() => {
    expandPlaylistGroup.mockReset();
    showToast.mockReset();
  });
  async function render(group = createGroup()) {
    applyDocumentLocale(i18n.global.locale.value);
    wrapper = mount(defineComponent({
      setup: () => () => h(ConfigProvider, { dir: localeDirection(i18n.global.locale.value) }, () => h(PlaylistSelectionStep, { group })),
    }), { global: { plugins: [i18n] } });
    await flushPromises();
    return wrapper;
  }
  function button(label: string) {
    return wrapper.findAll('button').find(button => button.text() === label)!;
  }
  it('shows flat metadata and fallback labels, applies exactly the checked subset', async () => {
    await render();
    expect(wrapper.text()).toContain('First video');
    expect(wrapper.text()).toContain('01:05');
    expect(wrapper.text()).toContain('Item 2');
    expect(wrapper.get('img').attributes('src')).toBe('https://example.com/one.jpg');
    await wrapper.get('#playlist-selection-group-1-item-1').trigger('click');
    await button('Download 1 video').trigger('click');
    expect(expandPlaylistGroup).toHaveBeenCalledExactlyOnceWith('group-1', { rows: [], positions: [0] });
  });
  it('selects none and all, preventing an empty expansion', async () => {
    await render();
    await button('Select none').trigger('click');
    expect(wrapper.findAll('[role=checkbox]').every(box => box.attributes('aria-checked') === 'false')).toBe(true);
    expect(button('Download 0 videos').attributes('disabled')).toBeDefined();
    expect(expandPlaylistGroup).not.toHaveBeenCalled();
    await button('Select all').trigger('click');
    expect(wrapper.findAll('[role=checkbox]').every(box => box.attributes('aria-checked') === 'true')).toBe(true);
    await button('Download 2 videos').trigger('click');
    expect(expandPlaylistGroup).toHaveBeenCalledWith('group-1', { rows: [], positions: [0, 1] });
  });
  it.each([
    { trackType: TrackType.both, height: 1080, fps: 60 },
    { trackType: TrackType.audio, abr: 128 },
  ])('preselects the captured quality while the checklist changes (%j)', async (options) => {
    useMediaOptionsStore().setOptions('group-1', options);
    await render();
    const field = wrapper.getComponent(SelectField as never) as unknown as VueWrapper;
    const props = field.props() as { modelValue: string; options: { value: string }[] };
    const selectedValue = props.modelValue;
    expect(props.options.some(option => option.value === selectedValue)).toBe(true);
    await wrapper.get('#playlist-selection-group-1-item-1').trigger('click');
    field.vm.$emit('update:modelValue', selectedValue);
    await flushPromises();
    expect(useMediaOptionsStore().getOptions('group-1')).toEqual(options);
  });
  it('offers every batch ceiling and applies the chosen quality', async () => {
    useMediaOptionsStore().setOptions('group-1', { trackType: TrackType.both, height: 720 });
    await render();
    const field = wrapper.getComponent(SelectField as never) as unknown as VueWrapper;
    expect((field.props() as { modelValue: string }).modelValue).toBe('video:720:0');
    expect((field.props() as { options: { value: string }[] }).options.map(option => option.value)).toEqual(['best', 'video:1080:0', 'video:720:0', 'video:480:0', 'audio']);
    for (const height of [1080, 720, 480]) {
      field.vm.$emit('update:modelValue', `video:${height}:0`);
      await flushPromises();
      expect(useMediaOptionsStore().getOptions('group-1')).toMatchObject({ trackType: TrackType.both, height });
    }
  });
  it('toggles a focused row with Space and selects all with Ctrl/Cmd+A', async () => {
    await render();
    const row = wrapper.get('[role=listitem]');
    await row.trigger('keydown', { key: ' ' });
    expect(wrapper.get('[role=checkbox]').attributes('aria-checked')).toBe('false');
    for (const modifier of ['ctrlKey', 'metaKey']) {
      await button('Select none').trigger('click');
      await row.trigger('keydown', { key: 'a', [modifier]: true });
      expect(wrapper.findAll('[role=checkbox]').every(box => box.attributes('aria-checked') === 'true')).toBe(true);
    }
  });
  it('shows selected bytes only when every checked item has a known size', async () => {
    const group = createGroup();
    Object.assign(group.entries![0], { filesize: 1000000 });
    await render(group);
    expect(wrapper.find('[data-testid=playlist-size]').exists()).toBe(false);
    await wrapper.get('#playlist-selection-group-1-item-1').trigger('click');
    expect(wrapper.get('[data-testid=playlist-size]').text()).toContain('1 MB');
  });
  it('keeps checks across equivalent rereads and resets for a different entry list', async () => {
    const group = reactive(createGroup());
    await render(group);
    await wrapper.get('#playlist-selection-group-1-item-1').trigger('click');
    group.entries = group.entries!.map(entry => ({ ...entry, title: 'Updated metadata' }));
    await flushPromises();
    expect(wrapper.get('#playlist-selection-group-1-item-1').attributes('aria-checked')).toBe('false');
    expect(wrapper.text()).toContain('Updated metadata');
    group.entries = group.entries!.map((entry, position) => ({ ...entry, videoUrl: `https://example.com/new-${position}` }));
    await flushPromises();
    expect(wrapper.get('#playlist-selection-group-1-item-1').attributes('aria-checked')).toBe('true');
  });
  it('shares checks between views and keeps them after a view remounts', async () => {
    const group = createGroup();
    wrapper = mount(defineComponent({
      setup: () => () => h('div', [
        h(PlaylistSelectionStep, { group, idPrefix: 'row' }),
        h(PlaylistSelectionStep, { group, idPrefix: 'sheet', showAction: false }),
      ]),
    }), { global: { plugins: [i18n] } });
    await flushPromises();
    await wrapper.get('#row-item-1').trigger('click');
    expect(wrapper.get('#sheet-item-1').attributes('aria-checked')).toBe('false');
    expect(wrapper.findAll('[role=status]').map(status => status.text())).toEqual(['1 of 2 item selected', '1 of 2 item selected']);
    expect(wrapper.findAll('button').filter(button => button.text() === 'Download 1 video')).toHaveLength(1);
    wrapper.unmount();
    await render(group);
    expect(wrapper.get('#playlist-selection-group-1-item-1').attributes('aria-checked')).toBe('false');
  });
  it('windows long lists and retains selections while scrolling', async () => {
    const group = createGroup();
    group.entries = Array.from({ length: 5000 }, (_, index) => ({ index, videoUrl: `https://example.com/${index}` }));
    await render(group);
    expect(wrapper.findAll('[role=checkbox]').length).toBeLessThan(30);
    const region = wrapper.get('[role=region]');
    Object.defineProperty(region.element, 'clientHeight', { value: 320 });
    region.element.scrollTop = 64000;
    await region.trigger('scroll');
    const box = wrapper.get('[role=checkbox]');
    expect(Number(box.attributes('id')!.split('-').at(-1))).toBeGreaterThan(900);
    await box.trigger('click');
    await button('Download 4999 videos').trigger('click');
    const positions = expandPlaylistGroup.mock.calls[0][1].positions as number[];
    expect(positions).toHaveLength(4999);
    expect(positions).toContain(0);
  });
  it.each(['en', 'ar', 'fr'] as const)('renders a bidi-safe checklist in %s', async (locale) => {
    const previous = i18n.global.locale.value;
    i18n.global.locale.value = locale;
    try {
      await render();
      expect(document.documentElement.dir).toBe(locale === 'ar' ? 'rtl' : 'ltr');
      expect(wrapper.get('[role=combobox]').attributes('dir')).toBe(locale === 'ar' ? 'rtl' : 'ltr');
      expect(wrapper.get('label').classes()).toContain('text-start');
      expect(wrapper.get('bdi').attributes('dir')).toBe('auto');
      expect(wrapper.text()).not.toContain('inputFilters.');
      if (locale === 'ar') expect(wrapper.text()).toContain('عنصران من');
    } finally {
      i18n.global.locale.value = previous;
      applyDocumentLocale(previous);
    }
  });
  it('shows a translated error when expansion fails', async () => {
    expandPlaylistGroup.mockRejectedValue(new Error('failed'));
    const error = vi.spyOn(console, 'error').mockImplementation(() => {});
    await render();
    await button('Download 2 videos').trigger('click');
    await flushPromises();
    expect(showToast).toHaveBeenCalledWith('Couldn\'t apply that selection. Try again', { style: 'error' });
    error.mockRestore();
  });
});
