import { computed } from 'vue';
import { useI18n } from 'vue-i18n';
import { usePlatform } from './usePlatform';
import { localeDirection } from '../i18n';

/** macOS traffic lights stay at the physical top-left, including in RTL. */
export function useTrafficLightsInset() {
  const { isMac } = usePlatform();
  const { locale } = useI18n();
  const inset = computed(() => isMac.value ? 84 : 0);
  const rtlInset = computed(() => localeDirection(locale.value) === 'rtl' ? inset.value : 0);
  return { inset, rtlInset };
}
