<template>
  <context-menu>
    <context-menu-trigger as-child>
      <li
        class="row group relative flex items-start gap-3 border-b border-hairline pe-4 ps-8 outline-none focus-visible:outline-solid focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-[var(--nz-focus)] animate-in fade-in-0 slide-in-from-top-1 duration-200"
        :class="[tint, compactRows ? 'py-1.5' : 'py-2.5']"
        :aria-selected="selected"
        :data-phase="phase"
        :aria-busy="phase === 'reading' || phase === 'downloading' ? 'true' : undefined"
        tabindex="0"
        :aria-keyshortcuts="canOpen ? 'Enter' : undefined"
        @keydown="onKey"
        @click="onRowClick"
        @dblclick="onDblClick"
      >
        <!-- Selection: shows on hover or focus, and stays while this row or any row is selected. -->
        <Checkbox
          class="absolute start-3 top-3 z-[1] transition-opacity"
          :class="selected || anySelected ? 'opacity-100' : 'opacity-0 group-hover:opacity-100 group-focus-within:opacity-100'"
          :model-value="selected"
          :aria-label="t('downloads.selection.select', { title })"
          @click.stop="onBoxClick"
        />
        <figure
          v-if="showThumbnails"
          class="relative m-0 grid shrink-0 place-items-center overflow-hidden rounded-[5px] bg-muted text-muted-foreground ring-1 ring-black/5 dark:ring-white/5"
          :class="[compactRows ? 'h-[27px] w-12' : 'h-10 w-[72px]', phase === 'reading' && !thumbUrl && 'animate-pulse', canOpen && 'cursor-pointer']"
          @click="openFromRow"
        >
          <file-type-icon v-if="fileKind" :category="fileKind" :class="compactRows ? 'size-4' : 'size-5'" />
          <img
            v-else-if="thumbUrl"
            :src="thumbUrl"
            :alt="t('media.view.thumbnailAlt', { title: group.title ?? '' })"
            class="size-full object-cover"
            loading="lazy"
            @error="thumbFailed = true"
          />
          <!-- No picture: what it is (video, audio, photos) and where it came from, never a broken box. -->
          <template v-else-if="phase !== 'reading'">
            <component :is="kindIcon" :class="compactRows ? 'size-3.5' : 'size-4'" aria-hidden="true" />
            <span
              v-if="siteInitial && !compactRows"
              class="absolute bottom-0.5 end-0.5 grid size-3.5 place-items-center rounded-[3px] bg-background/90 text-[9px] font-semibold leading-none text-foreground/70"
              aria-hidden="true"
            >{{ siteInitial }}</span>
          </template>
        </figure>

        <div class="min-w-0 flex-1 @min-[1040px]/list:min-h-0 @min-[1040px]/list:self-center" :class="!compactRows && 'min-h-10'">
          <Tooltip :content="title" side="top" :disabled="!titleClipped">
            <!-- The flex line packs at the UI start; the title is its own dir=auto box, so a Latin title in Arabic UI cuts at its own end. -->
            <p class="flex min-w-0 font-medium leading-5" @pointerenter="measureTitle">
              <span :dir="group.title ? 'auto' : 'ltr'" data-clip class="min-w-0 truncate" :class="canOpen && 'cursor-pointer decoration-foreground/40 underline-offset-2 hover:underline'" @click="openFromRow">{{ title }}</span>
            </p>
          </Tooltip>

          <login-needed v-if="loginCard" class="mt-1" :card="loginCard" :site="site" @retry="retry" />
          <p v-else-if="phase === 'error'" class="mt-0.5 flex items-start gap-1.5 text-[12.5px] font-medium text-destructive-foreground">
            <triangle-alert class="mt-0.5 size-3.5 shrink-0" aria-hidden="true" />
            <!-- What happened, then what to do (muted): every runner error carries both. -->
            <i18n-t v-if="errorDisplay.action" keypath="errors.rowLine" tag="span" scope="global" class="line-clamp-2 text-pretty" data-testid="row-error">
              <template #what>{{ errorDisplay.shortMessage }}</template>
              <template #action><span class="font-normal text-foreground/85">{{ errorDisplay.action }}</span></template>
            </i18n-t>
            <span v-else class="line-clamp-2 text-pretty" data-testid="row-error">{{ errorDisplay.shortMessage }}</span>
          </p>
          <div v-else-if="showProgress || phase === 'reading'" class="mt-1 flex min-w-0 items-center gap-2.5 text-[12px] text-muted-foreground">
            <nz-progress :value="phase === 'reading' ? null : percent" :muted="phase === 'paused'" class="max-w-[180px] flex-[1_1_120px]" />
            <span class="flex min-w-0 items-center gap-1.5 truncate">
              <template v-for="(part, index) in metaLine.slice(1)" :key="index">
                <span v-if="index > 0" class="nz-sep" aria-hidden="true" />
                <bdi class="tnum">{{ part }}</bdi>
              </template>
            </span>
          </div>
          <p v-else class="mt-0.5 flex min-w-0 items-center gap-1.5 truncate text-[12px] text-muted-foreground" :class="(phase === 'done' || phase === 'ready') && '@min-[1040px]/list:hidden'">
            <template v-for="(part, index) in metaLine" :key="index">
              <span v-if="index > 0" class="nz-sep" aria-hidden="true" />
              <bdi>{{ part }}</bdi>
            </template>
          </p>

          <LoginProfile :group-id="group.id" :show-picker="false" :tone="loginCard?.kind === 'needed' ? 'amber' : phase === 'error' ? 'error' : 'muted'" :disabled="phase === 'reading' || phase === 'downloading' || loginCard?.kind === 'needed'" class="mt-0.5" />

          <div v-if="phase === 'playlist'" class="mt-2">
            <playlist-selection-step :group="group" />
          </div>
        </div>

        <!-- Finder-style columns when the list is wide (labels in HomeView). Unknown yet: a skeleton while
             reading, a dash after, never a blank hole. -->
        <span v-for="cell in cells" :key="cell.key" :class="[colCls, cell.cls]">
          <span v-if="cell.value"><bdi :class="cell.num && 'tnum'">{{ cell.value }}</bdi></span>
          <span v-else-if="phase === 'reading'" class="inline-block h-2.5 w-10 rounded-full bg-foreground/[0.12] align-middle animate-pulse motion-reduce:animate-none" aria-hidden="true" />
          <span v-else class="text-muted-foreground/45" aria-hidden="true">{{ EMPTY_CELL }}</span>
        </span>

        <!-- Trailing actions: one column, same inset on every row. -->
        <div class="flex shrink-0 items-center gap-1 self-center @min-[1040px]/list:w-[200px] @min-[1040px]/list:justify-end">
          <template v-if="phase === 'ready'">
            <Button variant="outline" @click="sheet.show(group.id)">{{ t('downloads.row.options') }}</Button>
            <Button @click="download">{{ t('downloads.row.download') }}</Button>
          </template>
          <template v-else-if="phase === 'error' && !loginOwnsAction">
            <Button v-if="errorIsFilter" variant="ghost" @click="router.push({ name: 'input-filters' })">{{ t('media.steps.error.filters') }}</Button>
            <!-- A permanent refusal (removed, protected, region…) keeps Try again, but quiet: retrying rarely helps. -->
            <Button :variant="errorIsPermanent ? 'ghost' : 'outline'" @click="recover"><Film v-if="recoveryIsSwitch" /><rotate-cw v-else />{{ recoveryLabel }}</Button>
          </template>
          <!-- Merging or converting: nothing to pause. Cancel takes the same slot, so the row doesn't shift. -->
          <tooltip v-else-if="processing && loginCard?.kind !== 'needed'" :content="t('downloads.row.cancel')">
            <Button
              size="icon-xs"
              variant="outline"
              class="rounded-full"
              :aria-label="t('downloads.row.cancel')"
              @click="remove"
            >
              <x />
            </Button>
          </tooltip>
          <tooltip v-else-if="(phase === 'downloading' || phase === 'paused') && loginCard?.kind !== 'needed'" :content="phase === 'paused' ? t('downloads.row.resume') : t('downloads.row.pause')">
            <Button
              size="icon-xs"
              variant="outline"
              class="rounded-full"
              :aria-label="phase === 'paused' ? t('downloads.row.resume') : t('downloads.row.pause')"
              @click="phase === 'paused' ? download() : pause()"
            >
              <Play v-if="phase === 'paused'" /><Pause v-else />
            </Button>
          </tooltip>
          <template v-else-if="phase === 'done'">
            <!-- Primary action stays visible (first-time, touch and magnifier users); the folder icon waits for hover. -->
            <tooltip :content="t('downloads.row.noLocation')" :disabled="!!(destination)">
              <Button
                variant="ghost"
                class="text-primary hover:text-primary"
                :disabled="!destination"
                @click="openFile"
              >
                {{ group.isCombined ? t('downloads.row.openFirst') : revealsFile ? t('downloads.row.show') : t('downloads.row.open') }}
              </Button>
            </tooltip>
            <tooltip v-if="!revealsFile" :content="t('downloads.row.show')">
              <Button
                size="icon-xs"
                variant="ghost"
                class="opacity-0 transition-opacity group-hover:opacity-100 group-focus-within:opacity-100 focus-visible:opacity-100"
                :disabled="!destination"
                :aria-label="t('downloads.row.show')"
                @click="revealFile"
              >
                <folder-open />
              </Button>
            </tooltip>
          </template>

          <!-- Reading for a minute: offer a fresh start next to Remove. -->
          <Button v-if="phase === 'reading' && readingStuck" variant="outline" @click="retry"><rotate-cw />{{ t('downloads.row.retry') }}</Button>
          <tooltip v-if="phase === 'reading' || phase === 'playlist'" :content="t('downloads.row.remove')">
            <Button
              size="icon-xs"
              variant="ghost"
              class="rounded-full text-muted-foreground"
              :aria-label="t('downloads.row.remove')"
              @click="remove"
            >
              <x />
            </Button>
          </tooltip>
          <dropdown-menu v-else>
            <dropdown-menu-trigger as-child>
              <Button size="icon-xs" variant="ghost" class="rounded-full text-muted-foreground" :aria-label="t('downloads.row.more')">
                <ellipsis />
              </Button>
            </dropdown-menu-trigger>
            <dropdown-menu-content>
              <template v-for="(list, i) in menuGroups" :key="i">
                <dropdown-menu-separator v-if="i > 0" />
                <dropdown-menu-item v-for="item in list" :key="item.key" :disabled="item.disabled" :danger="item.danger" @select="item.run()">
                  <component :is="item.icon" aria-hidden="true" />{{ item.label }}
                </dropdown-menu-item>
              </template>
            </dropdown-menu-content>
          </dropdown-menu>
        </div>

        <div
          v-if="showProgress && percent != null"
          class="sr-only"
          role="progressbar"
          :aria-valuenow="Math.round(percent)"
          aria-valuemin="0"
          aria-valuemax="100"
          :aria-label="title"
        />
      </li>
    </context-menu-trigger>
    <context-menu-content>
      <template v-for="(list, i) in menuGroups" :key="i">
        <context-menu-separator v-if="i > 0" />
        <context-menu-item v-for="item in list" :key="item.key" :disabled="item.disabled" :danger="item.danger" @select="item.run()">
          <component :is="item.icon" aria-hidden="true" />{{ item.label }}
        </context-menu-item>
      </template>
    </context-menu-content>
  </context-menu>
</template>

<script setup lang="ts">
import LoginProfile from './LoginProfile.vue';
import FileTypeIcon from './FileTypeIcon.vue';
import { revealsInFolder } from '../../helpers/fileCategory';
import { ROW_SELECTOR } from '../../helpers/domIds';
import { hostOf } from '../../helpers/siteName';
import { isPermanentError } from '../../helpers/errorOutcome';
import { computed, nextTick, ref, toRef, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { Copy, Ellipsis, ExternalLink, File, Film, FolderOpen, Image as ImageIcon, Images, Info, ListX, MessageSquareWarning, Music, Pause, Play, RotateCw, Trash2, TriangleAlert, Wrench, X } from '@lucide/vue';
import type { RouteLocationRaw } from 'vue-router';
import { useRouter } from 'vue-router';
import Tooltip from '@/components/ui/tooltip/Tooltip.vue';
import { useOverflowTip } from '../../composables/useOverflowTip';
import type { Group } from '../../tauri/types/group';
import { Button } from '@/components/ui/button';
import { Checkbox } from '@/components/ui/checkbox';
import NzProgress from '@/components/ui/progress/Progress.vue';
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger } from '@/components/ui/dropdown-menu';
import { ContextMenu, ContextMenuContent, ContextMenuItem, ContextMenuSeparator, ContextMenuTrigger } from '@/components/ui/context-menu';
import { useUiPrefs } from '../../composables/useUiPrefs';
import LoginNeeded from './LoginNeeded.vue';
import PlaylistSelectionStep from '../media-card/steps/PlaylistSelectionStep.vue';
import { useDownloadRow } from '../../composables/useDownloadRow';
import { useDownloadActions } from '../../composables/useDownloadActions';
import { usePlatform } from '../../composables/usePlatform';
import { useDownloadSelection } from '../../composables/useDownloadSelection';
import { useDiagnostic } from '../../composables/useDiagnostic';
import { useDownloadSheetStore } from '../../stores/downloadSheet';
import { useLoginStore, type LoginCard } from '../../stores/login';
import { useReportStore } from '../../stores/report';
import { useMediaDiagnosticsStore } from '../../stores/media/diagnostics';
import { downloadReportContext } from '../../helpers/reportContext';

defineOptions({ inheritAttrs: false });
const props = defineProps<{ group: Group }>();
const router = useRouter();
const { compactRows, showThumbnails } = useUiPrefs();
const group = toRef(props, 'group');

const { t } = useI18n();
const sheet = useDownloadSheetStore();
const colCls = 'hidden shrink-0 self-center truncate text-[12.5px] text-muted-foreground @min-[1040px]/list:block';
const { phase, site, percent, metaLine, columns, needsLogin, lastDiagnostic, lastFatal, destination, fileKind, readingStuck, processing } = useDownloadRow(group);
/** Not known (yet): a typographic placeholder, not copy, so it needs no translation. */
const EMPTY_CELL = '\u2014';
const cells = computed(() => [
  { key: 'site', value: site.value, cls: 'w-32', num: false },
  { key: 'quality', value: columns.value.quality, cls: 'w-24', num: false },
  { key: 'size', value: columns.value.size, cls: 'w-20 text-end', num: true },
  { key: 'length', value: columns.value.length, cls: 'w-16 text-end', num: true },
]);
const { download, pause, retry, recover, recoveryLabel, recoveryIsSwitch, remove, openFile, revealFile, openPage, copyLink, deleteFile, canTrash } = useDownloadActions(group);
/** Finished with a known file: double-click, thumbnail, title and Enter open it. Nothing else opens. */
/** A file that only reveals its folder (installer, archive, unknown): the primary action says so. */
const revealsFile = computed(() => group.value.mediaKind === 'file' && revealsInFolder(destination.value));
const canOpen = computed(() => phase.value === 'done' && !!destination.value);
const loginStore = useLoginStore();
/** Engine request for this row, or a site refusal the engine did not ask about (fixed browser / never). */
const loginCard = computed<LoginCard | null>(() => loginStore.cardFor(group.value.id)
  ?? (needsLogin.value ? { kind: 'failed', failure: { groupId: group.value.id, host: hostOf(group.value.url), reason: 'refused' } } : null));

/** The login card carries its own button (log in, try another browser, log in again): no second Retry. */
const loginOwnsAction = computed(() => loginCard.value?.kind === 'needed'
  || (loginCard.value?.kind === 'failed' && ['exhausted', 'dismissed', 'expired'].includes(loginCard.value.failure.reason)));

const title = computed(() => group.value.title ?? group.value.url ?? '');
const showProgress = computed(() => phase.value === 'downloading' || phase.value === 'paused');

const diagnostic = computed<MediaDiagnostic>(() => {
  const base: MediaDiagnostic = {
    id: 'unknown', code: 'unknown', groupId: group.value.id, raw: 'unknown', message: 'unknown', level: 'error', component: null, timestamp: 0,
  };
  const fatal = lastFatal.value;
  if (fatal?.internal) {
    return { ...base, id: fatal.id, message: fatal.message, component: 'runner', raw: fatal.details ?? 'unknown', timestamp: fatal.timestamp };
  }
  if (lastDiagnostic.value) return lastDiagnostic.value;
  if (fatal?.exitCode === 101) {
    return { ...base, id: fatal.id, message: fatal.message, code: 'rejectedByFilter', component: 'runner', raw: fatal.details ?? '', timestamp: fatal.timestamp };
  }
  return base;
});
const { diagnosticDisplay: errorDisplay } = useDiagnostic(diagnostic, true);
const errorIsFilter = computed(() => diagnostic.value.code === 'rejectedByFilter');
const errorIsPermanent = computed(() => isPermanentError(diagnostic.value.code));

interface RowAction { key: string; label: string; icon: unknown; run: () => unknown; danger?: boolean; disabled?: boolean }
const go = (to: RouteLocationRaw) => () => router.push(to);
/** One list feeds both the ⋯ menu and the right-click menu. */
const menuGroups = computed<RowAction[][]>(() => {
  const primary: RowAction[] = [];
  if (phase.value === 'done') {
    if (!revealsFile.value) primary.push({ key: 'open', label: t('downloads.row.open'), icon: File, run: openFile, disabled: !destination.value });
    primary.push({ key: 'show', label: t('downloads.row.show'), icon: FolderOpen, run: revealFile, disabled: !destination.value });
  }
  if (phase.value === 'downloading' && !processing.value) primary.push({ key: 'pause', label: t('downloads.row.pause'), icon: Pause, run: pause });
  if (phase.value === 'paused') primary.push({ key: 'resume', label: t('downloads.row.resume'), icon: Play, run: download });
  if (phase.value === 'error') primary.push({ key: 'retry', label: recoveryLabel.value, icon: recoveryIsSwitch.value ? Film : RotateCw, run: recover });
  const more: RowAction[] = [
    { key: 'copy', label: t('downloads.row.copyLink'), icon: Copy, run: copyLink, disabled: !group.value.url },
    { key: 'page', label: t('downloads.row.openPage'), icon: ExternalLink, run: openPage, disabled: !group.value.url },
    ...(phase.value === 'ready' ? [{ key: 'per', label: t('downloads.row.perDownload'), icon: Wrench, run: go({ name: 'preferences.quality', params: { groupId: group.value.id } }) }] : []),
    { key: 'details', label: t('downloads.row.details'), icon: Info, run: go({ name: 'group.metadata', params: { groupId: group.value.id } }) },
    ...(phase.value === 'error' ? [{ key: 'report', label: t('report.rowAction'), icon: MessageSquareWarning, run: reportProblem }] : []),
  ];
  const removal: RowAction[] = [{ key: 'remove', label: t('downloads.row.removeFromList'), icon: ListX, run: remove }];
  const destructive: RowAction[] = phase.value === 'done' && destination.value && canTrash.value
    ? [{ key: 'delete', label: t('downloads.row.deleteFile'), icon: Trash2, run: deleteFile, danger: true }]
    : [];
  return [primary, more, removal, destructive].filter(list => list.length > 0);
});

const reportStore = useReportStore();
const diagnosticsStore = useMediaDiagnosticsStore();
function reportProblem() {
  reportStore.openReport({
    context: downloadReportContext(diagnostic.value.code, lastFatal.value?.exitCode, diagnosticsStore.findAttemptsByGroupId(group.value.id)),
    url: group.value.url ?? null,
    extractor: group.value.extractor ?? null,
  });
}

const tint = computed(() => (selected.value ? 'bg-primary-soft' : loginCard.value?.kind === 'needed' ? 'bg-amber-soft' : phase.value === 'error' ? 'bg-destructive-soft' : 'hover:bg-foreground/[0.025] focus-visible:bg-primary-soft'));

/** Native list keys: ↑/↓ move between rows, Enter opens (done) or starts (ready), Delete removes, Space pauses/resumes. */
function onKey(event: KeyboardEvent) {
  const row = event.currentTarget as HTMLElement;
  if (event.target !== row) return;
  if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
    event.preventDefault();
    const rows = Array.from(document.querySelectorAll<HTMLElement>(ROW_SELECTOR));
    rows[rows.indexOf(row) + (event.key === 'ArrowDown' ? 1 : -1)]?.focus();
  } else if (event.key === 'Enter') {
    if (canOpen.value) openFromRow();
    else if (phase.value === 'ready') void download();
  } else if (event.key === 'Delete' || event.key === 'Backspace') {
    event.preventDefault();
    // Keep the keyboard in the list: focus moves to the next row (or the previous one at the end).
    const rows = Array.from(document.querySelectorAll<HTMLElement>(ROW_SELECTOR));
    const index = rows.indexOf(row);
    const next = rows[index + 1] ?? rows[index - 1];
    void remove();
    void nextTick(() => next?.focus());
  } else if (event.key === ' ') {
    event.preventDefault();
    if (phase.value === 'downloading' && !processing.value) void pause();
    else if (phase.value === 'paused') void download();
  }
}

const { isMac } = usePlatform();
const selection = useDownloadSelection();
const selected = computed(() => selection.isSelected(group.value.id));
const anySelected = computed(() => selection.count.value > 0);

/** Cmd (mac) / Ctrl click toggles this row, Shift+click selects a range. Plain clicks do what they always did. */
function onRowClick(event: MouseEvent) {
  if ((event.target as HTMLElement).closest('button, a, input, [role="menu"]')) return;
  const toggleKey = isMac.value ? event.metaKey : event.ctrlKey;
  if (event.shiftKey) selection.extend(group.value.id);
  else if (toggleKey) selection.toggle(group.value.id);
}

function onBoxClick(event: MouseEvent) {
  if (event.shiftKey) selection.extend(group.value.id);
  else selection.toggle(group.value.id);
}

/** One open per gesture: a double-click on the title would otherwise fire click, click and dblclick. */
let lastOpen = 0;
function openFromRow() {
  if (!canOpen.value) return;
  const now = Date.now();
  if (now - lastOpen < 600) return;
  lastOpen = now;
  void openFile();
}

function onDblClick(event: MouseEvent) {
  // Buttons and menus inside the row keep their own double-click.
  if ((event.target as HTMLElement).closest('button, a, input, [role="menu"]')) return;
  openFromRow();
}

const { clipped: titleClipped, measure: measureTitle } = useOverflowTip();

const thumbFailed = ref(false);
watch(() => group.value.thumbnail, () => {
  thumbFailed.value = false;
});
const thumbUrl = computed(() => (!thumbFailed.value && group.value.thumbnail) || null);
const kindIcon = computed(() => {
  const kind = group.value.mediaKind;
  if (kind === 'audio') return Music;
  if (kind === 'image') return ImageIcon;
  if (kind === 'carousel' || kind === 'mixed') return Images;
  return Film;
});
/** First letter of the site name, as a tiny badge on the fallback thumbnail. */
const siteInitial = computed(() => (site.value ? [...site.value][0]?.toLocaleUpperCase() : ''));
</script>
