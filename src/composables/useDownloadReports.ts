import type { Group } from '../tauri/types/group';
import { MediaState, useMediaStateStore } from '../stores/media/state';
import { useMediaDiagnosticsStore } from '../stores/media/diagnostics';
import { useReportStore } from '../stores/report';
import { statusOf } from '../helpers/listFilter';
import { downloadReportContext } from '../helpers/reportContext';

/** Build the same context as a row report; the dialog owns the shared note and link consent. */
export function useDownloadReports() {
  const state = useMediaStateStore();
  const diagnostics = useMediaDiagnosticsStore();
  const reports = useReportStore();

  function reportGroups(groups: Group[]) {
    reports.openBatchReport(groups.map(group => ({
      status: statusOf(state.getGroupState(group.id) ?? MediaState.fetching),
      download: {
        context: downloadReportContext(
          diagnostics.findLastDiagnosticByGroupId(group.id)?.code,
          diagnostics.findLastFatalByGroupId(group.id)?.exitCode,
          diagnostics.findAttemptsByGroupId(group.id),
        ),
        url: group.url ?? null,
        extractor: group.extractor ?? null,
      },
    })));
  }

  return { reportGroups };
}
