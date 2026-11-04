<template>
  <div class="flex min-h-full flex-col">
    <!-- Empty: one calm moment with a single call to action (the field lives in the toolbar). -->
    <div v-if="!hasAny" class="flex flex-1 flex-col items-center justify-center px-8 py-12 text-center animate-in fade-in-0 duration-200">
      <img :src="nezzelMark" alt="" class="size-16" />
      <h2 class="mt-5 text-[22px] font-semibold tracking-tight text-balance rtl:tracking-normal">{{ t('downloads.empty.title') }}</h2>
      <p class="mt-1.5 max-w-md text-[13.5px] text-muted-foreground text-balance">{{ t('downloads.empty.body') }}</p>
      <Button size="lg" class="mt-6" @click="focusField">{{ t('downloads.empty.cta') }}</Button>
      <!-- What Nezzel can do: three ways in, each with its real key. -->
      <!-- Equal columns, top-aligned; icon | text grid so every hint starts at the same inline offset. -->
      <ul class="mt-10 grid w-full max-w-xl grid-cols-1 items-start gap-x-6 gap-y-3 border-t border-hairline pt-5 text-start text-[12.5px] text-muted-foreground sm:grid-cols-3">
        <li v-for="hint in hints" :key="hint.key" class="grid min-w-0 grid-cols-[1rem_minmax(0,1fr)] items-start gap-x-2">
          <component :is="hint.icon" class="mt-0.5 size-4 text-primary" aria-hidden="true" />
          <div class="min-w-0 text-pretty rtl:leading-[1.6]">
            <!-- The brand stays a Latin island inside Arabic/French text. -->
            <i18n-t :keypath="hint.textKey" tag="p" scope="global">
              <template #brand><bdi dir="ltr">{{ brandName }}</bdi></template>
            </i18n-t>
            <!-- Key chips: block at the inline start, chips themselves always in LTR order (⌥ ⇧ D). -->
            <div v-if="hint.keys.length" class="mt-1">
              <span class="inline-flex gap-0.5" dir="ltr"><Kbd v-for="cap in hint.keys" :key="cap">{{ cap }}</Kbd></span>
            </div>
          </div>
        </li>
      </ul>
    </div>

    <template v-else>
      <!-- Search: title or site, case/accent/harakat-insensitive. Esc clears. -->
      <div class="flex items-center gap-1.5 border-b border-hairline px-4 py-1.5" role="search">
        <search-icon class="size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
        <label for="list-search" class="sr-only">{{ t('downloads.list.searchLabel') }}</label>
        <Input variant="bare"
          id="list-search"
          ref="searchInput"
          v-model="query"
          type="search"
          class="h-6 min-w-0 flex-1 bg-transparent text-start text-[12.5px] text-foreground outline-none placeholder:text-muted-foreground [&::-webkit-search-cancel-button]:hidden"
          :placeholder="t('downloads.list.searchPlaceholder')"
          autocomplete="off"
          spellcheck="false"
          @keydown.esc.prevent="clearSearch" />
        <tooltip v-if="query" :content="t('downloads.list.clearSearch')">
          <Button size="icon-xs" variant="ghost" class="size-6 text-muted-foreground" :aria-label="t('downloads.list.clearSearch')" @click="clearSearch">
            <x aria-hidden="true" />
          </Button>
        </tooltip>
        <!-- Status chips: only when something is in progress or failed; they combine with the sidebar's type. -->
        <div v-if="showChips" class="flex shrink-0 items-center gap-1" role="group" :aria-label="t('downloads.chips.label')">
          <router-link
            v-for="chip in chips"
            :key="chip.key"
            :to="chip.to"
            :aria-current="chip.on ? 'true' : undefined"
            class="inline-flex h-6 items-center gap-1.5 rounded-full border px-2.5 text-[12px] outline-none transition-colors duration-100 focus-visible:ring-2 focus-visible:ring-ring/40"
            :class="chip.on ? 'border-transparent bg-primary-soft font-medium text-primary-soft-foreground' : 'border-hairline text-foreground/80 hover:bg-foreground/[0.04]'"
          >
            {{ chip.label }}
            <span v-if="chip.count" class="tnum" :class="chip.key === 'failed' && !chip.on ? 'text-destructive' : chip.on ? '' : 'text-muted-foreground'">{{ count(chip.count) }}</span>
          </router-link>
        </div>
      </div>
      <p class="sr-only" role="status" aria-live="polite">{{ query ? t('downloads.list.results', { n: count(shownCount) }, shownCount) : '' }}</p>
      <!-- Finished and failed rows are announced once, so screen-reader users hear outcomes without hunting. -->
      <p class="sr-only" role="status" aria-live="polite" data-testid="row-announcer">{{ announcement }}</p>

      <p v-if="shownCount === 0" class="flex flex-1 items-center justify-center p-8 text-center text-[13px] text-muted-foreground">
        <bdi v-if="query">{{ t('downloads.list.noMatch', { query }) }}</bdi>
        <template v-else-if="view.status === 'all' && clearedCount">{{ t('downloads.list.allCleared') }}</template>
        <template v-else>{{ t('downloads.empty.filtered') }}</template>
      </p>

      <!-- Failed rows first, under their own header, so "In progress" never counts a row that stopped. -->
      <section v-if="shownFailed.length" aria-labelledby="ledger-failed">
        <selection-bar v-if="selectionCount && firstSection === 'failed'" :groups="selectedGroups" />
        <header v-else :class="headCls">
          <h2 id="ledger-failed" class="flex-1">{{ t('downloads.sections.failed') }} <span class="tnum ms-1 font-normal">{{ count(shownFailed.length) }}</span></h2>
        </header>
        <ul>
          <download-row v-for="row in shownFailed" :key="row.id" :group="row.group" />
        </ul>
      </section>

      <section v-if="shownActive.length" aria-labelledby="ledger-active">
        <selection-bar v-if="selectionCount && firstSection === 'active'" :groups="selectedGroups" />
        <header v-else :class="headCls">
          <h2 id="ledger-active" class="flex-1">{{ t('downloads.sections.active') }} <span class="tnum ms-1 font-normal">{{ count(shownActive.length) }}</span></h2>
          <template v-if="shownReady.length > 1">
            <span class="font-normal">{{ t('downloads.summary.ready', { n: count(shownReady.length) }, shownReady.length) }}</span>
            <Button variant="ghost" class="-my-1 h-6 px-2 text-primary" :disabled="isStarting" @click="downloadAllReady">{{ t('downloads.summary.downloadAll') }}</Button>
          </template>
        </header>
        <ul>
          <download-row v-for="row in shownActive" :key="row.id" :group="row.group" />
        </ul>
      </section>

      <section v-if="shownFinished.length" aria-labelledby="ledger-finished">
        <selection-bar v-if="selectionCount && firstSection === 'finished'" :groups="selectedGroups" />
        <header v-else :class="headCls">
          <h2 id="ledger-finished" class="flex-1">{{ t('downloads.sections.finished') }} <span class="tnum ms-1 font-normal">{{ count(shownFinished.length) }}</span></h2>
          <Tooltip v-if="view.status !== 'done'" :content="t('downloads.list.clearFinishedHint')">
            <Button variant="ghost" class="-my-1 h-6 px-2" @click="clearFinished(shownFinished)">
              {{ t('downloads.list.clearFinished') }}
            </Button>
          </Tooltip>
        </header>
        <ul>
          <download-row v-for="row in shownFinished" :key="row.id" :group="row.group" />
        </ul>
      </section>
    </template>
  </div>
</template>

<script setup lang="ts">
import { Input, type InputApi } from '../../components/ui/input';
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import { useI18n } from 'vue-i18n';
import { Search as SearchIcon, X } from '@lucide/vue';
import nezzelMark from '../../assets/brand/nezzel-mark.svg';
import { ClipboardCheck, ClipboardPaste, Zap } from '@lucide/vue';
import { Button } from '@/components/ui/button';
import Tooltip from '@/components/ui/tooltip/Tooltip.vue';
import Kbd from '@/components/ui/kbd/Kbd.vue';
import { usePlatform } from '../../composables/usePlatform';
import { useSettingsStore } from '../../stores/settings';
import { shortcutKeys } from '../../helpers/shortcut';
import { useCaptureShortcut } from '../../composables/useCaptureShortcut';
import SelectionBar from '../../components/downloads/SelectionBar.vue';
import { useDownloadSelection } from '../../composables/useDownloadSelection';
import DownloadRow from '../../components/downloads/DownloadRow.vue';
import { useDownloadBuckets } from '../../composables/useDownloadBuckets';
import { useLocaleNumber } from '../../composables/useLocaleNumber';
import { useDownloadList } from '../../composables/useDownloadList';
import { parseListView, viewQuery, type StatusFilter } from '../../helpers/listFilter';
import { useListHiddenStore } from '../../stores/listHidden';
import { useMediaStore } from '../../stores/media/media';
import { useMediaOptionsStore } from '../../stores/media/options';
import { optionsForChoice } from '../../helpers/quickChoice';
import { useUiPrefs } from '../../composables/useUiPrefs';

const { t } = useI18n();
const route = useRoute();
const router = useRouter();
const { count } = useLocaleNumber();
const { active, finished, ready } = useDownloadBuckets();
const { rows, shown, query, clearFinished, counts } = useDownloadList();
const hidden = useListHiddenStore();
const mediaStore = useMediaStore();
const optionsStore = useMediaOptionsStore();
const { defaultChoice } = useUiPrefs();
const isStarting = ref(false);
const { isMac } = usePlatform();
const captureShortcut = useCaptureShortcut();
const settingsStore = useSettingsStore();

/** Product name: never translated, isolated as a Latin run inside RTL text. */
const brandName = 'Nezzel';

const hints = computed(() => {
  const input = settingsStore.settings.input;
  // The OS may have given Quick download the fallback combo; show what actually works.
  const accelerator = input?.globalShortcuts && !captureShortcut.failed ? (captureShortcut.active ?? input.captureShortcut) : '';
  const quick = accelerator ? shortcutKeys(accelerator, isMac.value) : [];
  return [
    { key: 'paste', icon: ClipboardPaste, textKey: 'downloads.empty.hintPaste', keys: shortcutKeys('CommandOrControl+V', isMac.value) },
    { key: 'quick', icon: Zap, textKey: 'downloads.empty.hintQuick', keys: quick },
    { key: 'copied', icon: ClipboardCheck, textKey: 'downloads.empty.hintCopied', keys: [] as string[] },
  ];
});

const headCls = 'sticky top-0 z-[1] flex items-center gap-2 border-b border-hairline bg-background/90 pe-4 ps-8 py-1.5 text-[11.5px] font-semibold text-muted-foreground backdrop-blur';

const view = computed(() => parseListView(route.query));
const statusCounts = computed(() => counts(view.value.type).statuses);
const clearedCount = computed(() => rows.value.filter(row => row.cleared).length);
/**
 * Chips appear when they split the list: at least two kinds of rows (in progress, failed, finished),
 * a picked chip, or finished rows taken off the list (Completed is the only way back to them).
 */
const showChips = computed(() => {
  const { all, active, failed } = statusCounts.value;
  const kinds = [active, failed, all - active - failed].filter(n => n > 0).length;
  return view.value.status !== 'all' || (kinds > 1 && active + failed > 0) || clearedCount.value > 0;
});
const chips = computed(() => (['all', 'active', 'failed', 'done'] as StatusFilter[])
  .filter(key => key === 'all' || statusCounts.value[key] > 0 || view.value.status === key)
  .map(key => ({
    key,
    label: t(`downloads.chips.${key}`),
    count: key === 'all' ? 0 : statusCounts.value[key],
    on: view.value.status === key,
    to: { name: 'home', query: viewQuery({ type: view.value.type, status: key }) },
  })));
/** A picked chip whose rows all finished would leave an empty list: fall back to all. */
watch(() => view.value.status !== 'all' && statusCounts.value[view.value.status] === 0, (empty) => {
  if (empty) void router.replace({ name: 'home', query: viewQuery({ type: view.value.type, status: 'all' }) });
});
/** Rows hidden by "Clear finished" still count until the undo window closes, so onboarding doesn't cover the toast. */
const hasAny = computed(() => active.value.length + finished.value.length + hidden.ids.size > 0);
const shownRows = computed(() => shown(view.value));
const shownActive = computed(() => shownRows.value.filter(row => row.status === 'active'));
const shownFailed = computed(() => shownRows.value.filter(row => row.status === 'failed'));
const shownFinished = computed(() => shownRows.value.filter(row => row.status === 'done'));
const shownCount = computed(() => shownRows.value.length);
/** "Download all" acts only on the rows the current filter and search show. */
const shownReady = computed(() => {
  const ids = new Set(shownActive.value.map(row => row.id));
  return ready.value.filter(group => ids.has(group.id));
});

/** Only transitions count (in progress → done/failed), never rows that were already finished on load. */
const announcement = ref('');
const lastStatus = new Map<string, string>();
watch(rows, (list) => {
  const lines: string[] = [];
  for (const row of list) {
    const before = lastStatus.get(row.id);
    if (before === 'active' && row.status !== 'active') lines.push(t(`downloads.announce.${row.status}`, { title: row.title }));
    lastStatus.set(row.id, row.status);
  }
  if (lines.length) announcement.value = lines.join('. ');
}, { immediate: true });

const searchInput = ref<InputApi | null>(null);
function clearSearch() {
  query.value = '';
}

const selection = useDownloadSelection();
const selectionCount = selection.count;
const visibleOrder = computed(() => [...shownFailed.value, ...shownActive.value, ...shownFinished.value].map(row => row.id));
watch(visibleOrder, ids => selection.setOrder(ids), { immediate: true });
const selectedGroups = computed(() => [...shownFailed.value, ...shownActive.value, ...shownFinished.value].filter(row => selection.isSelected(row.id)).map(row => row.group));
const firstSection = computed(() => (shownFailed.value.length ? 'failed' : shownActive.value.length ? 'active' : 'finished'));
const inTextField = (el: EventTarget | null) => el instanceof HTMLElement && (el.isContentEditable || ['INPUT', 'TEXTAREA', 'SELECT'].includes(el.tagName));

/** Cmd/Ctrl+F jumps to the search field, like a native list. Cmd/Ctrl+A selects all rows, Esc clears. */
function onWindowKey(event: KeyboardEvent) {
  if ((event.metaKey || event.ctrlKey) && !event.altKey && event.key.toLowerCase() === 'a' && !inTextField(event.target) && visibleOrder.value.length) {
    event.preventDefault();
    selection.selectAll();
    return;
  }
  if (event.key === 'Escape' && selectionCount.value && !inTextField(event.target)) {
    selection.clear();
    return;
  }
  if ((event.metaKey || event.ctrlKey) && !event.altKey && event.key.toLowerCase() === 'f' && searchInput.value) {
    event.preventDefault();
    searchInput.value.focus();
    searchInput.value.select();
  }
}
onMounted(() => window.addEventListener('keydown', onWindowKey));
onBeforeUnmount(() => {
  window.removeEventListener('keydown', onWindowKey);
  selection.clear();
});

function focusField() {
  document.getElementById('queue-url-input')?.focus();
}

async function downloadAllReady() {
  isStarting.value = true;
  try {
    await Promise.all(shownReady.value.map((group) => {
      const options = optionsStore.getOptions(group.id) ?? optionsForChoice(defaultChoice.value, group.formats ?? [], group.mediaKind);
      optionsStore.setOptions(group.id, options);
      return mediaStore.downloadGroup(group.id, options);
    }));
  } finally {
    isStarting.value = false;
  }
}
</script>
