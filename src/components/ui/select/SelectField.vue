<script setup lang="ts" generic="T extends string | number">
// Options-driven select: the drop-in for a native <select> + <option v-for>.
// - `options` may contain an empty-string value (reka forbids it on items), mapped to a sentinel here.
// - Name it with `id` + an outside <label for>, or `aria-label` (VoiceOver/Narrator/Orca read the trigger as a combobox).
import type { HTMLAttributes } from 'vue';
import { computed } from 'vue';
import { SelectRoot, SelectValue } from 'reka-ui';
import SelectTrigger from './SelectTrigger.vue';
import SelectContent from './SelectContent.vue';
import SelectItem from './SelectItem.vue';

import type { SelectFieldOption, SelectFieldValue } from './types';

const EMPTY = '__nz_empty__';

defineOptions({ inheritAttrs: false });
const props = defineProps<{
  options: (SelectFieldOption & { value: T })[];
  id?: string;
  placeholder?: string;
  disabled?: boolean;
  name?: string;
  class?: HTMLAttributes['class'];
  contentClass?: HTMLAttributes['class'];
  valueDir?: 'ltr' | 'rtl';
}>();
const model = defineModel<T | null | undefined>();

const toInner = (v: SelectFieldValue | null | undefined) => (v === '' ? EMPTY : v ?? undefined);
// reka hands back the inner value; the sentinel turns back into ''.
// ESLint's parser does not read the SFC `generic` attribute, so it sees T as `any`; vue-tsc needs the cast.
// eslint-disable-next-line @typescript-eslint/no-unnecessary-type-assertion
const fromInner = (v: SelectFieldValue | undefined) => (v === EMPTY ? '' : v) as unknown as T | undefined;
const hasEmptyOption = computed(() => props.options.some(o => o.value === ''));
const inner = computed<SelectFieldValue | undefined>({
  // An empty model only selects the "" option when one exists; otherwise the placeholder shows.
  get: () => (model.value === '' && !hasEmptyOption.value ? undefined : toInner(model.value)),
  set: (v) => { model.value = fromInner(v); },
});
const items = computed(() => props.options.map(o => ({ ...o, inner: toInner(o.value) as SelectFieldValue })));
</script>

<template>
  <SelectRoot v-model="inner" :disabled="disabled" :name="name">
    <SelectTrigger :id="id" v-bind="$attrs" :class="props.class">
      <SelectValue v-if="$slots.value" :placeholder="placeholder" :dir="valueDir">
        <slot name="value" />
      </SelectValue>
      <SelectValue v-else :placeholder="placeholder" :dir="valueDir" />
    </SelectTrigger>
    <SelectContent :class="contentClass">
      <SelectItem v-for="option in items" :key="String(option.inner)" :value="option.inner" :disabled="option.disabled" :text-value="option.label">
        <slot name="option" :option="option">{{ option.label }}</slot>
      </SelectItem>
    </SelectContent>
  </SelectRoot>
</template>
