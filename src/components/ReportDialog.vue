<template>
  <Dialog :open="store.open" @update:open="value => { if (!value) store.close(); }">
    <DialogContent :show-close-button="false" class="w-[min(32rem,calc(100vw-2rem))] max-w-none sm:max-w-none" data-testid="report-dialog">
      <form class="flex flex-col gap-4" @submit.prevent="store.send(note, includeLink)">
        <DialogHeader>
          <DialogTitle>{{ store.download ? t('report.downloadTitle') : t('report.title') }}</DialogTitle>
          <DialogDescription>{{ t('report.lead') }}</DialogDescription>
        </DialogHeader>

        <div class="flex flex-col gap-1.5">
          <Label for="report-note">{{ t('report.noteLabel') }}</Label>
          <Textarea
            id="report-note"
            v-model="note"
            data-testid="report-note"
            rows="4"
            :maxlength="2000"
            :placeholder="t('report.notePlaceholder')"
            :disabled="store.sending"
          />
        </div>

        <label v-if="store.download?.url" class="flex cursor-pointer items-start gap-2.5 text-[13px]">
          <Checkbox v-model="includeLink" data-testid="report-include-link" class="mt-0.5" :disabled="store.sending" />
          <span class="flex min-w-0 flex-col items-start gap-0.5 text-start">
            <span>{{ t('report.includeLink') }}</span>
            <small class="text-nz-caption text-muted-foreground">{{ t('report.includeLinkHelp') }}</small>
          </span>
        </label>

        <p class="text-nz-caption text-muted-foreground">{{ t('report.never') }}</p>

        <DialogFooter>
          <Button type="button" variant="ghost" data-testid="report-cancel" :disabled="store.sending" @click="store.close()">{{ t('report.cancel') }}</Button>
          <Button type="submit" data-testid="report-send" :disabled="store.sending" :aria-busy="store.sending || undefined">
            <LoaderCircle v-if="store.sending" class="animate-spin" aria-hidden="true" /><Send v-else aria-hidden="true" />
            {{ store.sending ? t('report.sending') : t('report.send') }}
          </Button>
        </DialogFooter>
      </form>
    </DialogContent>
  </Dialog>
</template>

<script setup lang="ts">
import { ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { LoaderCircle, Send } from '@lucide/vue';
import { Button } from '@/components/ui/button';
import { Checkbox } from '@/components/ui/checkbox';
import { Label } from '@/components/ui/label';
import { Textarea } from '@/components/ui/textarea';
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog';
import { useReportStore } from '../stores/report';

const { t } = useI18n();
const store = useReportStore();

const note = ref('');
const includeLink = ref(true);

watch(() => store.open, (open) => {
  if (open) {
    note.value = '';
    includeLink.value = true;
  }
});
</script>
