<template>
  <!-- Secret field for a settings row: 28px field, show/hide and clear as small icon buttons. -->
  <div class="flex w-full min-w-0 items-center gap-1 sm:w-64">
    <label v-if="label" :for="id" class="sr-only">{{ label }}</label>
    <Input
      :id="id"
      :type="reveal ? 'text' : (password ? 'password' : 'text')"
      class="min-w-0 flex-1"
      :model-value="displayValue"
      :disabled="disabled"
      dir="ltr"
      autocomplete="off"
      spellcheck="false"
      @input="onInput" />
    <tooltip v-if="password" :content="reveal ? t('components.base.secretInput.hide') : t('components.base.secretInput.show')">
      <Button
        variant="ghost"
        size="icon-xs"
        class="text-muted-foreground"
        :aria-label="`${reveal ? t('components.base.secretInput.hide') : t('components.base.secretInput.show')} ${label ?? ''}`.trim()"
        @click="reveal = !reveal"
      >
        <EyeOff v-if="reveal" aria-hidden="true" />
        <Eye v-else aria-hidden="true" />
      </Button>
    </tooltip>
    <tooltip v-if="clearable" :content="t('components.base.secretInput.clear', { label })">
      <Button
        variant="ghost"
        size="icon-xs"
        class="text-muted-foreground"
        :disabled="modelValue == null"
        :aria-label="t('components.base.secretInput.clear', { label })"
        @click="$emit('update:modelValue', null)"
      >
        <X aria-hidden="true" />
      </Button>
    </tooltip>
  </div>
</template>

<script setup lang="ts">
import Tooltip from '../ui/tooltip/Tooltip.vue';
import { Input } from '../ui/input';
import { computed, ref } from 'vue';
import { Eye, EyeOff, X } from '@lucide/vue';
import { Button } from '../ui/button';
import { useI18n } from 'vue-i18n';

const { t } = useI18n();

const props = withDefaults(defineProps<{
  modelValue: string | null | undefined;
  id: string;
  label?: string;
  password?: boolean;
  disabled?: boolean;
  clearable?: boolean;
}>(), { password: false, disabled: false, clearable: true });

const emit = defineEmits<{ (event: 'update:modelValue', value: string | null): void }>();

const reveal = ref(false);

const displayValue = computed(() => props.modelValue ?? '');
function onInput(e: Event) {
  const v = (e.target as HTMLInputElement).value;
  emit('update:modelValue', v.trim() === '' ? null : v);
}
</script>
