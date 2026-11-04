import { mount } from '@vue/test-utils';
import { defineComponent, ref } from 'vue';
import { describe, expect, it } from 'vitest';
import { Input } from '../../src/components/ui/input';
import { Textarea } from '../../src/components/ui/textarea';

// v-model modifiers must keep working on the kit fields that replaced native inputs (SettingsNetwork uses .trim).
describe('kit field v-model modifiers', () => {
  it.each([
    ['Input', Input, 'input'],
    ['Textarea', Textarea, 'textarea'],
  ] as const)('%s applies .trim', async (_name, Field, tag) => {
    const value = ref('');
    const wrapper = mount(defineComponent({
      components: { Field },
      setup: () => ({ value }),
      template: '<Field v-model.trim="value" />',
    }));
    await wrapper.get(tag).setValue('  socks5://127.0.0.1:1080  ');
    expect(value.value).toBe('socks5://127.0.0.1:1080');
  });

  it('Input applies .number', async () => {
    const value = ref<number | string | null>(null);
    const wrapper = mount(defineComponent({
      components: { Field: Input },
      setup: () => ({ value }),
      template: '<Field v-model.number="value" type="number" />',
    }));
    await wrapper.get('input').setValue('2.5');
    expect(value.value).toBe(2.5);
  });
});
