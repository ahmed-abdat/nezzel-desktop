<template>
  <tooltip :content="`${label}  ${shortcut}`">
    <Button
      size="icon-xs"
      variant="ghost"
      class="text-muted-foreground hover:text-foreground"
      :aria-label="label"
      :aria-expanded="!collapsed"
      :aria-keyshortcuts="ariaKeyShortcuts(BUILT_IN_KEYS.sidebar)"
      @click="emit('toggle')"
    >
      <panel-left class="rtl:rotate-180" aria-hidden="true" />
    </Button>
  </tooltip>
</template>

<script setup lang="ts">
// Sidebar toggle (shadcn SidebarTrigger). Lives in the title strip after the macOS traffic lights, or at the
// toolbar's start elsewhere, so it never moves under the cursor when the sidebar collapses.
import { computed } from 'vue';
import { useI18n } from 'vue-i18n';
import { PanelLeft } from '@lucide/vue';
import { Button } from '@/components/ui/button';
import Tooltip from '@/components/ui/tooltip/Tooltip.vue';
import { usePlatform } from '../../composables/usePlatform';
import { ariaKeyShortcuts, BUILT_IN_KEYS, shortcutLabel } from '../../helpers/shortcut';

const props = defineProps<{ collapsed: boolean }>();
const emit = defineEmits<{ toggle: [] }>();
const { t } = useI18n();
const { isMac } = usePlatform();
const label = computed(() => (props.collapsed ? t('shell.sidebar.show') : t('shell.sidebar.hide')));
const shortcut = computed(() => shortcutLabel(BUILT_IN_KEYS.sidebar, isMac.value));
</script>
