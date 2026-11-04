import { useI18n } from 'vue-i18n';
import { formatCount, formatPercent, formatSize, formatSpeed } from '../helpers/localeNumber';

export function useLocaleNumber() {
  const { locale } = useI18n();
  return {
    count: (value: number) => formatCount(value, locale.value),
    percent: (value: number, digits = 0) => formatPercent(value, locale.value, digits),
    size: (bytes: number) => formatSize(bytes, locale.value),
    speed: (bytesPerSecond: number) => formatSpeed(bytesPerSecond, locale.value),
  };
}
