import { describe, expect, it } from 'vitest';
import { defineComponent, h, nextTick, ref } from 'vue';
import { flushPromises, mount } from '@vue/test-utils';
import { SelectField } from '@/components/ui/select';

const options = [
  { value: '', label: 'Same as Settings' },
  { value: 'mp4', label: 'MP4' },
  { value: 'mkv', label: 'MKV', disabled: true },
];

function mountField(initial: string | undefined, opts = options) {
  const model = ref<string | number | null | undefined>(initial);
  const Host = defineComponent(() => () => h('div', [
    h('label', { for: 'fmt' }, 'Format'),
    h(SelectField, {
      'id': 'fmt',
      'options': opts,
      'placeholder': 'Pick one',
      'modelValue': model.value,
      'onUpdate:modelValue': (v: string | number | null | undefined) => { model.value = v; },
    }),
  ]));
  const wrapper = mount(Host, { attachTo: document.body });
  return { wrapper, model };
}

async function open(wrapper: ReturnType<typeof mount>) {
  await wrapper.find('#fmt').trigger('keydown', { key: 'ArrowDown' });
  await flushPromises();
  await nextTick();
}

const items = () => Array.from(document.body.querySelectorAll('[role="option"]')) as HTMLElement[];

describe('SelectField', () => {
  it('names the trigger from its <label for> and shows the selected label', async () => {
    const { wrapper } = mountField('mp4');
    await flushPromises();
    const trigger = wrapper.find('#fmt');
    expect(trigger.attributes('role')).toBe('combobox');
    expect(document.querySelector('label[for="fmt"]')).not.toBeNull();
    expect(trigger.text()).toContain('MP4');
    wrapper.unmount();
  });

  it('round-trips an empty-string option, which reka items cannot hold', async () => {
    const { wrapper, model } = mountField('');
    await flushPromises();
    expect(wrapper.find('#fmt').text()).toContain('Same as Settings');
    await open(wrapper);
    items().find(el => el.textContent?.includes('MP4'))!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
    await flushPromises();
    expect(model.value).toBe('mp4');

    await open(wrapper);
    items().find(el => el.textContent?.includes('Same as Settings'))!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
    await flushPromises();
    expect(model.value).toBe('');
    wrapper.unmount();
  });

  it('shows the placeholder for an empty model when no empty option exists', async () => {
    const { wrapper } = mountField('', options.slice(1));
    await flushPromises();
    expect(wrapper.find('#fmt').text()).toContain('Pick one');
    wrapper.unmount();
  });

  it('marks disabled options as not selectable', async () => {
    const { wrapper } = mountField('mp4');
    await open(wrapper);
    const mkv = items().find(el => el.textContent?.includes('MKV'))!;
    expect(mkv.getAttribute('aria-disabled')).toBe('true');
    wrapper.unmount();
  });

  it('updates the selected value when translated labels change without remounting', async () => {
    const translated = ref([{ value: 'best', label: 'Best quality' }]);
    const host = mount(defineComponent(() => () => h(SelectField, { id: 'translated', options: translated.value, modelValue: 'best' })));
    try {
      await flushPromises();
      expect(host.find('#translated').text()).toContain('Best quality');
      for (const label of ['أفضل جودة', 'Meilleure qualité']) {
        translated.value = [{ value: 'best', label }];
        await flushPromises();
        expect(host.find('#translated').text()).toContain(label);
      }
    } finally { host.unmount(); }
  });
});
