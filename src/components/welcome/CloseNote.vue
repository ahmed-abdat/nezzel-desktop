<template>
  <config-provider :dir="dir">
    <div ref="root" class="cn-wrap" :class="[`edge-${edge}`, dir === 'rtl' && 'is-rtl']" data-testid="close-note">
      <div
        class="cn-pop"
        role="dialog"
        aria-labelledby="cn-title"
        :aria-describedby="hint ? 'cn-body cn-hint' : 'cn-body'"
        :style="{ '--nx': `${arrowX - 10}px` }"
        @keydown.esc.prevent="done(false)"
      >
        <div class="flex items-start gap-2.5">
          <img :src="nezzelMark" alt="" class="size-8 flex-none">
          <div class="min-w-0">
            <h2 id="cn-title" class="cn-h">{{ t('closeNote.title') }}</h2>
            <p id="cn-body" class="cn-p">{{ t('closeNote.body') }}</p>
            <p v-if="os === 'windows'" id="cn-hint" class="cn-p cn-hint">
              <i18n-t keypath="closeNote.hintWindows" scope="global">
                <template #chev><span class="cn-chev" role="img" aria-label="^"><ChevronUp class="size-[11px]" aria-hidden="true" /></span></template>
              </i18n-t>
            </p>
            <p v-else-if="os === 'linux'" id="cn-hint" class="cn-p cn-hint">{{ t('closeNote.hintLinux') }}</p>
          </div>
        </div>
        <div class="mt-3 flex justify-end gap-2">
          <Button variant="outline" size="sm" data-testid="close-note-quit" @click="done(true)">{{ t('closeNote.quit') }}</Button>
          <Button ref="gotIt" size="sm" data-testid="close-note-ok" @click="done(false)">{{ t('closeNote.gotIt') }}</Button>
        </div>
      </div>
    </div>
  </config-provider>
</template>

<script setup lang="ts">
import { computed, nextTick, onMounted, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import { ConfigProvider } from 'reka-ui';
import { invoke } from '@tauri-apps/api/core';
import { ChevronUp } from '@lucide/vue';
import nezzelMark from '../../assets/brand/nezzel-mark.svg';
import { Button } from '../ui/button';
import { localeDirection } from '../../i18n';

const props = defineProps<{ os: string; edge: 'top' | 'bottom'; arrowX: number }>();
const { t, locale } = useI18n();
const dir = computed(() => localeDirection(locale.value));
const hint = computed(() => props.os === 'windows' || props.os === 'linux');
const root = ref<HTMLElement>();
const gotIt = ref<{ $el: HTMLElement }>();

function done(quit: boolean) {
  void invoke('close_note_done', { quit }).catch(() => {});
}

onMounted(async () => {
  await (document.fonts?.ready ?? Promise.resolve());
  await nextTick();
  const height = Math.ceil(root.value?.getBoundingClientRect().height ?? 200);
  await invoke('close_note_ready', { height }).catch(() => {});
  gotIt.value?.$el?.focus?.({ preventScroll: true });
});
</script>

<style scoped>
.cn-wrap { padding: 16px 10px 20px; }
.cn-wrap.edge-bottom { padding: 6px 10px 16px; }
.cn-pop {
  --e-out: cubic-bezier(.23, 1, .32, 1);
  position: relative; background: var(--card); color: var(--foreground); border-radius: 12px; padding: 14px 14px 12px;
  box-shadow: 0 10px 20px -8px rgb(0 30 25 / .35), 0 0 0 1px rgb(14 22 20 / .08);
  font-size: 13px; line-height: 1.45;
  transform-origin: var(--nx) top;
  animation: cn-in 200ms var(--e-out);
}
.edge-bottom .cn-pop { transform-origin: var(--nx) bottom; }
/* Arrow toward the tray icon: positioned from the measured anchor, so physical left on purpose. */
.cn-pop::before { content: ""; position: absolute; top: -6px; left: var(--nx); width: 12px; height: 12px; margin-left: -6px; background: var(--card); transform: rotate(45deg); box-shadow: -1px -1px 0 rgb(14 22 20 / .08); }
.edge-bottom .cn-pop::before { top: auto; bottom: -6px; box-shadow: 1px 1px 0 rgb(14 22 20 / .08); }
@keyframes cn-in { from { opacity: 0; transform: scale(.96); } to { opacity: 1; transform: none; } }
.cn-h { font-size: 15px; font-weight: 600; line-height: 1.35; }
.cn-p { font-size: 12.5px; color: var(--muted-foreground); margin-top: 2px; line-height: 1.5; text-wrap: pretty; }
.cn-hint { margin-top: 8px; color: var(--foreground); }
.cn-chev { display: inline-grid; place-items: center; vertical-align: -3px; width: 18px; height: 16px; margin-inline: 1px; border-radius: 4px; background: var(--muted); box-shadow: inset 0 0 0 1px var(--input); }
.is-rtl .cn-h { font-size: 16px; line-height: 1.5; }
.is-rtl .cn-p { font-size: 13.5px; line-height: 1.7; }
.is-rtl :deep(button) { font-size: 14px; }
@media (prefers-reduced-motion: reduce) {
  .cn-pop { animation: cn-fade 150ms ease-out; }
  @keyframes cn-fade { from { opacity: 0; } to { opacity: 1; } }
}
</style>
