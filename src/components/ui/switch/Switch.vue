<script setup lang="ts">
import type { HTMLAttributes } from 'vue';
import { SwitchRoot, SwitchThumb, useForwardPropsEmits, type SwitchRootEmits, type SwitchRootProps } from 'reka-ui';
import { cn } from 'cn';

const props = defineProps<SwitchRootProps & { class?: HTMLAttributes['class'] }>();
const emits = defineEmits<SwitchRootEmits>();
const forwarded = useForwardPropsEmits(props, emits);
</script>

<template>
  <SwitchRoot
    v-bind="forwarded"
    data-slot="switch"
    :class="cn('peer inline-flex h-[15px] w-[26px] shrink-0 items-center rounded-full outline-none transition-colors duration-150 focus-visible:ring-2 focus-visible:ring-ring/40 disabled:cursor-not-allowed disabled:opacity-50 data-[state=checked]:bg-primary data-[state=unchecked]:bg-foreground/25 data-[state=unchecked]:ring-1 data-[state=unchecked]:ring-inset data-[state=unchecked]:ring-foreground/10', props.class)"
  >
    <SwitchThumb
      data-slot="switch-thumb"
      class="pointer-events-none block size-[13px] rounded-full bg-white shadow-[0_1px_2px_rgb(0_0_0/0.25)] transition-transform duration-150 data-[state=checked]:translate-x-[12px] data-[state=unchecked]:translate-x-px rtl:data-[state=checked]:-translate-x-[12px] rtl:data-[state=unchecked]:-translate-x-px"
    />
  </SwitchRoot>
</template>
