<template>
  <!-- Legacy API (daisy class names on `class`) mapped onto the kit's Button look, so old screens match the new ones. -->
  <tooltip v-if="to" :content="tooltip ?? ''" :disabled="!tooltip">
    <router-link
      :to="to"
      v-bind="passthrough"
      :class="classes"
      :aria-disabled="disabled || undefined"
      :tabindex="disabled ? -1 : undefined"
    >
      <Loader2 v-if="loading" class="animate-spin" aria-hidden="true" />
      <span v-if="loading" class="sr-only">{{ t('common.loading') }}</span>
      <slot v-else />
    </router-link>
  </tooltip>
  <tooltip v-else :content="tooltip ?? ''" :disabled="!tooltip">
    <button
      v-bind="passthrough"
      :class="classes"
      :disabled="disabled || loading ? true : undefined"
      :aria-busy="loading || undefined"
      @click="emit('click', $event)"
    >
      <Loader2 v-if="loading" class="animate-spin" aria-hidden="true" />
      <span v-if="loading" class="sr-only">{{ t('common.loading') }}</span>
      <slot v-else />
    </button>
  </tooltip>
</template>
<script setup lang="ts">
import Tooltip from '../ui/tooltip/Tooltip.vue';
import { computed, PropType, useAttrs } from 'vue';
import { RouteLocationRaw } from 'vue-router';
import { useI18n } from 'vue-i18n';
import { Loader2 } from '@lucide/vue';
import { cn } from 'cn';
import { buttonVariants, type ButtonVariants } from '../ui/button';

const { t } = useI18n();

defineOptions({
  inheritAttrs: false,
});

const { to, disabled } = defineProps({
  loading: {
    type: Boolean,
    required: false,
  },
  tooltip: {
    type: String,
    required: false,
  },
  // Kept for API compatibility; the native title tooltip has no placement.
  tooltipLocation: {
    type: String,
    required: false,
    default: 'top',
  },
  to: {
    type: [String, Object] as PropType<RouteLocationRaw>,
    required: false,
  },
  disabled: {
    type: Boolean,
    required: false,
    default: false,
  },
});

const emit = defineEmits<{
  (e: 'click', event: MouseEvent): void;
}>();

const attrs = useAttrs();
const legacyClass = computed(() => {
  const value = attrs.class;
  if (typeof value === 'string') return value;
  if (Array.isArray(value)) return value.filter(v => typeof v === 'string').join(' ');
  if (value && typeof value === 'object') return Object.entries(value).filter(([, on]) => on).map(([name]) => name).join(' ');
  return '';
});
const passthrough = computed(() => {
  return Object.fromEntries(Object.entries(attrs).filter(([key]) => key !== 'class'));
});

const variant = computed<ButtonVariants['variant']>(() => {
  const c = legacyClass.value;
  if (c.includes('btn-primary')) return 'default';
  if (c.includes('btn-warning')) return 'amber';
  if (c.includes('btn-error')) return 'destructive';
  if (c.includes('btn-soft')) return 'secondary';
  return 'outline';
});
const size = computed<ButtonVariants['size']>(() => (legacyClass.value.includes('btn-sm') ? 'xs' : 'sm'));
/** Daisy names are dropped; layout utilities (ms-auto, w-full, ...) pass through. */
const extra = computed(() => legacyClass.value.split(/\s+/).filter(name => name && !name.startsWith('btn')).join(' '));
const classes = computed(() => cn(buttonVariants({ variant: variant.value, size: size.value }), disabled && to && 'pointer-events-none opacity-50', extra.value));
</script>
