<template>
  <div class="pointer-events-none fixed bottom-11 end-5 z-70" aria-live="polite">
    <TransitionGroup name="nz-toast" tag="div" class="flex flex-col items-end gap-2">
      <div
        v-for="toast in toasts"
        :key="toast.id"
        role="status"
        class="pointer-events-auto flex max-w-sm cursor-pointer items-center gap-2.5 rounded-xl border bg-card px-3.5 py-3 text-start text-nz-meta text-foreground shadow-lg"
        :class="toast.style === 'error' ? 'border-destructive-border' : 'border-hairline'"
        @click="removeToast(toast.id)"
      >
        <component :is="iconFor(toast.style)" class="size-4.5 shrink-0" :class="iconClass(toast.style)" aria-hidden="true" />
        <span class="min-w-0 flex-auto [overflow-wrap:anywhere]">{{ toast.message }}</span>
        <button
          v-if="toast.action"
          type="button"
          class="-my-1 shrink-0 rounded-md px-2 py-1 font-semibold text-primary outline-none hover:bg-primary-soft focus-visible:ring-2 focus-visible:ring-ring/40"
          @click.stop="runAction(toast)"
        >
          {{ toast.action.label }}
        </button>
      </div>
    </TransitionGroup>
  </div>
</template>

<script setup lang="ts">
import { CircleCheck, CircleX, Info, TriangleAlert } from '@lucide/vue';
import { Toast, ToastStyle, useToastStore } from '../stores/toast';

const { toasts, removeToast } = useToastStore();

function runAction(toast: Toast) {
  toast.action?.run();
  removeToast(toast.id);
}

const iconFor = (style: ToastStyle) => {
  switch (style) {
    case 'success': return CircleCheck;
    case 'error': return CircleX;
    case 'warning': return TriangleAlert;
    default: return Info;
  }
};

const iconClass = (style: ToastStyle) => {
  switch (style) {
    case 'success': return 'text-primary';
    case 'error': return 'text-destructive';
    case 'warning': return 'text-amber-foreground';
    default: return 'text-muted-foreground';
  }
};
</script>

<style scoped>
.nz-toast-enter-active, .nz-toast-leave-active { transition: opacity var(--nz-dur) var(--nz-ease), transform var(--nz-dur) var(--nz-ease); }
.nz-toast-enter-from { opacity: 0; transform: translateY(0.5rem); }
.nz-toast-leave-to { opacity: 0; }
</style>
