<script setup lang="ts">
// shadcn-vue Calendar (reka-ui CalendarRoot), trimmed to the desktop kit: native month/year
// selects in the header so far-back dates are two picks away, chevrons that mirror in RTL.
// Locale drives month/day names and the first weekday; pass `xx-u-nu-latn` for Latin digits.
import type { HTMLAttributes } from 'vue';
import { computed, shallowRef, watch } from 'vue';
import { getLocalTimeZone, today } from '@internationalized/date';
import {
  CalendarRoot,
  useDateFormatter,
  useForwardPropsEmits,
  type CalendarRootEmits,
  type CalendarRootProps,
  type DateValue,
} from 'reka-ui';
import { createYear, createYearRange, toDate } from 'reka-ui/date';
import { cn } from 'cn';
import { NativeSelect } from '../native-select';
import {
  CalendarCell,
  CalendarCellTrigger,
  CalendarGrid,
  CalendarGridBody,
  CalendarGridHead,
  CalendarGridRow,
  CalendarHeadCell,
  CalendarHeader,
  CalendarNextButton,
  CalendarPrevButton,
} from '.';

const props = withDefaults(defineProps<CalendarRootProps & {
  class?: HTMLAttributes['class'];
  prevLabel?: string;
  nextLabel?: string;
  monthLabel?: string;
  yearLabel?: string;
}>(), { modelValue: undefined });
const emits = defineEmits<CalendarRootEmits>();

const delegated = computed(() => {
  const rest = { ...props };
  for (const key of ['class', 'placeholder', 'prevLabel', 'nextLabel', 'monthLabel', 'yearLabel'] as const) delete rest[key];
  return rest;
});
const forwarded = useForwardPropsEmits(delegated, emits);

const initial = () => (props.placeholder ?? props.modelValue ?? props.defaultPlaceholder ?? today(getLocalTimeZone())) as DateValue;
const view = shallowRef<DateValue>(initial());
watch(() => props.placeholder, (value) => {
  if (value) view.value = value;
});
watch(view, value => emits('update:placeholder', value));

const formatter = useDateFormatter(props.locale ?? 'en');

const months = computed(() => createYear({ dateObj: view.value }).map(month => ({
  value: month.month,
  label: formatter.custom(toDate(month), { month: 'long' }),
})));
const years = computed(() => {
  const now = today(getLocalTimeZone());
  const start = props.minValue ?? now.cycle('year', -30);
  const end = props.maxValue ?? now.cycle('year', 1);
  const range = createYearRange({ start, end });
  // Keep the shown year selectable even when it is outside the default window.
  if (!range.some(year => year.year === view.value.year)) range.push(view.value);
  return range
    .sort((a, b) => b.year - a.year)
    .map(year => ({ value: year.year, label: formatter.custom(toDate(year), { year: 'numeric' }) }));
});

const month = computed<number>({
  get: () => view.value.month,
  set: (value) => { view.value = view.value.set({ month: Number(value) }); },
});
const year = computed<number>({
  get: () => view.value.year,
  set: (value) => { view.value = view.value.set({ year: Number(value) }); },
});
</script>

<template>
  <CalendarRoot
    v-slot="{ grid, weekDays }"
    v-bind="forwarded"
    v-model:placeholder="view"
    data-slot="calendar"
    :class="cn('w-fit', props.class)"
  >
    <CalendarHeader>
      <CalendarPrevButton :aria-label="prevLabel" />
      <div class="flex min-w-0 items-center gap-1">
        <NativeSelect v-model="month" :aria-label="monthLabel" class="min-w-0">
          <option v-for="option in months" :key="option.value" :value="option.value">{{ option.label }}</option>
        </NativeSelect>
        <NativeSelect v-model="year" :aria-label="yearLabel" class="tabular-nums">
          <option v-for="option in years" :key="option.value" :value="option.value">{{ option.label }}</option>
        </NativeSelect>
      </div>
      <CalendarNextButton :aria-label="nextLabel" />
    </CalendarHeader>

    <div class="mt-3 flex flex-col gap-4">
      <CalendarGrid v-for="monthGrid in grid" :key="monthGrid.value.toString()">
        <CalendarGridHead>
          <CalendarGridRow>
            <CalendarHeadCell v-for="day in weekDays" :key="day">{{ day }}</CalendarHeadCell>
          </CalendarGridRow>
        </CalendarGridHead>
        <CalendarGridBody>
          <CalendarGridRow v-for="(weekDates, index) in monthGrid.rows" :key="`week-${index}`" class="mt-1 w-full">
            <CalendarCell v-for="weekDate in weekDates" :key="weekDate.toString()" :date="weekDate">
              <CalendarCellTrigger :day="weekDate" :month="monthGrid.value" />
            </CalendarCell>
          </CalendarGridRow>
        </CalendarGridBody>
      </CalendarGrid>
    </div>
  </CalendarRoot>
</template>
