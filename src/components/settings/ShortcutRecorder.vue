<template>
  <div class="flex flex-col items-end gap-1">
    <div class="flex items-center gap-1.5">
      <Check v-if="saved" class="size-3.5 text-primary" :aria-label="t('shortcut.saved')" />
      <button
        ref="control"
        type="button"
        class="inline-flex h-7 min-w-[7.5rem] items-center justify-center gap-1 rounded-[6px] border px-2 text-[12.5px] outline-none transition-colors"
        :class="readonly
          ? 'cursor-default border-transparent bg-transparent'
          : [
            'bg-card shadow-xs disabled:cursor-not-allowed disabled:opacity-50',
            recording ? 'border-primary ring-2 ring-primary/25' : 'border-input hover:border-foreground/25 focus-visible:ring-2 focus-visible:ring-ring/30',
          ]"
        :disabled="disabled || readonly"
        :aria-label="ariaLabel"
        :aria-describedby="hintId"
        data-testid="shortcut-control"
        @click="start"
        @keydown="onKey"
        @keyup="onKeyUp"
        @blur="stop"
      >
        <template v-if="recording">
          <span class="size-1.5 shrink-0 animate-pulse rounded-full bg-primary motion-reduce:animate-none" aria-hidden="true" />
          <span v-if="held.length" class="inline-flex items-center gap-0.5" dir="ltr" data-testid="shortcut-held">
            <Kbd v-for="cap in held" :key="cap" class="text-primary">{{ cap }}</Kbd>
          </span>
          <span v-else class="text-muted-foreground">{{ t('shortcut.press') }}</span>
        </template>
        <span v-else-if="keys.length" class="inline-flex items-center gap-0.5" dir="ltr">
          <Kbd v-for="key in keys" :key="key" class="text-foreground/80">{{ key }}</Kbd>
        </span>
        <span v-else class="text-muted-foreground">{{ t('shortcut.none') }}</span>
      </button>
      <tooltip v-if="!readonly && defaultValue && !isDefault" :content="t('shortcut.reset')">
        <Button
          variant="ghost"
          size="icon-xs"
          class="text-muted-foreground"
          :aria-label="t('shortcut.reset')"
          :disabled="disabled"
          data-testid="shortcut-reset"
          @click="reset"
        >
          <RotateCcw aria-hidden="true" />
        </Button>
      </tooltip>
    </div>
    <p v-if="message" :id="hintId" class="text-[12px]" :class="messageTone" role="status">{{ message }}</p>
  </div>
</template>

<script setup lang="ts">
// Key recorder for a global shortcut: shows keycaps, click to record, Esc cancels, Backspace clears.
// The parent persists the value (which registers it) and reports the OS result through `status`; a refused
// combination is rolled back to the previous one and explained inline.
import Tooltip from '../ui/tooltip/Tooltip.vue';
import { computed, nextTick, onBeforeUnmount, ref, useId, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { invoke } from '@tauri-apps/api/core';
import { Check, RotateCcw } from '@lucide/vue';
import { Button } from '@/components/ui/button';
import Kbd from '@/components/ui/kbd/Kbd.vue';
import { heldKeys, recordKey, sameShortcut, shortcutKeys } from '../../helpers/shortcut';

const props = withDefaults(defineProps<{
  mac: boolean;
  defaultValue?: string;
  disabled?: boolean;
  readonly?: boolean;
  label?: string;
  /** Registration result from the OS: `seq` grows with every report, `failed` tells if the last one failed. */
  status?: { seq: number; failed: boolean };
  /** Returns why a combination can't be used (reserved by the OS, used by another Nezzel action), or null. */
  validate?: (accelerator: string) => string | null;
}>(), { defaultValue: '', label: '', status: () => ({ seq: 0, failed: false }), validate: undefined });
const model = defineModel<string>({ required: true });

const { t } = useI18n();
const hintId = useId();
const control = ref<HTMLButtonElement | null>(null);
const recording = ref(false);
/** Modifiers held while recording, shown live so the user sees the keys land. */
const held = ref<string[]>([]);
/** Release the OS-wide Quick download hook while listening, so pressing it reaches the recorder. */
function pauseGlobal(paused: boolean) {
  void invoke('shortcuts_pause', { paused }).catch(() => {});
}
watch(recording, pauseGlobal);
const recordingHint = computed(() => t(props.mac ? 'shortcut.recordingMac' : 'shortcut.recordingPc'));
const message = ref('');
const messageTone = ref('text-muted-foreground');
const saved = ref(false);
let pending: { previous: string; seq: number } | null = null;
let savedTimer: ReturnType<typeof setTimeout> | undefined;

const keys = computed(() => shortcutKeys(model.value, props.mac));
const isDefault = computed(() => sameShortcut(model.value, props.defaultValue));
const ariaLabel = computed(() => {
  const current = keys.value.length ? keys.value.join(' ') : t('shortcut.none');
  return recording.value ? t('shortcut.press') : `${props.label}: ${current}. ${t('shortcut.change')}`;
});

function say(text: string, tone = 'text-muted-foreground') {
  message.value = text;
  messageTone.value = tone;
}

function start() {
  if (props.disabled || props.readonly) return;
  recording.value = true;
  held.value = [];
  saved.value = false;
  say(recordingHint.value);
  // WebKit (macOS) does not focus a button on click: without this, keys never arrive and blur never ends recording.
  control.value?.focus();
}

function stop() {
  if (message.value === recordingHint.value) say('');
  recording.value = false;
  held.value = [];
}

function commit(next: string) {
  recording.value = false;
  held.value = [];
  if (sameShortcut(next, model.value) && next !== '') return;
  const problem = next ? props.validate?.(next) : null;
  if (problem) {
    say(problem, 'text-destructive');
    return;
  }
  pending = { previous: model.value, seq: props.status.seq };
  model.value = next;
  if (next === '') {
    pending = null;
    say(t('shortcut.cleared'));
  }
}

function onKey(event: KeyboardEvent) {
  if (!recording.value) return; // Enter/Space: the native button click starts recording
  event.preventDefault();
  event.stopPropagation();
  const result = recordKey(event, props.mac);
  held.value = heldKeys(event, props.mac);
  switch (result.kind) {
    case 'pending':
      return;
    case 'cancel':
      stop();
      return;
    case 'clear':
      commit('');
      return;
    case 'needsModifier':
      say(t(props.mac ? 'shortcut.needsModifierMac' : 'shortcut.needsModifierPc'), 'text-amber-foreground');
      return;
    case 'ok':
      commit(result.accelerator);
  }
}

function onKeyUp(event: KeyboardEvent) {
  if (recording.value) held.value = heldKeys(event, props.mac);
}

function reset() {
  if (!props.defaultValue) return;
  commit(props.defaultValue);
  void nextTick(() => control.value?.focus());
}

watch(() => props.status.seq, (seq) => {
  if (!pending || seq <= pending.seq) return;
  const { previous } = pending;
  pending = null;
  if (props.status.failed) {
    model.value = previous;
    say(t('shortcut.taken'), 'text-destructive');
    return;
  }
  say('');
  saved.value = true;
  clearTimeout(savedTimer);
  savedTimer = setTimeout(() => {
    saved.value = false;
  }, 1500);
});

onBeforeUnmount(() => {
  clearTimeout(savedTimer);
  if (recording.value) pauseGlobal(false);
});
</script>
