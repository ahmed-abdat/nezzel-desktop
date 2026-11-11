import { describe, expect, it, vi } from 'vitest';
import { batchReportPayloads, downloadReportContext } from '../../src/helpers/reportContext';
import { logFrontendError, stripOrigins } from '../../src/tauri/errorLog';
import { reportFailedDownload, useReportStore } from '../../src/stores/report';
import { useToastStore } from '../../src/stores/toast';
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

describe('batchReportPayloads', () => {
  it.each([false, true])('builds one report per failed row, includeLink=%s', (includeLink) => {
    const rows = ['failed', 'done', 'active', 'failed', 'failed'].map((status, index) => ({
      status: status as 'failed' | 'done' | 'active',
      download: { context: downloadReportContext(`code${index}`, 1, []), url: `https://example.com/${index}`, extractor: 'Example', title: 'private filename' },
    }));
    expect(batchReportPayloads(rows, '  ملاحظة  ', includeLink, 'shared-batch')).toEqual([0, 3, 4].map(index => ({
      note: 'ملاحظة', download: rows[index]!.download.context,
      url: includeLink ? `https://example.com/${index}` : null,
      extractor: 'Example', includeLink, batchId: 'shared-batch', eventId: null,
    })));
    expect(batchReportPayloads([], '', includeLink, 'batch')).toEqual([]);
  });
});

describe('batch report delivery', () => {
  it('ignores concurrent and completed submits without sending or toasting twice', async () => {
    const sent: unknown[] = [];
    installTauriMock({ diagnostics_send: (_cmd, args) => {
      sent.push(args);
    } });
    const toast = vi.spyOn(useToastStore(), 'showToast').mockReturnValue(1);
    const store = useReportStore();
    store.openBatchReport([0, 1, 2].map(index => ({
      status: 'failed', download: { context: downloadReportContext(`code${index}`, 1, []), url: null, extractor: null },
    })));
    try {
      await Promise.all([store.send('', false), store.send('', false)]);
      await store.send('', false);
      expect(sent).toHaveLength(3);
      expect(store.download).toBeNull();
      expect(toast).toHaveBeenCalledTimes(1);
      expect(store.open).toBe(false);
    } finally { toast.mockRestore(); }
  });

  it('retries unsent rows only, with the same batch id and consent', async () => {
    const sent: { download: { errorCode: string }; batchId: string; eventId: string; note: string; url: string | null }[] = [];
    let fail = true;
    let failedEventId = '';
    installTauriMock({ diagnostics_send: (_cmd, args) => {
      const payload = args as (typeof sent)[number];
      if (payload.download.errorCode === 'code2' && fail) {
        failedEventId = payload.eventId;
        throw new Error('offline');
      }
      sent.push(payload);
    } });
    const store = useReportStore();
    store.openBatchReport([0, 1, 2].map(index => ({
      status: 'failed', download: { context: downloadReportContext(`code${index}`, 1, []), url: `https://example.com/${index}`, extractor: null },
    })));
    await store.send('note', false);
    expect(store.open).toBe(true);
    expect(store.isBatch).toBe(true);
    expect(store.batchCount).toBe(3);
    expect(sent.map(row => row.download.errorCode)).toEqual(['code0', 'code1']);
    fail = false;
    await store.send('changed note', true);
    expect(store.open).toBe(false);
    expect(sent.map(row => row.download.errorCode)).toEqual(['code0', 'code1', 'code2']);
    expect(new Set(sent.map(row => row.batchId)).size).toBe(1);
    expect(new Set(sent.map(row => row.eventId)).size).toBe(3);
    expect(sent[2]!.eventId).toBe(failedEventId);
    expect(sent.every(row => row.note === 'note')).toBe(true);
    expect(sent.every(row => row.url === null)).toBe(true);
  });
});
