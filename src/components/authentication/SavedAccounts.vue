<template>
  <!-- Saved accounts live in the encrypted store, not in settings: they keep their own Save. -->
  <form @submit.prevent="save">
    <credentials-config v-model="fields" />
    <div v-if="strongholdStore.status.unlocked" class="mt-3 flex justify-end">
      <Button type="submit" :disabled="!hasChanges || isSaving" :aria-busy="isSaving || undefined">{{ t('common.save') }}</Button>
    </div>
  </form>
</template>

<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { Button } from '@/components/ui/button';
import CredentialsConfig from './CredentialsConfig.vue';
import { StrongholdFields, useStrongholdStore } from '../../stores/stronghold';
import { useToastStore } from '../../stores/toast';

const { t } = useI18n();
const strongholdStore = useStrongholdStore();
const toastStore = useToastStore();

const fields = ref<StrongholdFields>({ username: null, password: null, videoPassword: null, bearer: null, headers: null });
const snapshot = ref(JSON.stringify(fields.value));
const isSaving = ref(false);
const hasChanges = computed(() => strongholdStore.status.unlocked === true && JSON.stringify(fields.value) !== snapshot.value);

async function load() {
  if (!strongholdStore.status.unlocked) return;
  try {
    fields.value = await strongholdStore.getValues();
    snapshot.value = JSON.stringify(fields.value);
  } catch (e) {
    console.error(e);
  }
}
onMounted(load);
// Enabling the store on this page unlocks it: load what it holds.
watch(() => strongholdStore.status.unlocked, (unlocked) => {
  if (unlocked) void load();
});

async function save() {
  if (!hasChanges.value) return;
  isSaving.value = true;
  try {
    await strongholdStore.setValues(fields.value);
    snapshot.value = JSON.stringify(fields.value);
    toastStore.showToast(t('auth.toasts.saved'), { style: 'success' });
  } catch (e) {
    toastStore.showToast(t('auth.toasts.error', { error: e }), { style: 'error' });
  } finally {
    isSaving.value = false;
  }
}
</script>
