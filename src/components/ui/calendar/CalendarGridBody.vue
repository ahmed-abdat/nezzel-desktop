<script setup lang="ts">
import { CalendarGridBody, type CalendarGridBodyProps } from 'reka-ui';

const props = defineProps<CalendarGridBodyProps>();
</script>

<template>
  <CalendarGridBody data-slot="calendar-grid-body" v-bind="props">
    <slot />
  </CalendarGridBody>
</template>
