<template>
  <config-provider :dir="dir">
    <tooltip-provider :delay-duration="500">
      <a href="#nz-main" class="nz-skip">{{ t('shell.skipToContent') }}</a>
      <div class="relative flex h-full min-h-0 bg-background text-[13px] text-foreground" :class="dir === 'rtl' && 'text-[14px]'">
        <app-sidebar :collapsed="collapsed" :can-toggle="!narrowWindow" :inert="overlayOpen || undefined" @toggle="toggleSidebar" />
        <!-- macOS (LTR): the toggle sits in the title strip right after the traffic lights and stays put when the
             sidebar collapses. Physical `left` on purpose: the lights are at the physical left in every language. -->
        <div v-if="toggleInStrip" class="absolute top-3 z-40" style="left: 78px" :inert="overlayOpen || undefined">
          <sidebar-toggle :collapsed="collapsed" @toggle="toggleSidebar" />
        </div>

        <div class="flex min-w-0 flex-1 flex-col" :inert="overlayOpen || undefined">
          <app-toolbar ref="toolbar" :lights-inset="lightsInset" :show-toggle="!toggleInStrip && !narrowWindow" :collapsed="collapsed" @toggle-sidebar="toggleSidebar" />
          <main id="nz-main" class="relative min-h-0 flex-1 overflow-y-auto overflow-x-hidden outline-none" tabindex="-1">
            <router-view :key="$route.matched[1]?.path ?? $route.fullPath" />
            <the-updater />
            <Transition name="nz-drop">
              <div v-if="isDragging" class="pointer-events-none absolute inset-0 z-30 bg-background/85 p-3" aria-hidden="true">
                <div class="flex h-full flex-col items-center justify-center gap-3 rounded-xl border-2 border-dashed border-primary bg-primary-soft/50 text-[15px] font-semibold text-primary">
                  <arrow-down-to-line class="size-10" />
                  <span>{{ t('downloads.drop') }}</span>
                </div>
              </div>
            </Transition>
          </main>
          <app-status-bar />
        </div>

        <!-- Options inspector: pushes the list on wide windows, slides over it (with a scrim) on narrow ones. -->
        <Transition name="nz-fade">
          <div v-if="sheet.open && narrow" class="absolute inset-0 z-20 bg-[var(--scrim)]" aria-hidden="true" @click="sheet.close()" />
        </Transition>
        <Transition name="nz-inspector">
          <aside
            v-if="sheet.open"
            class="z-30 w-[320px] shrink-0 border-s border-hairline bg-background"
            :class="narrow && 'absolute inset-y-0 end-0 border-0 shadow-lg'"
          >
            <download-inspector :modal="narrow" />
          </aside>
        </Transition>
      </div>
    </tooltip-provider>
  </config-provider>
</template>

<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch, watchEffect } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import { useI18n } from 'vue-i18n';
import { ConfigProvider, TooltipProvider } from 'reka-ui';
import { useMediaQuery } from '../composables/useMediaQuery';
import { ArrowDownToLine } from '@lucide/vue';
import AppSidebar from '../components/shell/AppSidebar.vue';
import AppToolbar from '../components/shell/AppToolbar.vue';
import AppStatusBar from '../components/shell/AppStatusBar.vue';
import SidebarToggle from '../components/shell/SidebarToggle.vue';
import TheUpdater from '../components/TheUpdater.vue';
import DownloadInspector from '../components/downloads/DownloadInspector.vue';
import { useMediaGroupStore } from '../stores/media/group.ts';
import { useDragDropStore } from '../stores/dragDrop.ts';
import { useDownloadSheetStore } from '../stores/downloadSheet';
import { useUiPrefs } from '../composables/useUiPrefs';
import { usePlatform } from '../composables/usePlatform';
import { localeDirection } from '../i18n';
import { eventAccelerator, sameShortcut } from '../helpers/shortcut';
import { runShortcutAction } from '../tauri/listeners/shortcuts';
import { paneScrollTop } from '../helpers/paneScroll';

const { t, locale } = useI18n();
const route = useRoute();
const router = useRouter();
const groupStore = useMediaGroupStore();
const dragDropStore = useDragDropStore();
const sheet = useDownloadSheetStore();
const { sidebarCollapsed, inAppShortcuts } = useUiPrefs();
const { isMac } = usePlatform();
const isDragging = computed(() => dragDropStore.isOver);
const isMissingGroupRedirecting = ref(false);
const toolbar = ref<{ pasteField: () => Promise<void> } | null>(null);

const dir = computed(() => localeDirection(locale.value));
const narrowWindow = useMediaQuery('(max-width: 859px)');
const narrow = useMediaQuery('(max-width: 1023px)');
/** Small windows always get the icon sidebar; wide ones follow the user's toggle. */
const collapsed = computed(() => narrowWindow.value || sidebarCollapsed.value);
/** The inspector belongs to the download list: leaving it (Settings, power screens) closes it. */
watch(() => route.name, (name) => {
  if (name !== 'home') sheet.close();
});

/** Narrow windows: the inspector slides over the page, so the page behind it is inert (modal). */
const overlayOpen = computed(() => narrow.value && sheet.open);
/** macOS LTR: the toggle lives in the title strip after the traffic lights. Everywhere else it opens the toolbar. */
const toggleInStrip = computed(() => isMac.value && dir.value === 'ltr' && !narrowWindow.value);
/** Toolbar left padding clear of the traffic lights (+ the strip toggle) when the toolbar touches that edge. */
const lightsInset = computed<number>(() => {
  if (!isMac.value) return 0;
  if (dir.value === 'rtl') return 84;
  if (!collapsed.value) return 0;
  // Collapsed: toolbar starts at 56px; lights end ~70px, toggle spans 78-106px.
  return toggleInStrip.value ? 64 : 28;
});
function toggleSidebar() {
  if (!narrowWindow.value) sidebarCollapsed.value = !sidebarCollapsed.value;
}

watchEffect(() => {
  const requiresGroup = route.matched.some(record => record.meta.requiresGroup);
  if (!requiresGroup) return;

  const rawGroupId = route.params.groupId;
  const groupId = Array.isArray(rawGroupId) ? rawGroupId[0] : rawGroupId;
  if (!groupId || !groupStore.findGroupById(groupId)) {
    if (isMissingGroupRedirecting.value) return;
    isMissingGroupRedirecting.value = true;
    void router.replace({ name: 'home' }).finally(() => {
      isMissingGroupRedirecting.value = false;
    });
  }
});

/** App-wide keys: ⌘/Ctrl+V outside a text field drops the copied link into the field; ⌘/Ctrl+B (alias ⌘/Ctrl+\)
 *  toggles the sidebar;
 *  ⌘/Ctrl+, opens Settings; Esc closes the options panel. */
function onKey(event: KeyboardEvent) {
  const mod = event.metaKey || event.ctrlKey;
  const target = event.target as HTMLElement | null;
  const typing = !!target && (target.tagName === 'INPUT' || target.tagName === 'TEXTAREA' || target.tagName === 'SELECT' || target.isContentEditable);
  if (mod && !event.shiftKey && !event.altKey && (event.key === '\\' || event.key.toLowerCase() === 'b')) {
    event.preventDefault();
    toggleSidebar();
  } else if (mod && event.key === ',') {
    event.preventDefault();
    void router.push({ name: 'settings.general' });
  } else if (mod && event.key.toLowerCase() === 'v' && !typing) {
    // A paste dispatched to the old target need not follow a keydown focus change.
    event.preventDefault();
    void toolbar.value?.pasteField();
  } else if (event.key === 'Escape' && sheet.open) {
    sheet.close();
  } else if (!mod && !event.altKey && !typing && !event.defaultPrevented) {
    scrollPaneByKey(event, target);
  } else if ((mod || event.altKey) && !typing) {
    // In-app queue shortcuts (Settings > Keyboard shortcuts), only while Nezzel is focused.
    const pressed = eventAccelerator(event, isMac.value);
    if (!pressed) return;
    if (inAppShortcuts.downloadAll.value && sameShortcut(pressed, inAppShortcuts.downloadAll.value)) {
      event.preventDefault();
      void runShortcutAction('download_all');
    } else if (inAppShortcuts.addAndDownload.value && sameShortcut(pressed, inAppShortcuts.addAndDownload.value)) {
      event.preventDefault();
      void runShortcutAction('media_add_and_download');
    }
  }
}
/** Scroll keys pressed outside the content pane still scroll it (Settings was unreachable by keyboard on Linux). */
function scrollPaneByKey(event: KeyboardEvent, target: HTMLElement | null) {
  const pane = document.getElementById('nz-main');
  if (!pane) return;
  const onPage = !target || target === document.body || target === document.documentElement;
  const native = !onPage && (pane.contains(target) || !!target.closest('[role=dialog],[role=menu],[role=listbox],[role=slider],[role=radiogroup],[role=tablist]'));
  const next = paneScrollTop(event.key, event.shiftKey, onPage ? 'page' : native ? 'native' : 'shell', pane);
  if (next === null) return;
  event.preventDefault();
  pane.scrollTop = next;
}
onMounted(() => window.addEventListener('keydown', onKey));
onBeforeUnmount(() => window.removeEventListener('keydown', onKey));
</script>

<style scoped>
.nz-skip {
  position: absolute;
  inset-inline-start: 0.5rem;
  top: -3rem;
  z-index: 100;
  padding: 0.375rem 0.75rem;
  border-radius: 6px;
  background: var(--primary);
  color: var(--primary-foreground);
  font-weight: 600;
}
.nz-skip:focus { top: 0.5rem; }
.nz-drop-enter-active, .nz-drop-leave-active, .nz-fade-enter-active, .nz-fade-leave-active { transition: opacity 150ms ease; }
.nz-drop-enter-from, .nz-drop-leave-to, .nz-fade-enter-from, .nz-fade-leave-to { opacity: 0; }
.nz-inspector-enter-active, .nz-inspector-leave-active { transition: opacity 180ms ease, translate 180ms cubic-bezier(0.2, 0.8, 0.2, 1); }
.nz-inspector-enter-from, .nz-inspector-leave-to { opacity: 0; translate: 16px 0; }
[dir="rtl"] .nz-inspector-enter-from, [dir="rtl"] .nz-inspector-leave-to { translate: -16px 0; }
@media (prefers-reduced-motion: reduce) {
  .nz-inspector-enter-active, .nz-inspector-leave-active, .nz-drop-enter-active, .nz-drop-leave-active { transition: none; }
}
</style>

<style>
#app { display: block; }
</style>
