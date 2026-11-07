<script setup lang="ts">
import { computed, onBeforeUnmount, shallowRef } from 'vue';
import { useI18n } from 'vue-i18n';
import { Button } from '@/components/ui/button';
import { useCapturePlatform } from '@/composables/useCapturePlatform';
import { CAPTURE_COMMAND } from '@/helpers/shortcut';
import { copyText } from '@/tauri/clipboard';

const { t } = useI18n();
const platform = useCapturePlatform();
const command = computed(() => platform.captureCommand || CAPTURE_COMMAND);
const result = shallowRef<'copied' | 'failed' | null>(null);

let reset: ReturnType<typeof setTimeout> | undefined;
async function copyCommand() {
  clearTimeout(reset);
  try {
    await copyText(command.value);
    result.value = 'copied';
    // The label goes back to "Copy command" so a second copy also gets feedback.
    reset = setTimeout(() => {
      result.value = null;
    }, 2000);
  } catch {
    result.value = 'failed';
  }
}
onBeforeUnmount(() => clearTimeout(reset));
</script>

<template>
  <span class="block text-[12px] leading-relaxed text-muted-foreground">
    <span class="block">{{ t('shortcut.wayland') }}</span>
    <span class="mt-1 flex items-center gap-2">
      <code dir="ltr" class="min-w-0 flex-1 break-all font-mono text-[11.5px] [unicode-bidi:isolate]">{{ command }}</code>
      <Button variant="outline" data-testid="copy-shortcut-command" @click="copyCommand">
        {{ t(result === 'copied' ? 'shortcut.commandCopied' : 'shortcut.copyCommand') }}
      </Button>
    </span>
    <span role="status" class="block" :class="result === 'failed' && 'text-destructive'">{{ result === 'failed' ? t('shortcut.copyFailed') : '' }}</span>
  </span>
</template>
