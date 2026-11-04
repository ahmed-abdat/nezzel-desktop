<template>
  <component :is="icon" aria-hidden="true" />
</template>

<script setup lang="ts">
import { computed } from 'vue';
import { Disc3, File, FileArchive, FileCode, FileSpreadsheet, FileText, Package, Presentation } from '@lucide/vue';
import type { FileCategory } from '../../helpers/fileCategory';

const props = defineProps<{ category: FileCategory }>();
const ICONS = { document: FileText, spreadsheet: FileSpreadsheet, presentation: Presentation, archive: FileArchive, installer: Package, diskImage: Disc3, data: FileCode, other: File };
const icon = computed(() => ICONS[props.category]);
</script>
