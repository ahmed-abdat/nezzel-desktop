import { listen } from '@tauri-apps/api/event';
import { useMediaDiagnosticsStore, type MediaAttempts } from '../../stores/media/diagnostics.ts';

export function registerDiagnosticsListeners() {
  const diagnosticsStore = useMediaDiagnosticsStore();

  void listen<MediaDiagnostic>('media_diagnostic', (event) => {
    diagnosticsStore.processMediaDiagnosticPayload(event.payload);
  });

  void listen<MediaAttempts>('media_attempts', (event) => {
    diagnosticsStore.processMediaAttemptsPayload(event.payload);
  });

  void listen<MediaFatal>('media_fatal', (event) => {
    diagnosticsStore.processMediaFatalPayload(event.payload);
  });
}
