<script setup lang="ts">
// 32x18, knob 14: on = teal, knob at the inline end; off = neutral grey (no tint), knob at the start.
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
    :class="cn('peer inline-flex h-[18px] w-8 shrink-0 items-center rounded-full outline-none transition-colors duration-150 focus-visible:outline-solid focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--nz-focus)] disabled:cursor-not-allowed disabled:opacity-50 data-[state=checked]:bg-primary data-[state=unchecked]:bg-[oklch(0.86_0_0)] dark:data-[state=unchecked]:bg-[oklch(0.42_0_0)]', props.class)"
  >
    <SwitchThumb
      data-slot="switch-thumb"
      class="pointer-events-none block size-3.5 rounded-full bg-white shadow-[0_1px_2px_rgb(0_0_0/0.3),0_0_0_0.5px_rgb(0_0_0/0.06)] transition-transform duration-150 ease-[var(--nz-ease)] motion-reduce:transition-none data-[state=checked]:translate-x-4 data-[state=unchecked]:translate-x-0.5 rtl:data-[state=checked]:-translate-x-4 rtl:data-[state=unchecked]:-translate-x-0.5"
    />
  </SwitchRoot>
</template>
