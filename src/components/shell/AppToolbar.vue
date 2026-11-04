<template>
  <header
    class="relative flex h-[52px] shrink-0 items-center gap-3 border-b border-hairline px-4"
    :style="lightsInset ? { paddingLeft: `${lightsInset}px` } : undefined"
    data-tauri-drag-region
  >
    <sidebar-toggle v-if="showToggle" :collapsed="collapsed" @toggle="emit('toggleSidebar')" />
    <h1 class="shrink-0 truncate text-[15px] font-semibold" data-tauri-drag-region>{{ title }}</h1>
    <!-- v-show, not v-if: the field owns clipboard watching, which must keep running on Settings. -->
    <link-field v-show="!isSettings" ref="field" />
    <template v-if="!isSettings">
      <dropdown-menu>
        <dropdown-menu-trigger as-child>
          <Button size="icon-xs" variant="ghost" :aria-label="t('shell.view.title')">
            <sliders-horizontal />
          </Button>
        </dropdown-menu-trigger>
        <dropdown-menu-content class="w-56">
          <dropdown-menu-label>{{ t('shell.view.title') }}</dropdown-menu-label>
          <dropdown-menu-checkbox-item v-model="compactRows">{{ t('shell.view.compact') }}</dropdown-menu-checkbox-item>
          <dropdown-menu-checkbox-item v-model="showThumbnails">{{ t('shell.view.thumbnails') }}</dropdown-menu-checkbox-item>
        </dropdown-menu-content>
      </dropdown-menu>
    </template>
  </header>
</template>

<script setup lang="ts">
import { computed, ref } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import { useI18n } from 'vue-i18n';
import { SlidersHorizontal } from '@lucide/vue';
import { Button } from '@/components/ui/button';
import { DropdownMenu, DropdownMenuCheckboxItem, DropdownMenuContent, DropdownMenuLabel, DropdownMenuTrigger } from '@/components/ui/dropdown-menu';
import LinkField from '../downloads/LinkField.vue';
import SidebarToggle from './SidebarToggle.vue';
import { useUiPrefs } from '../../composables/useUiPrefs';
import { parseListView } from '../../helpers/listFilter';

/** `lightsInset`: px of physical left padding. The macOS overlay title bar puts the traffic lights at the window's
 *  physical left; pad clear of them when this toolbar touches that edge (RTL, or the sidebar collapsed under them).
 *  `showToggle`: the sidebar toggle opens the toolbar (Windows/Linux, and macOS RTL). */
defineProps<{ lightsInset: number; showToggle: boolean; collapsed: boolean }>();
const emit = defineEmits<{ toggleSidebar: [] }>();

const { t } = useI18n();
const route = useRoute();
const router = useRouter();
const { compactRows, showThumbnails } = useUiPrefs();
const field = ref<{ paste: () => Promise<void> } | null>(null);
defineExpose({ pasteField: async () => {
  if (isSettings.value) await router.push({ name: 'home' });
  await field.value?.paste();
} });

const isSettings = computed(() => String(route.name ?? '').startsWith('settings'));
const title = computed(() => {
  if (isSettings.value) return t('shell.nav.settings');
  if (route.name !== 'home') return t('shell.brand');
  const { type } = parseListView(route.query);
  return t(`shell.nav.${type}`);
});
</script>
