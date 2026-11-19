<template>
  <!-- Parent line: one row for the whole playlist (design A). Children follow as sibling rows, indented. -->
  <li
    :id="`playlist-${parent.id}`"
    class="row group relative flex items-start gap-3 border-b border-hairline pe-4 ps-8 outline-none focus-visible:outline-solid focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-[var(--nz-focus)]"
    :class="[tint, compactRows ? 'py-1.5' : 'py-2.5']"
    :aria-selected="selected"
    :aria-level="1"
    :data-phase="a.phase"
    :data-group-id="parent.id"
    data-playlist
    :aria-busy="a.phase === 'downloading' || a.phase === 'reading' ? 'true' : undefined"
    :aria-label="title"
    :aria-description="metaText"
    tabindex="0"
    @keydown="onKey"
    @click="onRowClick"
  >
    <Checkbox
      class="absolute start-3 top-3 z-[1] transition-opacity"
      :class="selected || anySelected ? 'opacity-100' : 'opacity-0 group-hover:opacity-100 group-focus-within:opacity-100'"
      :model-value="selected"
      :aria-label="t('downloads.selection.select', { title })"
      @click.stop="onBoxClick"
    />
    <Button
      size="icon-xs"
      variant="ghost"
      class="-ms-1 shrink-0 self-center rounded-full text-muted-foreground"
      :aria-expanded="open"
      :aria-controls="open ? childIds : undefined"
      :aria-label="t(open ? 'downloads.playlist.hide' : 'downloads.playlist.show', { title })"
      data-testid="playlist-toggle"
      @click.stop="toggle"
    >
      <ChevronRight class="transition-transform duration-150 ease-[var(--nz-ease)] rtl:-scale-x-100 motion-reduce:transition-none" :class="open && 'rotate-90 rtl:-rotate-90'" aria-hidden="true" />
    </Button>

    <!-- Stacked thumbnail: two edges behind it say "several", the badge says how many. -->
    <figure v-if="showThumbnails" class="relative m-0 shrink-0 self-start" :class="compactRows ? 'mt-0 h-[27px] w-12' : 'mt-1 h-10 w-[72px]'" aria-hidden="true">
      <span class="absolute inset-x-2 -top-1 h-2 rounded-t-[4px] bg-foreground/10" />
      <span class="absolute inset-x-1 -top-0.5 h-2 rounded-t-[4px] bg-foreground/15" />
      <span class="relative grid size-full place-items-center overflow-hidden rounded-[5px] bg-muted text-muted-foreground ring-1 ring-black/5 dark:ring-white/5">
        <img v-if="thumbUrl" :src="thumbUrl" alt="" class="size-full object-cover" loading="lazy" referrerpolicy="no-referrer" @error="thumbFailed = true" />
        <ListVideo v-else class="size-4" />
      </span>
      <span v-if="!compactRows" class="absolute bottom-1 end-1 inline-flex items-center gap-0.5 rounded-[4px] bg-black/70 px-1 py-px text-[10.5px] font-semibold leading-tight text-white tnum">
        <ListVideo class="size-3" />{{ count(a.total) }}
      </span>
    </figure>

    <div class="min-w-0 flex-1" :class="!compactRows && 'min-h-10'">
      <p class="flex min-w-0 font-medium leading-5">
        <span :dir="parent.title ? 'auto' : 'ltr'" class="min-w-0 truncate" :title="title">{{ title }}</span>
      </p>
      <div class="mt-1 flex min-w-0 items-center gap-2.5 text-[12px] text-muted-foreground">
        <nz-progress
          v-if="showBar"
          :value="a.phase === 'reading' ? null : a.fraction * 100"
          :muted="a.phase === 'paused' || a.phase === 'cancelled'"
          class="max-w-[180px] flex-[1_1_120px]"
        />
        <span class="flex min-w-0 items-center gap-1.5 truncate" data-testid="playlist-meta">
          <template v-for="(part, index) in meta" :key="part.key">
            <span v-if="index > 0" class="nz-sep" aria-hidden="true" />
            <span v-if="part.danger" class="inline-flex items-center gap-1 font-medium text-destructive-foreground"><triangle-alert class="size-3.5 shrink-0" aria-hidden="true" /><bdi class="tnum">{{ part.text }}</bdi></span>
            <bdi v-else class="tnum">{{ part.text }}</bdi>
          </template>
        </span>
      </div>
      <login-needed v-if="loginCard" class="mt-1" :card="loginCard" :site="site" diagnostic-code="unknown" />
    </div>

    <!-- Trailing actions: fixed slot for Pause/Resume so the column never shifts when it swaps. -->
    <div class="flex shrink-0 items-center gap-1 self-center">
      <Button v-if="a.phase === 'ready'" :disabled="busy" @click.stop="run(resumeAll)">{{ t('downloads.playlist.downloadAll') }}</Button>
      <Button v-else-if="a.phase === 'paused' || a.phase === 'cancelled'" variant="soft" :disabled="busy" @click.stop="run(resumeAll)">
        <Play aria-hidden="true" />{{ t('downloads.playlist.resumeAll') }}
      </Button>
      <Button v-else-if="a.phase === 'failed'" variant="outline" :disabled="busy" @click.stop="run(retryFailed)">
        <RotateCw aria-hidden="true" />{{ t('downloads.playlist.retryFailed') }}
      </Button>
      <tooltip v-else-if="a.phase === 'downloading' || a.phase === 'waiting'" :content="t('downloads.playlist.pauseAll')">
        <Button size="icon-xs" variant="outline" class="rounded-full" :aria-label="t('downloads.playlist.pauseAll')" :disabled="busy" @click.stop="run(pauseAll)">
          <Pause />
        </Button>
      </tooltip>
      <tooltip v-if="canCancel" :content="t('downloads.playlist.cancelAll')">
        <Button size="icon-xs" variant="ghost" class="rounded-full text-muted-foreground" :aria-label="t('downloads.playlist.cancelAll')" @click.stop="cancelAll">
          <X />
        </Button>
      </tooltip>
      <tooltip v-if="a.phase !== 'reading'" :content="t('downloads.playlist.openFolder')">
        <Button
          size="icon-xs"
          variant="ghost"
          class="rounded-full text-muted-foreground"
          :class="a.phase === 'done' && 'opacity-0 transition-opacity group-hover:opacity-100 group-focus-within:opacity-100 focus-visible:opacity-100'"
          :aria-label="t('downloads.playlist.openFolder')"
          @click.stop="openFolder"
        >
          <FolderOpen />
        </Button>
      </tooltip>
      <dropdown-menu>
        <dropdown-menu-trigger as-child>
          <Button size="icon-xs" variant="ghost" class="rounded-full text-muted-foreground" :aria-label="t('downloads.row.more')" @click.stop>
            <Ellipsis />
          </Button>
        </dropdown-menu-trigger>
        <dropdown-menu-content>
          <dropdown-menu-item v-if="a.failed > 0 && a.phase !== 'failed'" @select="run(retryFailed)"><RotateCw aria-hidden="true" />{{ t('downloads.playlist.retryFailed') }}</dropdown-menu-item>
          <dropdown-menu-item v-if="parent.url" @select="copyLink"><Copy aria-hidden="true" />{{ t('downloads.playlist.copyLink') }}</dropdown-menu-item>
          <dropdown-menu-separator v-if="parent.url || a.failed > 0" />
          <dropdown-menu-item @select="removeFromList"><ListX aria-hidden="true" />{{ t('downloads.playlist.removeFromList') }}</dropdown-menu-item>
        </dropdown-menu-content>
      </dropdown-menu>
    </div>
    <div v-if="showBar && a.phase !== 'reading'" class="sr-only" role="progressbar" :aria-valuenow="Math.round(a.fraction * 100)" aria-valuemin="0" aria-valuemax="100" :aria-label="title" />
  </li>

  <template v-if="open">
    <li v-if="hiddenSaved > 0" :class="foldCls">
      <button type="button" :class="foldBtn" class="text-primary" data-testid="playlist-show-saved" @click="savedLimit += FOLD_PAGE">{{ t('downloads.playlist.showSaved', { n: count(showSavedCount) }, showSavedCount) }}</button>
    </li>
    <li v-else-if="savedFolds && savedLimit > 0" :class="foldCls">
      <button type="button" :class="foldBtn" class="text-muted-foreground" @click="savedLimit = 0">{{ t('downloads.playlist.hideSaved') }}</button>
    </li>
    <download-row
      v-for="(child, i) in visibleChildren"
      :key="child.id"
      :group="child"
      :highlighted="highlightedId === child.id"
      nested
      :parent-id="parent.id"
      :posinset="(child.parentIndex ?? i) + 1"
      :setsize="children.length"
    />
    <li v-if="hiddenWaiting > 0" :class="foldCls">
      <button type="button" :class="foldBtn" class="text-muted-foreground" data-testid="playlist-more-waiting" @click="waitingLimit += FOLD_PAGE">{{ t('downloads.playlist.moreWaiting', { n: count(hiddenWaiting) }, hiddenWaiting) }}</button>
    </li>
    <li v-if="parent.skipped > 0" :class="foldCls">
      <span class="inline-flex items-center gap-1.5 py-1 text-[12px] text-muted-foreground" role="status"><triangle-alert class="size-3.5" aria-hidden="true" />{{ t('downloads.playlist.skipped', { n: count(parent.skipped) }, parent.skipped) }}</span>
    </li>
  </template>
</template>

<script setup lang="ts">
import { computed, ref, toRef, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { ChevronRight, Copy, Ellipsis, FolderOpen, ListVideo, ListX, Pause, Play, RotateCw, TriangleAlert, X } from '@lucide/vue';
import type { Group, PlaylistParent } from '../../tauri/types/group';
import { Button } from '@/components/ui/button';
import { Checkbox } from '@/components/ui/checkbox';
import NzProgress from '@/components/ui/progress/Progress.vue';
import Tooltip from '@/components/ui/tooltip/Tooltip.vue';
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger } from '@/components/ui/dropdown-menu';
import DownloadRow from './DownloadRow.vue';
import LoginNeeded from './LoginNeeded.vue';
import { usePlaylistRow } from '../../composables/usePlaylistRow';
import { useLocaleNumber } from '../../composables/useLocaleNumber';
import { useUiPrefs } from '../../composables/useUiPrefs';
import { useDownloadSelection } from '../../composables/useDownloadSelection';
import { usePlatform } from '../../composables/usePlatform';
import { copyText } from '../../tauri/clipboard';
import { useToastStore } from '../../stores/toast';
import { useMediaProgressStore } from '../../stores/media/progress';
import { ProgressStage } from '../../tauri/types/progress';
import { useSettingsStore } from '../../stores/settings';
import { useLoginStore } from '../../stores/login';
import { remoteImagesAllowed } from '../../helpers/remoteImages';
import { groupSite } from '../../helpers/groupView';
import { formatDuration } from '../../helpers/units';
import { ROW_SELECTOR } from '../../helpers/domIds';
import { isPlaylistOpen, setPlaylistOpen } from '../../composables/usePlaylistOpen';

const props = defineProps<{
  parent: PlaylistParent;
  /** Every child, playlist order (aggregate and actions). */
  children: Group[];
  /** Children the current filter and search show. */
  shown: Group[];
  highlightedId?: string | null;
}>();

/** Waiting downloads listed before the "N more waiting" fold. */
const WAITING_VISIBLE = 3;
/** Fewer saved items than this are never folded. */
const SAVED_FOLD_MIN = 3;
/** Rows a fold opens per click. */
const FOLD_PAGE = 100;

const { t, locale } = useI18n();
const { count, size: formatSize, speed: formatSpeed } = useLocaleNumber();
const { compactRows, showThumbnails } = useUiPrefs();
const settingsStore = useSettingsStore();
const loginStore = useLoginStore();
const selection = useDownloadSelection();
const { isMac } = usePlatform();
const toastStore = useToastStore();

const parentId = computed(() => props.parent.id);
const { aggregate: a, pauseAll, resumeAll, cancelAll, retryFailed, openFolder, removeFromList, phaseOfChild } = usePlaylistRow(parentId, toRef(props, 'children'));

const title = computed(() => props.parent.title?.trim() || props.parent.url || '');
const site = computed(() => (props.children[0] ? groupSite(props.children[0]) ?? '' : ''));
const loginCard = computed(() => loginStore.cardFor(props.parent.id));
const isolate = (text: string) => (locale.value.startsWith('ar') ? `⁦${text}⁩` : text);

/** Open by default while it is fresh or has a failure; a playlist restored from history starts closed. */
const open = computed(() => isPlaylistOpen(props.parent.id, !props.parent.id.startsWith('library-playlist:') || a.value.failed > 0));
function toggle() {
  setPlaylistOpen(props.parent.id, !open.value);
}

const showBar = computed(() => ['reading', 'downloading', 'waiting', 'paused'].includes(a.value.phase));
const canCancel = computed(() => ['downloading', 'waiting', 'paused', 'ready'].includes(a.value.phase) || (a.value.phase === 'reading' && a.value.active + a.value.paused > 0));

type MetaPart = { key: string; text: string; danger?: boolean };
const meta = computed<MetaPart[]>(() => {
  const x = a.value;
  const parts: MetaPart[] = [];
  const of = { key: 'of', text: t('downloads.playlist.itemsOf', { done: count(x.done), total: count(x.countable) }) };
  const bytes = x.savedBytes > 0
    ? { key: 'bytes', text: x.totalBytes ? `${isolate(formatSize(x.savedBytes))} / ${isolate(formatSize(x.totalBytes))}` : isolate(formatSize(x.savedBytes)) }
    : null;
  const failed = x.failed > 0 ? { key: 'failed', text: t('downloads.playlist.failed', { n: count(x.failed) }, x.failed), danger: true } : null;
  const saved = { key: 'saved', text: t('downloads.playlist.saved', { n: count(x.done) }, x.done) };
  const byPhase: Record<string, (MetaPart | null | false)[]> = {
    reading: [{ key: 'reading', text: t('downloads.playlist.reading') }],
    ready: [{ key: 'items', text: t('downloads.playlist.items', { n: count(x.total) }, x.total) }, { key: 'ready', text: t('downloads.playlist.ready') }],
    downloading: [
      of, bytes,
      x.speedBps > 0 && { key: 'speed', text: isolate(formatSpeed(x.speedBps)) },
      x.etaSecs != null && { key: 'eta', text: t('downloads.row.left', { eta: isolate(formatDuration(Math.max(60, x.etaSecs))) }) },
    ],
    waiting: [{ key: 'waiting', text: t('downloads.playlist.waiting') }, of],
    paused: [{ key: 'paused', text: t('downloads.playlist.paused') }, of, bytes],
    failed: [saved],
    cancelled: [{ key: 'cancelled', text: t('downloads.playlist.cancelled') }, x.done > 0 && saved],
    done: [saved, x.savedBytes > 0 && { key: 'bytes', text: isolate(formatSize(x.savedBytes)) }],
  };
  for (const part of byPhase[x.phase] ?? []) if (part) parts.push(part);
  if (failed) parts.push(failed);
  return parts;
});
const metaText = computed(() => meta.value.map(part => part.text).join(', '));

const tint = computed(() => (selected.value || props.highlightedId === props.parent.id
  ? 'bg-primary-soft'
  : a.value.phase === 'ready' || loginCard.value?.kind === 'needed'
    ? 'bg-amber-soft'
    // Partial failure: the failed items carry the red tint; the playlist line stays calm (icon + count say it).
    : 'hover:bg-foreground/[0.025] focus-visible:bg-primary-soft'));

const thumbFailed = ref(false);
const thumbSource = computed(() => props.parent.thumbnail ?? props.children.find(child => child.thumbnail)?.thumbnail);
watch(thumbSource, () => {
  thumbFailed.value = false;
});
const thumbUrl = computed(() => (!thumbFailed.value && thumbSource.value && remoteImagesAllowed(settingsStore.settings.network, thumbSource.value) && thumbSource.value) || null);

/* Folds: saved items fold away once there are a few; waiting ones show the next few. Failed, live and ready ones
   always show. Folds open a page at a time, so a 1000-item playlist never mounts 1000 rows in one go. */
const savedLimit = ref(0);
const waitingLimit = ref(WAITING_VISIBLE);
const savedChildren = computed(() => props.shown.filter(child => phaseOfChild(child) === 'done'));
const savedFolds = computed(() => savedChildren.value.length >= SAVED_FOLD_MIN);
const hiddenSaved = computed(() => (savedFolds.value ? Math.max(0, savedChildren.value.length - savedLimit.value) : 0));
const queuedChildren = computed(() => props.shown.filter(child => isQueued(child)));
const hiddenWaiting = computed(() => Math.max(0, queuedChildren.value.length - waitingLimit.value));
const visibleChildren = computed(() => {
  const list: Group[] = [];
  let savedShown = 0;
  let waitingShown = 0;
  const queued = new Set(queuedChildren.value.map(child => child.id));
  for (const child of props.shown) {
    if (savedFolds.value && phaseOfChild(child) === 'done' && savedShown++ >= savedLimit.value) continue;
    if (queued.has(child.id) && waitingShown++ >= waitingLimit.value) continue;
    list.push(child);
  }
  return list;
});
const showSavedCount = computed(() => Math.min(FOLD_PAGE, hiddenSaved.value));
const childIds = computed(() => visibleChildren.value.map(child => `playlist-item-${child.id}`).join(' '));

const progressStore = useMediaProgressStore();
function isQueued(child: Group) {
  if (phaseOfChild(child) !== 'downloading') return false;
  const progress = progressStore.findDownloadProgress(child.id);
  return !progress || progress.stage === ProgressStage.queued;
}

/* Selection: the playlist is one selectable line; the bar acts on its downloads. */
const selected = computed(() => selection.isSelected(props.parent.id));
const anySelected = computed(() => selection.count.value > 0);
function onRowClick(event: MouseEvent) {
  if ((event.target as HTMLElement).closest('button, a, input, [role="menu"]')) return;
  const toggleKey = isMac.value ? event.metaKey : event.ctrlKey;
  if (event.shiftKey) selection.extend(props.parent.id);
  else if (toggleKey) selection.toggle(props.parent.id);
}
function onBoxClick(event: MouseEvent) {
  if (event.shiftKey) selection.extend(props.parent.id);
  else selection.toggle(props.parent.id);
}

const busy = ref(false);
async function run(action: () => Promise<void>) {
  if (busy.value) return;
  busy.value = true;
  try {
    await action();
  } finally {
    busy.value = false;
  }
}

async function copyLink() {
  if (!props.parent.url) return;
  try {
    await copyText(props.parent.url);
    toastStore.showToast(t('downloads.row.linkCopied'), { style: 'success' });
  } catch {
    toastStore.showToast(t('downloads.row.copyFailed'), { style: 'error' });
  }
}

/** List keys: ↑/↓ move, →/← open/close (mirrored in RTL), Enter toggles, Space pauses or resumes all, Delete removes. */
function onKey(event: KeyboardEvent) {
  const row = event.currentTarget as HTMLElement;
  if (event.target !== row) return;
  const rtl = document.documentElement.dir === 'rtl';
  const forward = rtl ? 'ArrowLeft' : 'ArrowRight';
  const back = rtl ? 'ArrowRight' : 'ArrowLeft';
  if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
    event.preventDefault();
    const rows = Array.from(document.querySelectorAll<HTMLElement>(ROW_SELECTOR));
    rows[rows.indexOf(row) + (event.key === 'ArrowDown' ? 1 : -1)]?.focus();
  } else if (event.key === forward || event.key === back) {
    event.preventDefault();
    if ((event.key === forward) !== open.value) toggle();
  } else if (event.key === 'Enter') {
    event.preventDefault();
    toggle();
  } else if (event.key === ' ') {
    event.preventDefault();
    if (a.value.phase === 'downloading' || a.value.phase === 'waiting') void run(pauseAll);
    else if (a.value.phase === 'paused' || a.value.phase === 'cancelled') void run(resumeAll);
  } else if (event.key === 'Delete' || event.key === 'Backspace') {
    event.preventDefault();
    removeFromList();
  }
}

const foldCls = 'flex items-center border-b border-hairline pe-4 ps-[72px] relative before:absolute before:inset-y-0 before:start-[43px] before:w-px before:bg-hairline';
const foldBtn = 'min-h-7 cursor-pointer rounded-[6px] px-1 text-[12px] font-medium hover:underline underline-offset-2 focus-visible:outline-2 focus-visible:outline-[var(--nz-focus)]';
</script>
