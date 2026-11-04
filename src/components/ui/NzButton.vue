<template>
  <component
    :is="to ? RouterLink : 'button'"
    :to="to"
    :type="to ? undefined : type"
    :disabled="to ? undefined : (disabled || loading)"
    :aria-disabled="disabled || loading ? 'true' : undefined"
    :aria-busy="loading ? 'true' : undefined"
    class="nz-btn"
    :class="[`nz-btn--${variant}`, `nz-btn--${size}`, { 'nz-btn--icon': icon }]"
  >
    <span v-if="loading" class="nz-btn__spinner" aria-hidden="true" />
    <slot />
  </component>
</template>

<script setup lang="ts">
import { RouterLink, type RouteLocationRaw } from 'vue-router';

withDefaults(defineProps<{
  variant?: 'primary' | 'quiet' | 'ghost' | 'danger';
  size?: 'sm' | 'md' | 'lg';
  type?: 'button' | 'submit' | 'reset';
  to?: RouteLocationRaw;
  icon?: boolean;
  disabled?: boolean;
  loading?: boolean;
}>(), {
  variant: 'quiet',
  size: 'md',
  type: 'button',
  to: undefined,
  icon: false,
  disabled: false,
  loading: false,
});
</script>

<style scoped>
.nz-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 0.4rem;
  flex-shrink: 0;
  border: 1px solid transparent;
  border-radius: var(--nz-r-control);
  font-weight: 600;
  line-height: 1;
  white-space: nowrap;
  cursor: pointer;
  user-select: none;
  text-decoration: none;
  transition: background-color var(--nz-dur-fast) var(--nz-ease), color var(--nz-dur-fast) var(--nz-ease), border-color var(--nz-dur-fast) var(--nz-ease), transform var(--nz-dur-fast) var(--nz-ease);
}
.nz-btn:active:not(:disabled) { transform: translateY(0.5px) scale(0.985); }
.nz-btn:disabled, .nz-btn[aria-disabled="true"] { cursor: not-allowed; opacity: 0.5; }

.nz-btn--sm { height: 1.875rem; padding-inline: 0.7rem; font-size: var(--nz-fs-meta); }
.nz-btn--md { height: 2.25rem; padding-inline: 0.95rem; font-size: var(--nz-fs-body); }
.nz-btn--lg { height: 2.75rem; padding-inline: 1.25rem; font-size: var(--nz-fs-body); }
.nz-btn--icon.nz-btn--sm { width: 1.875rem; padding: 0; }
.nz-btn--icon.nz-btn--md { width: 2.25rem; padding: 0; }
.nz-btn--icon.nz-btn--lg { width: 2.75rem; padding: 0; }

.nz-btn--primary { background: var(--nz-primary); color: var(--nz-on-primary); }
.nz-btn--primary:hover:not(:disabled) { background: var(--nz-primary-hover); }
.nz-btn--primary:disabled, .nz-btn--primary[aria-disabled="true"] { background: var(--nz-sunken); color: var(--nz-muted); border-color: var(--nz-line); opacity: 1; }
.nz-btn--primary[aria-busy="true"] { background: var(--nz-primary); color: var(--nz-on-primary); opacity: 0.8; }

.nz-btn--quiet { background: var(--nz-sunken); color: var(--nz-ink); border-color: var(--nz-line); }
.nz-btn--quiet:hover:not(:disabled) { background: var(--nz-line); }

.nz-btn--ghost { background: transparent; color: var(--nz-muted); }
.nz-btn--ghost:hover:not(:disabled) { background: var(--nz-sunken); color: var(--nz-ink); }

.nz-btn--danger { background: var(--nz-danger-soft); color: var(--nz-danger-ink); }
.nz-btn--danger:hover:not(:disabled) { background: color-mix(in oklab, var(--nz-danger) 18%, var(--nz-danger-soft)); }

.nz-btn :deep(svg) { width: 1.1em; height: 1.1em; flex-shrink: 0; }
.nz-btn--icon :deep(svg) { width: 1.15rem; height: 1.15rem; }

.nz-btn__spinner {
  width: 0.9em; height: 0.9em;
  border-radius: 999px;
  border: 2px solid currentColor;
  border-inline-end-color: transparent;
  animation: nz-spin 0.8s linear infinite;
}
@keyframes nz-spin { to { transform: rotate(360deg); } }
@media (prefers-reduced-motion: reduce) { .nz-btn__spinner { animation-duration: 2.4s; } }
</style>
