<template>
  <form class="mx-auto flex min-w-0 max-w-[720px] flex-1 items-center justify-center gap-2" autocomplete="off" @submit.prevent="onSubmit">
    <!-- Grows and centres in the toolbar, like Safari's address field. -->
    <div class="flex h-8 w-full min-w-28 flex-1 items-center gap-1.5 rounded-[7px] ps-2.5 bg-foreground/[0.055] pe-0.5 transition-[background-color,box-shadow] focus-within:bg-card focus-within:ring-2 focus-within:ring-ring/30">
      <Link class="size-3.5 shrink-0 text-muted-foreground rtl:-scale-x-100" aria-hidden="true" />
      <label for="queue-url-input" class="sr-only">{{ t('downloads.field.label') }}</label>
      <Input variant="bare"
        id="queue-url-input"
        ref="input"
        v-model="url"
        class="h-full min-w-0 flex-1 bg-transparent text-start text-[12.5px] text-foreground outline-none placeholder:text-muted-foreground"
        :placeholder="nudge ? t('downloads.field.pasteFirst') : placeholder"
        :class="nudge && 'placeholder:text-foreground/80'"
        type="text"
        inputmode="url"
        :dir="url ? 'auto' : clipboardHasValidUrl ? 'ltr' : undefined"
        autocomplete="off"
        autocorrect="off"
        autocapitalize="off"
        spellcheck="false"
        @keydown.enter.shift.prevent="submit(true)"
        @paste="onNativePaste" />
      <Kbd v-if="!url" class="hidden sm:inline-flex">{{ isMac ? t('shell.keys.pasteMac') : t('shell.keys.paste') }}</Kbd>
      <!-- Clipboard state as a small indicator, not placeholder text. -->
      <tooltip :content="clipboardTip">
        <span class="inline-flex size-5 shrink-0 items-center justify-center" :aria-label="clipboardTip" role="img">
          <ClipboardCheck v-if="watchClipboard.enabled" class="size-3.5 text-primary" aria-hidden="true" />
          <ClipboardX v-else class="size-3.5 text-muted-foreground/60" aria-hidden="true" />
        </span>
      </tooltip>
      <input ref="fileInput" type="file" accept=".csv,.txt,text/csv,text/plain" class="hidden" @change="onFile" />
      <div ref="menuRoot" class="relative">
        <tooltip :content="t('downloads.field.more')">
          <Button
            size="icon-xs"
            variant="ghost"
            class="size-6 text-muted-foreground"
            :aria-label="t('downloads.field.more')"
            aria-haspopup="menu"
            :aria-expanded="menuOpen ? 'true' : 'false'"
            @click="menuOpen = !menuOpen"
            @keydown.down.prevent="menuOpen = true"
          >
            <Ellipsis />
          </Button>
        </tooltip>
        <ul
          v-if="menuOpen"
          class="absolute end-0 top-8 z-[80] w-72 rounded-lg border border-hairline bg-popover p-1 text-[13px] shadow-lg animate-in fade-in-0 zoom-in-95 duration-150"
          role="menu"
          @keydown="onMenuKey"
        >
          <li role="none">
            <button role="menuitem" type="button" :class="itemCls" @click="pickFile">
              <FileUp aria-hidden="true" />
              <span class="min-w-0 flex-1">
                <span class="block">{{ t('downloads.field.importFile') }}</span>
                <span class="block text-[12px] text-muted-foreground">{{ t('downloads.field.importFileHint') }}</span>
              </span>
            </button>
          </li>
          <li role="none">
            <button role="menuitemcheckbox" type="button" :class="itemCls" :aria-checked="watchClipboard.enabled ? 'true' : 'false'" @click="toggleWatch">
              <ClipboardCheck aria-hidden="true" />
              <span class="min-w-0 flex-1">
                <span class="block">{{ t('downloads.field.watch') }}</span>
                <span class="block text-[12px] text-muted-foreground">{{ t('downloads.field.watchHint') }}</span>
              </span>
              <Switch :model-value="watchClipboard.enabled" tabindex="-1" aria-hidden="true" class="pointer-events-none mt-0.5" />
            </button>
          </li>
          <li role="none">
            <button role="menuitem" type="button" :class="[itemCls, hasActiveInputFilters && 'is-on']" @click="openFilters">
              <Filter :class="hasActiveInputFilters && 'text-primary'" aria-hidden="true" />
              <span class="min-w-0 flex-1">
                <span class="block" :class="hasActiveInputFilters && 'font-medium text-primary'">{{ t('downloads.field.filters') }}</span>
                <span class="block text-[12px] text-muted-foreground">{{ hasActiveInputFilters ? t('downloads.field.filtersOn') : t('downloads.field.filtersHint') }}</span>
              </span>
            </button>
          </li>
        </ul>
      </div>
    </div>
    <p class="sr-only" role="status" aria-live="polite">{{ nudge ? t('downloads.field.pasteFirst') : '' }}</p>
    <!-- Teal only once there is a link to act on; empty, it focuses the field and says what to do. -->
    <Button type="submit" :variant="canSubmit ? 'default' : 'outline'" @click.shift.prevent="canSubmit ? submit(true) : pointAtField()">
      {{ t('downloads.field.submit') }}
    </Button>
  </form>
</template>

<script setup lang="ts">
import { Input, type InputApi } from '../ui/input';
import { readClipboardText } from '../../tauri/clipboard';
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { useRouter } from 'vue-router';
import { ClipboardCheck, ClipboardX, Ellipsis, FileUp, Filter, Link } from '@lucide/vue';
import Switch from '@/components/ui/switch/Switch.vue';
import Tooltip from '@/components/ui/tooltip/Tooltip.vue';
import { Button } from '@/components/ui/button';
import Kbd from '@/components/ui/kbd/Kbd.vue';
import { usePlatform } from '../../composables/usePlatform';
import { useToastStore } from '../../stores/toast';
import { useLinkIntake } from '../../composables/useLinkIntake';
import { useCaptureShortcut } from '../../composables/useCaptureShortcut';
import { useCapturePlatform } from '../../composables/useCapturePlatform';
import { useSettingsStore } from '../../stores/settings';
import { shortcutLabel } from '../../helpers/shortcut';

const { t } = useI18n();
const router = useRouter();
const { url, placeholder, clipboardHasValidUrl, canSubmit, submit, importFile, hasActiveInputFilters, watchClipboard } = useLinkIntake();
const { isMac } = usePlatform();
const capture = useCaptureShortcut();
const capturePlatform = useCapturePlatform();
const settings = useSettingsStore();
const clipboardTip = computed(() => {
  const state = watchClipboard.enabled ? t('downloads.field.watchOnTip') : t('downloads.field.watchOffTip');
  if (capturePlatform.session === 'wayland') return state;
  const shortcut = settings.settings.input.globalShortcuts && !capture.failed && capture.active;
  return t('downloads.field.clipboardTip', {
    state,
    keys: shortcut
      ? t('downloads.field.captureShortcut', { shortcut: shortcutLabel(shortcut, isMac.value) })
      : t('downloads.field.captureSettings'),
  });
});

const input = ref<InputApi | null>(null);
const fileInput = ref<HTMLInputElement | null>(null);
const menuRoot = ref<HTMLElement | null>(null);
const menuOpen = ref(false);
const itemCls = 'flex w-full items-start gap-2.5 rounded-[6px] px-2 py-1.5 text-start outline-none transition-colors duration-100 hover:bg-foreground/[0.05] focus-visible:bg-foreground/[0.05] focus-visible:outline-solid focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--nz-focus)] [&>svg]:mt-0.5 [&>svg]:size-4 [&>svg]:shrink-0 [&>svg]:text-muted-foreground';
let immediateFromFile = false;

/** With nothing to add (empty field, no copied link) the button points the user at the field instead. */
function onSubmit() {
  if (!canSubmit.value) {
    pointAtField();
    return;
  }
  submit(false);
}

/** Empty submit: focus the field and swap its placeholder for a short "paste a link first" for a moment. */
const nudge = ref(false);
let nudgeTimer: ReturnType<typeof setTimeout> | undefined;
function pointAtField() {
  input.value?.focus();
  nudge.value = true;
  clearTimeout(nudgeTimer);
  nudgeTimer = setTimeout(() => {
    nudge.value = false;
  }, 2000);
}
watch(url, () => {
  nudge.value = false;
});

/** Closing the menu from an item puts focus back on its button, so keyboard users don't land on the page body. */
function closeMenu() {
  menuOpen.value = false;
  menuRoot.value?.querySelector<HTMLElement>('[aria-haspopup="menu"]')?.focus();
}

function pickFile(event: MouseEvent) {
  immediateFromFile = event.shiftKey;
  closeMenu();
  fileInput.value?.click();
}

async function onFile(event: Event) {
  const target = event.target as HTMLInputElement;
  const file = target.files?.[0];
  target.value = '';
  await importFile(file, immediateFromFile);
  immediateFromFile = false;
}

/** A switch, not a one-shot command: the menu stays open so the new state is visible. */
function toggleWatch() {
  watchClipboard.toggle();
}

function openFilters() {
  menuOpen.value = false;
  void router.push({ name: 'input-filters' });
}

/** Menu keys: focus moves into the menu on open; ↑/↓ move, Esc closes and returns to the button, Tab closes. */
watch(menuOpen, async (open) => {
  if (!open) return;
  await nextTick();
  menuRoot.value?.querySelector<HTMLElement>('[role^="menuitem"]')?.focus();
});

function onMenuKey(event: KeyboardEvent) {
  const items = Array.from(menuRoot.value?.querySelectorAll<HTMLElement>('[role^="menuitem"]') ?? []);
  const index = items.indexOf(document.activeElement as HTMLElement);
  if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
    event.preventDefault();
    const step = event.key === 'ArrowDown' ? 1 : -1;
    items[(index + step + items.length) % items.length]?.focus();
  } else if (event.key === 'Home' || event.key === 'End') {
    event.preventDefault();
    items[event.key === 'Home' ? 0 : items.length - 1]?.focus();
  } else if (event.key === 'Escape') {
    event.preventDefault();
    closeMenu();
  } else if (event.key === 'Tab') {
    menuOpen.value = false;
  }
}

function onDocumentClick(event: MouseEvent) {
  if (menuOpen.value && menuRoot.value && !menuRoot.value.contains(event.target as Node)) {
    menuOpen.value = false;
  }
}

async function paste() {
  input.value?.focus();
  const text = await readClipboardText();
  if (text != null) url.value = text.replace(/[\r\n]+/g, ' ');
  else useToastStore().showToast(t('feedback.clipboardFailed'), { style: 'error' });
}

/** WebKitGTK on some Wayland desktops hands the field an empty paste; read the clipboard ourselves then. */
function onNativePaste(event: ClipboardEvent) {
  const text = event.clipboardData?.getData('text/plain');
  if (text) {
    if (/[\r\n]/.test(text)) {
      event.preventDefault();
      const field = event.target as HTMLInputElement;
      const start = field.selectionStart ?? url.value.length;
      const end = field.selectionEnd ?? start;
      url.value = url.value.slice(0, start) + text.replace(/[\r\n]+/g, ' ') + url.value.slice(end);
    }
    return;
  }
  event.preventDefault();
  void readClipboardText().then((text) => {
    if (text) url.value = text.replace(/[\r\n]+/g, ' ');
  });
}

defineExpose({ focus: () => input.value?.focus(), paste });

onMounted(() => {
  input.value?.focus();
  document.addEventListener('click', onDocumentClick);
});
onBeforeUnmount(() => {
  document.removeEventListener('click', onDocumentClick);
  clearTimeout(nudgeTimer);
});
</script>
