import { nextTick } from 'vue';
import type { VueWrapper } from '@vue/test-utils';
import { SelectField } from '../../src/components/ui/select';

/** Picks `value` in the SelectField with this id, as a click on its option would. */
export async function pickOption(wrapper: VueWrapper, id: string, value: string | number): Promise<void> {
  const fields = wrapper.findAllComponents(SelectField as never) as unknown as VueWrapper[];
  const field = fields.find(component => (component.props() as { id?: string }).id === id);
  if (!field) throw new Error(`No SelectField with id "${id}"`);
  field.vm.$emit('update:modelValue', value);
  await nextTick();
}
