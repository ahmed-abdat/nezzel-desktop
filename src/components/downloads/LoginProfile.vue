<script setup lang="ts">
import { computed, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import { LoaderCircle } from '@lucide/vue';
import { Button } from '@/components/ui/button';
import { cn } from 'cn';
import LoginNeeded from './LoginNeeded.vue';
import { browserName } from '../../helpers/cookieBrowsers';
import { useLoginStore } from '../../stores/login';
import { useToastStore } from '../../stores/toast';

const props = withDefaults(defineProps<{ groupId: string; disabled?: boolean; showPicker?: boolean; tone?: 'muted' | 'amber' | 'error' }>(), { showPicker: true, tone: 'muted' });
const { t } = useI18n();
const login = useLoginStore();
const busy = ref(false);
const used = computed(() => login.using[props.groupId]);
const card = computed(() => login.cardFor(props.groupId));
const profile = computed(() => {
  const candidate = used.value?.candidate;
  if (!candidate) return '';
  const browser = browserName(candidate.browser);
  return candidate.profile && candidate.browser !== 'safari' ? `${browser} · ${candidate.profile}` : browser;
});
async function change() {
  busy.value = true;
  try {
    await login.change(props.groupId);
  } catch {
    useToastStore().showToast(t('downloads.login.profilesFailed'), { style: 'error' });
  } finally {
    busy.value = false;
  }
}
</script>

<template>
  <LoginNeeded v-if="showPicker && card?.kind === 'needed' && card.request.changeOnly" :card="card" />
  <div v-else-if="used && card?.kind !== 'needed'" :class="cn('flex flex-wrap items-center gap-1 text-[length:var(--nz-fs-caption)]', tone === 'error' ? 'text-destructive-foreground' : tone === 'amber' ? 'text-amber-foreground' : 'text-muted-foreground')">
    <i18n-t keypath="downloads.login.usingProfile" tag="span">
      <template #profile><bdi dir="ltr">{{ profile }}</bdi></template>
    </i18n-t>
    <Button size="xs" variant="ghost" :disabled="disabled || busy" :aria-busy="busy || undefined" @click="change">
      <loader-circle v-if="busy" class="animate-spin" aria-hidden="true" />{{ busy ? t('downloads.login.lookingForProfiles') : t('downloads.login.changeProfile') }}
    </Button>
  </div>
</template>
