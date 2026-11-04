<script setup lang="ts">
// Hover/focus label. Used where the visible label is hidden (collapsed sidebar, icon buttons).
import type { FunctionalComponent } from 'vue';
import { injectTooltipProviderContext, TooltipArrow, TooltipContent, TooltipPortal, TooltipProvider, TooltipRoot, TooltipTrigger } from 'reka-ui';

withDefaults(defineProps<{ content: string; side?: 'top' | 'right' | 'bottom' | 'left'; disabled?: boolean }>(), { side: 'bottom' });
/** The app shell provides one TooltipProvider; outside it (own windows, isolated mounts) bring our own. Neither renders DOM. */
const Passthrough: FunctionalComponent = (_, { slots }) => slots.default?.();
const Provider = injectTooltipProviderContext(null) ? Passthrough : TooltipProvider;
</script>

<template>
  <component :is="Provider">
    <TooltipRoot :disabled="disabled">
      <TooltipTrigger as-child><slot /></TooltipTrigger>
      <TooltipPortal>
        <TooltipContent :side="side" :side-offset="6" class="pointer-events-none z-[90] rounded-md bg-foreground px-2 py-1 text-[12px] text-background shadow-lg animate-in fade-in-0 zoom-in-95">
          {{ content }}
          <TooltipArrow class="fill-foreground" :width="8" :height="4" />
        </TooltipContent>
      </TooltipPortal>
    </TooltipRoot>
  </component>
</template>
