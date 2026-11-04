import { IPCHandler } from '../tauriMock';

type SendArgs = { note?: string | null; download?: unknown; url?: string | null; includeLink?: boolean };

/** Reports: records what the UI asked Rust to send in `window.E2E.reports`; a test can seed `crash`. */
export const diagnosticsHandlers: Record<string, IPCHandler> = {
  diagnostics_report: () => ({ title: 'Problem report', body: '### Diagnostics\n\n- App: Nezzel 0.1.0-beta.1\n' }),
  diagnostics_send: (_cmd, args) => {
    if (!window.E2E) return null;
    if (window.E2E.reportSend === 'fail') return Promise.reject(new Error('not configured'));
    window.E2E.reports = [...(window.E2E.reports ?? []), args as SendArgs];
    return null;
  },
  diagnostics_download_failed: () => false,
  diagnostics_crash_check: () => {
    if (!window.E2E?.crash) return null;
    const sent = window.E2E.crash === 'sent';
    window.E2E.crash = undefined;
    return { sent };
  },
  diagnostics_send_crash: () => null,
  diagnostics_log: () => null,
};
