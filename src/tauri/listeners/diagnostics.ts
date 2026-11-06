import { listen } from '@tauri-apps/api/event';
import { ROW_EVENTS } from '../../generated/contract';
import { useMediaDiagnosticsStore, type MediaAttempts } from '../../stores/media/diagnostics.ts';

export function registerDiagnosticsListeners() {
  const diagnosticsStore = useMediaDiagnosticsStore();

  void listen<MediaDiagnostic>(ROW_EVENTS.mediaDiagnostic, (event) => {
    diagnosticsStore.processMediaDiagnosticPayload(event.payload);
  });

  void listen<MediaAttempts>(ROW_EVENTS.mediaAttempts, (event) => {
    diagnosticsStore.processMediaAttemptsPayload(event.payload);
  });

  void listen<MediaFatal>(ROW_EVENTS.mediaFatal, (event) => {
    diagnosticsStore.processMediaFatalPayload(event.payload);
  });
}
