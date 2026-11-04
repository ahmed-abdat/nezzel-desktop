import { computed, Ref } from 'vue';
import { useI18n } from 'vue-i18n';
import { useMediaDiagnosticsStore } from '../stores/media/diagnostics.ts';
import { firstSentence } from '../helpers/diagnostics.ts';
import { useLinkify } from './useLinkify.ts';

type DisplayDiagnostic = {
  shortMessage: string;
  message: string;
  raw: string;
};

export function useDiagnostic(diagnostic: Ref<MediaDiagnostic>, short = false) {
  const diagnosticsStore = useMediaDiagnosticsStore();

  const i18n = useI18n();
  const t = i18n.t;

  const { linkify } = useLinkify();

  const diagnosticDisplay = computed((): DisplayDiagnostic => {
    const i18nValues: Record<string, Record<string, string>> = i18n.tm('errors.runner');
    const fallbackValues = i18n.getLocaleMessage('en') as { errors?: { runner?: Record<string, Record<string, string>> } };

    const message = diagnostic.value?.message ?? t('errors.runner.unknown.message');
    const linkifiedMessage = linkify(message);

    const raw = diagnostic.value?.raw ?? '';
    const linkifiedRaw = linkify(raw);

    const shortMessage = diagnostic.value.message
      ? firstSentence(diagnostic.value.message)
      : t('errors.runner.unknown.shortMessage');

    if (diagnostic.value?.code?.startsWith('file')) {
      // The Files engine supplies localized copy from its own locale catalog.
      const message = diagnostic.value.message;
      return { message, shortMessage: message, raw: linkifiedRaw };
    }
    if (diagnostic.value?.code && diagnostic.value.code !== 'unknown') {
      return {
        message: linkifiedMessage,
        shortMessage,
        ...(i18nValues[diagnostic.value.code] ?? fallbackValues.errors?.runner?.[diagnostic.value.code]),
        raw: linkifiedRaw,
      };
    } else if (short) {
      return {
        message: t('errors.runner.unknown.message'),
        shortMessage: t('errors.runner.unknown.hint'),
        raw: linkifiedRaw,
      };
    } else {
      return {
        message: linkifiedMessage,
        shortMessage,
        raw: linkifiedRaw,
      };
    }
  });

  const relatedFatal = computed(() => {
    const fatals = diagnosticsStore.findFatalsByGroupId(diagnostic.value.groupId);
    return fatals.find(fatal => fatal.id === diagnostic.value?.id);
  });

  return { relatedFatal, diagnosticDisplay };
}
