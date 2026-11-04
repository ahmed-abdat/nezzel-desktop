<script setup lang="ts">
// Date field: a 28px trigger showing the date in the app locale, a popover calendar, and a clear
// action. v-model is the `YYYY-MM-DD` string a native date input used to hold (or null).
import type { HTMLAttributes } from 'vue';
import { computed, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import { CalendarDays, X } from '@lucide/vue';
import type { DateValue } from 'reka-ui';
import { cn } from 'cn';
import { Button } from '../button';
import { Calendar } from '../calendar';
import { Popover, PopoverContent, PopoverTrigger } from '../popover';
import { calendarDateToIso, dateLocale, isoToCalendarDate } from './value';

const props = defineProps<{ id?: string; class?: HTMLAttributes['class']; placeholder?: string; disabled?: boolean }>();
const model = defineModel<string | null>({ default: null });

const { t, locale } = useI18n();
const open = ref(false);

const intlLocale = computed(() => dateLocale(locale.value));
const selected = computed(() => isoToCalendarDate(model.value));
const label = computed(() => {
  if (!selected.value) return null;
  const { year, month, day } = selected.value;
  return new Intl.DateTimeFormat(intlLocale.value, { dateStyle: 'long', timeZone: 'UTC' })
    .format(new Date(Date.UTC(year, month - 1, day)));
});

function pick(value: DateValue | undefined) {
  if (!value) return;
  model.value = calendarDateToIso(value);
  open.value = false;
}

function clear() {
  model.value = null;
  open.value = false;
}
</script>

<template>
  <Popover v-model:open="open">
    <PopoverTrigger as-child :disabled="disabled">
      <button
        :id="id"
        type="button"
        data-slot="date-picker-trigger"
        :aria-label="label ? undefined : (placeholder ?? t('datePicker.placeholder'))"
        :class="cn(
          'inline-flex h-7 w-48 items-center gap-2 rounded-[6px] border border-input bg-card px-2.5 text-start text-[13px] text-foreground shadow-xs outline-none transition-colors hover:border-foreground/25 focus-visible:ring-2 focus-visible:ring-ring/30 disabled:cursor-not-allowed disabled:opacity-50 data-[state=open]:ring-2 data-[state=open]:ring-ring/30',
          props.class,
        )"
      >
        <CalendarDays class="size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
        <span class="min-w-0 flex-1 truncate tabular-nums" :class="!label && 'text-muted-foreground/70'">
          {{ label ?? placeholder ?? t('datePicker.placeholder') }}
        </span>
      </button>
    </PopoverTrigger>
    <PopoverContent align="start" class="w-auto p-3">
      <Calendar
        :model-value="selected"
        :locale="intlLocale"
        :prev-label="t('datePicker.prevMonth')"
        :next-label="t('datePicker.nextMonth')"
        :month-label="t('datePicker.month')"
        :year-label="t('datePicker.year')"
        initial-focus
        @update:model-value="pick"
      />
      <div v-if="selected" class="mt-2 flex justify-end border-t border-hairline pt-2">
        <Button variant="ghost" size="xs" @click="clear">
          <X aria-hidden="true" />
          {{ t('datePicker.clear') }}
        </Button>
      </div>
    </PopoverContent>
  </Popover>
</template>
