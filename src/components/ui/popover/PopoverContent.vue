<script setup lang="ts">
// Popover surface: same skin as menus (popover token, hairline border, shadow-lg).
import type { HTMLAttributes } from 'vue';
import { computed } from 'vue';
import {
  PopoverContent,
  PopoverPortal,
  useForwardPropsEmits,
  type PopoverContentEmits,
  type PopoverContentProps,
} from 'reka-ui';
import { cn } from 'cn';

defineOptions({ inheritAttrs: false });
const props = withDefaults(
  defineProps<PopoverContentProps & { class?: HTMLAttributes['class'] }>(),
  { align: 'center', sideOffset: 4 },
);
const emits = defineEmits<PopoverContentEmits>();
const delegated = computed(() => {
  const rest = { ...props };
  delete rest.class;
  return rest;
});
const forwarded = useForwardPropsEmits(delegated, emits);
</script>

<template>
  <PopoverPortal>
    <PopoverContent
      data-slot="popover-content"
      v-bind="{ ...$attrs, ...forwarded }"
      :class="cn(
        'z-[80] w-72 max-w-(--reka-popover-content-available-width) origin-(--reka-popover-content-transform-origin) rounded-lg border border-hairline bg-popover p-3 text-foreground shadow-lg outline-hidden data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0 data-[state=closed]:zoom-out-95 data-[state=open]:zoom-in-95',
        props.class,
      )"
    >
      <slot />
    </PopoverContent>
  </PopoverPortal>
</template>
