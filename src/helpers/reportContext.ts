import type { MediaAttempts } from '../stores/media/diagnostics';
import type { ReportDownloadContext } from '../stores/report';

/**
 * A failed row's report context: the error code, exit code and the latest stage trail.
 * Only codes and stage names cross to Rust; the row's URL and title never do.
 */
export function downloadReportContext(code: string | null | undefined, exitCode: number | null | undefined, reports: MediaAttempts[]): ReportDownloadContext {
  const latest = reports.at(-1);
  return {
    errorCode: code && code !== 'unknown' ? code : latest?.finalCode ?? null,
    exitCode: exitCode ?? null,
    attempts: (latest?.attempts ?? []).map(({ stage, login, outcome, code: attemptCode, ms }) => ({ stage, login, outcome, code: attemptCode, ms })),
  };
}
