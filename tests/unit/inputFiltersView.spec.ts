import { mount } from '@vue/test-utils';
import { reactive, nextTick } from 'vue';
import { describe, expect, it, vi, beforeEach } from 'vitest';
import InputFiltersView from '../../src/views/app/InputFiltersView.vue';
import { DatePicker } from '../../src/components/ui/date-picker';
import { i18n } from '../../src/i18n';
import { createDefaultInputFilterSettings } from '../../src/helpers/inputFilters';
import { defaultSettings, type Settings } from '../../src/tauri/types/config';
import { pickOption } from '../utils/selectField';

const patch = vi.fn();
const showToast = vi.fn();
const settingsState = reactive<Settings>(structuredClone(defaultSettings));

vi.mock('../../src/stores/settings.ts', () => ({
  useSettingsStore: () => ({
    settings: settingsState,
    patch,
  }),
}));

vi.mock('../../src/stores/toast.ts', () => ({
  useToastStore: () => ({
    showToast,
  }),
}));

describe('InputFiltersView', () => {
  beforeEach(() => {
    patch.mockReset();
    showToast.mockReset();
    Object.assign(settingsState, structuredClone(defaultSettings));
    settingsState.inputFilters = createDefaultInputFilterSettings();
  });

  it('applies and clears date presets in the draft form', async () => {
    const wrapper = mount(InputFiltersView, {
      global: {
        plugins: [i18n],
        stubs: {
          RouterLink: true,
        },
      },
    });

    // The date field only exists while a date rule is on.
    expect(wrapper.find('#input-filters-date').exists()).toBe(false);

    await pickOption(wrapper, 'input-filters-date-preset', 'last7Days');
    await nextTick();

    expect(wrapper.findComponent(DatePicker).props('modelValue')).toMatch(/^\d{4}-\d{2}-\d{2}$/);

    await pickOption(wrapper, 'input-filters-date-mode', '');
    await nextTick();

    expect(wrapper.find('#input-filters-date').exists()).toBe(false);
  });

  it('normalizes values before saving and shows a success toast', async () => {
    patch.mockResolvedValue(undefined);

    const wrapper = mount(InputFiltersView, {
      global: {
        plugins: [i18n],
        stubs: {
          RouterLink: true,
        },
      },
    });

    await wrapper.get('#input-filters-min-size').setValue('50');
    await wrapper.get('#input-filters-match-filters').setValue('  !is_live  ');
    await pickOption(wrapper, 'input-filters-date-mode', 'exact');
    await nextTick();
    wrapper.findComponent(DatePicker).vm.$emit('update:modelValue', '2026-06-21');
    await nextTick();
    await nextTick();
    await wrapper.get('form').trigger('submit');

    expect(patch).toHaveBeenCalledWith({
      inputFilters: {
        minSize: { value: 50, unit: 'MB' },
        maxSize: { value: null, unit: 'MB' },
        dateFilter: { mode: 'exact', value: '2026-06-21' },
        matchFilters: '!is_live',
        breakMatchFilters: null,
      },
    });
    expect(showToast).toHaveBeenCalledWith('Link filters saved', { style: 'success' });
  });

  it('resets the draft back to defaults', async () => {
    const wrapper = mount(InputFiltersView, {
      global: {
        plugins: [i18n],
        stubs: {
          RouterLink: true,
        },
      },
    });

    await wrapper.get('#input-filters-min-size').setValue('25');
    await pickOption(wrapper, 'input-filters-date-preset', 'today');
    await nextTick();

    const resetButton = wrapper.findAll('button').find(button => button.text().includes('Reset'));
    expect(resetButton).toBeTruthy();

    await resetButton!.trigger('click');
    await nextTick();

    expect((wrapper.get('#input-filters-min-size').element as HTMLInputElement).value).toBe('');
    expect(wrapper.find('#input-filters-date').exists()).toBe(false);
  });

  it('shows an error toast when saving fails', async () => {
    patch.mockRejectedValue(new Error('boom'));
    const consoleError = vi.spyOn(console, 'error').mockImplementation(() => {});

    const wrapper = mount(InputFiltersView, {
      global: {
        plugins: [i18n],
        stubs: {
          RouterLink: true,
        },
      },
    });

    await wrapper.get('#input-filters-min-size').setValue('10');
    await wrapper.get('form').trigger('submit');
    await flushPromises();

    expect(showToast).toHaveBeenCalledWith('Couldn’t save link filters: Error: boom', {
      style: 'error',
    });

    consoleError.mockRestore();
  });
});

function flushPromises() {
  return new Promise(resolve => setTimeout(resolve, 0));
}
