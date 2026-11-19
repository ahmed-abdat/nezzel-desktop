<template>
  <!-- Search every setting, in every tab. Results replace the tab content while there is a query. -->
  <div role="search" class="mb-5">
    <div class="flex h-8 items-center gap-2 rounded-[7px] bg-foreground/[0.055] ps-2.5 pe-1 transition-[background-color,box-shadow] focus-within:bg-card focus-within:ring-2 focus-within:ring-ring/30">
      <Search class="size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
      <label :for="inputId" class="sr-only">{{ t('settings.search.label') }}</label>
      <Input
        :id="inputId"
        ref="field"
        v-model="query"
        variant="bare"
        type="search"
        role="combobox"
        aria-autocomplete="list"
        :aria-expanded="searching"
        :aria-controls="searching ? listId : undefined"
        :aria-activedescendant="activeHit ? optionId(activeHit.entry.id) : undefined"
        class="h-7 flex-1 text-[13px] placeholder:text-muted-foreground [&::-webkit-search-cancel-button]:hidden"
        :placeholder="t('settings.search.placeholder')"
        autocomplete="off"
        spellcheck="false"
        data-testid="settings-search"
        @keydown="onKeydown"
      />
      <Button v-if="query" size="icon-xs" variant="ghost" class="size-6 text-muted-foreground" :aria-label="t('settings.search.clear')" :title="t('settings.search.clear')" @click="clear(true)">
        <X aria-hidden="true" />
      </Button>
      <span v-else dir="ltr" class="flex shrink-0 gap-0.5 pe-1" aria-hidden="true"><Kbd v-for="key in findKeys" :key="key">{{ key }}</Kbd></span>
    </div>
    <span class="sr-only" aria-live="polite" aria-atomic="true">{{ searching ? t('settings.search.count', hits.length) : '' }}</span>

    <div v-if="searching && hits.length" :id="listId" role="listbox" :aria-label="t('settings.search.results')" class="mt-3">
      <div v-for="group in groups" :key="group.tab" role="group" :aria-labelledby="`${listId}-${group.tab}`" class="mt-4 first:mt-0">
        <div :id="`${listId}-${group.tab}`" class="mb-1 text-[12px] font-semibold text-muted-foreground">{{ group.label }}</div>
        <div
          v-for="hit in group.hits"
          :id="optionId(hit.entry.id)"
          :key="hit.entry.id"
          role="option"
          :aria-selected="hit.index === active"
          :data-result-id="hit.entry.id"
          class="-mx-2.5 flex min-h-11 cursor-default flex-col justify-center rounded-[6px] px-2.5 py-1.5"
          :class="hit.index === active && 'bg-primary-soft'"
          @mousemove="active = hit.index"
          @mousedown.prevent
          @click="choose(hit.entry)"
        >
          <span class="flex items-baseline gap-2">
            <span class="min-w-0 truncate text-[13px] text-foreground">{{ hit.label }}</span>
            <span v-if="hit.section" class="ms-auto shrink-0 text-[12px] text-muted-foreground">{{ hit.section }}</span>
          </span>
          <span v-if="hit.detail" class="truncate text-[12px] text-muted-foreground">{{ hit.detail }}</span>
        </div>
      </div>
    </div>

    <div v-else-if="searching" class="mt-6" data-testid="settings-search-empty">
      <p class="text-[13px] font-medium">{{ t('settings.search.emptyTitle', { query: query.trim() }) }}</p>
      <p class="mt-1 text-[12.5px] text-muted-foreground text-pretty">{{ t('settings.search.emptyHint') }}</p>
      <Button variant="link" class="-ms-2.5 mt-1" @click="report.openReport()">{{ t('settings.search.emptyReport') }}</Button>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, useId, useTemplateRef, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { Search, X } from '@lucide/vue';
import { Button } from '@/components/ui/button';
import { Input, type InputApi } from '@/components/ui/input';
import { Kbd } from '@/components/ui/kbd';
import { prepareDocs, searchSettings } from '../../helpers/settingsSearch';
import { usePlatform } from '../../composables/usePlatform';
import { useReportStore } from '../../stores/report';
import { buildSearchDocs, entryLabel, NO_VALUES, SETTINGS_INDEX, SETTINGS_TABS, type IndexTranslator, type SettingsIndexEntry } from './settingsIndex';

const query = defineModel<string>({ default: '' });
const emit = defineEmits<{ select: [entry: SettingsIndexEntry] }>();

const i18n = useI18n();
const { t, locale } = i18n;
const { isMac } = usePlatform();
const report = useReportStore();
const inputId = useId();
const listId = useId();
const field = useTemplateRef<InputApi>('field');

const entries = computed(() => SETTINGS_INDEX.filter(entry => !entry.macOnly || isMac.value));
const translator = computed<IndexTranslator>(() => ({
  text: (key, lang) => (lang ? t(key, NO_VALUES, { locale: lang }) : t(key, NO_VALUES)),
  has: (key, lang) => i18n.te(key, lang),
  locale: locale.value,
  isMac: isMac.value,
}));
/** Key names are symbols, not words: the same in every language. */
const findKeys = computed(() => (isMac.value ? ['⌘', 'F'] : ['Ctrl', 'F']));
const labelOf = (entry: SettingsIndexEntry) => entryLabel(entry, translator.value);
const docs = computed(() => prepareDocs(buildSearchDocs(entries.value, translator.value)));

const searching = computed(() => query.value.trim().length > 0);
const byId = computed(() => new Map(entries.value.map(entry => [entry.id, entry])));
const hits = computed(() => (searching.value ? searchSettings(docs.value, query.value) : [])
  .map(hit => byId.value.get(hit.id))
  .filter((entry): entry is SettingsIndexEntry => !!entry));

/** Grouped by tab, the tab with the best match first; `index` is the keyboard order (top to bottom on screen). */
const groups = computed(() => {
  let index = 0;
  const tabs = [...new Set(hits.value.map(entry => entry.tab))];
  return tabs
    .map(tab => ({ tab, label: t(SETTINGS_TABS.find(item => item.tab === tab)?.label ?? tab), entries: hits.value.filter(entry => entry.tab === tab) }))
    .map(group => ({
      ...group,
      hits: group.entries.map((entry) => {
        const parent = entry.parent ? byId.value.get(entry.parent) : undefined;
        return {
          entry,
          index: index++,
          label: labelOf(entry),
          section: entry.section ? t(entry.section) : '',
          detail: parent ? labelOf(parent) : entry.hint ? t(entry.hint, NO_VALUES) : '',
        };
      }),
    }));
});
const ordered = computed(() => groups.value.flatMap(group => group.hits));
const active = ref(0);
const activeHit = computed(() => ordered.value[active.value]);
watch(query, () => {
  active.value = 0;
});

const optionId = (id: string) => `${listId}-opt-${id}`;

function clear(refocus = false) {
  query.value = '';
  if (refocus) field.value?.focus();
}

function choose(entry: SettingsIndexEntry) {
  clear();
  emit('select', entry);
}

function onKeydown(event: KeyboardEvent) {
  const count = ordered.value.length;
  if (event.key === 'ArrowDown' && count) {
    event.preventDefault();
    active.value = (active.value + 1) % count;
  } else if (event.key === 'ArrowUp' && count) {
    event.preventDefault();
    active.value = (active.value - 1 + count) % count;
  } else if (event.key === 'Enter' && activeHit.value) {
    event.preventDefault();
    choose(activeHit.value.entry);
  } else if (event.key === 'Escape' && query.value) {
    event.preventDefault();
    event.stopPropagation();
    clear();
  }
}
watch(active, () => {
  const hit = activeHit.value;
  if (hit) document.getElementById(optionId(hit.entry.id))?.scrollIntoView?.({ block: 'nearest' });
});

/** Cmd/Ctrl+F jumps to the field while Settings is open, like a native preferences window. */
/** Another text field keeps its own Cmd/Ctrl+F (find in a textarea, a recorder); the search field itself re-selects. */
const inOtherField = (target: EventTarget | null) => target instanceof HTMLElement && target.id !== inputId
  && (target.isContentEditable || ['INPUT', 'TEXTAREA', 'SELECT'].includes(target.tagName));
function onWindowKey(event: KeyboardEvent) {
  if (event.defaultPrevented || inOtherField(event.target)) return;
  if ((event.metaKey || event.ctrlKey) && !event.altKey && !event.shiftKey && event.key.toLowerCase() === 'f') {
    event.preventDefault();
    field.value?.focus();
    field.value?.select();
  }
}
onMounted(() => window.addEventListener('keydown', onWindowKey));
onBeforeUnmount(() => window.removeEventListener('keydown', onWindowKey));
</script>
