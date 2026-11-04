import { describe, expect, it } from 'vitest';
import { downloadReportContext } from '../../src/helpers/reportContext';
import { stripOrigins } from '../../src/tauri/errorLog';
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
