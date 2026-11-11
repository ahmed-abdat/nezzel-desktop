import { describe, expect, it, vi } from 'vitest';
import { downloadReportContext } from '../../src/helpers/reportContext';
import { logFrontendError, stripOrigins } from '../../src/tauri/errorLog';
import { reportFailedDownload } from '../../src/stores/report';
import { countUsage } from '../../src/tauri/usage';
import { installTauriMock } from '../utils/tauriMock';
import type { MediaAttempts } from '../../src/stores/media/diagnostics';

const attempts = (finalCode: string | null, stages: string[]): MediaAttempts => ({
  id: 'item', groupId: 'group', winner: null, finalCode,
  attempts: stages.map(stage => ({ stage, login: stage === 'browser', outcome: 'failed', code: finalCode, ms: 10 })),
});

describe('downloadReportContext', () => {
  it('sends only codes and the latest stage trail', () => {
    const context = downloadReportContext('http403', 1, [attempts('old', ['download']), attempts('http403', ['download', 'browser'])]);
    expect(context).toEqual({
      errorCode: 'http403',
      exitCode: 1,
      attempts: [
        { stage: 'download', login: false, outcome: 'failed', code: 'http403', ms: 10 },
        { stage: 'browser', login: true, outcome: 'failed', code: 'http403', ms: 10 },
      ],
    });
    // Nothing that could carry the row's URL or title.
    expect(JSON.stringify(context)).not.toMatch(/https?:|group|item/);
  });

  it('falls back to the trail code when the row has no diagnostic', () => {
    expect(downloadReportContext('unknown', undefined, [attempts('rateLimited', [])]).errorCode).toBe('rateLimited');
    expect(downloadReportContext(null, null, [])).toEqual({ errorCode: null, exitCode: null, attempts: [] });
  });
});

describe('stripOrigins', () => {
  it('keeps bundle file:line and drops the webview origin', () => {
    expect(stripOrigins('at f (tauri://localhost/assets/index-abc.js:12:3)\nat g (http://tauri.localhost/assets/x.js:1:2)'))
      .toBe('at f (assets/index-abc.js:12:3)\nat g (assets/x.js:1:2)');
    expect(stripOrigins('fetch https://example.com/a failed')).toBe('fetch https://example.com/a failed');
  });
});

describe('best-effort IPC', () => {
  it.each([false, true])('queues a timeout report only while the existing online signal is true (%s)', (online) => {
    const signal = vi.spyOn(navigator, 'onLine', 'get').mockReturnValue(online);
    const send = vi.fn();
    installTauriMock({ diagnostics_download_failed: send });
    try {
      reportFailedDownload({ context: { errorCode: 'networkTimeout', exitCode: 1, attempts: [] }, url: null, extractor: null });
      expect(send).toHaveBeenCalledTimes(online ? 1 : 0);
    } finally {
      signal.mockRestore();
    }
  });

  it('never throws or leaves a rejection when the backend fails', async () => {
    installTauriMock({
      diagnostics_log: () => {
        throw new Error('backend gone');
      },
      usage_count: () => Promise.reject(new Error('io')),
      diagnostics_download_failed: () => Promise.reject(new Error('io')),
    });
    expect(() => {
      logFrontendError('window', new Error('boom'));
      countUsage('paste');
      reportFailedDownload({ context: { errorCode: 'http403', exitCode: 1, attempts: [] }, url: null, extractor: null });
    }).not.toThrow();
    // An unhandled rejection would fail the run.
    await new Promise(resolve => setTimeout(resolve, 0));
  });
});
