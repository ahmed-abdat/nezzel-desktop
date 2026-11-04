<script setup lang="ts">
// One option. Check mark at the inline start (macOS/GNOME menus), teal highlight like the kit's menus.
import type { HTMLAttributes } from 'vue';
import { SelectItem, SelectItemIndicator, SelectItemText, type SelectItemProps } from 'reka-ui';
import { Check } from '@lucide/vue';
import { cn } from 'cn';

const props = defineProps<SelectItemProps & { class?: HTMLAttributes['class'] }>();
</script>

<template>
  <SelectItem
    data-slot="select-item"
    :value="props.value"
    :disabled="props.disabled"
    :text-value="props.textValue"
    :class="cn(
      'relative flex w-full cursor-default select-none items-center rounded-[5px] py-1 ps-6 pe-2.5 text-nz-meta outline-none',
      'data-[highlighted]:bg-primary data-[highlighted]:text-primary-foreground data-[disabled]:pointer-events-none data-[disabled]:opacity-45',
      props.class,
    )"
  >
    <span class="absolute start-1.5 flex size-3.5 items-center justify-center">
      <SelectItemIndicator>
        <Check class="size-3.5" aria-hidden="true" />
      </SelectItemIndicator>
    </span>
    <SelectItemText>
      <slot />
    </SelectItemText>
  </SelectItem>
</template>
