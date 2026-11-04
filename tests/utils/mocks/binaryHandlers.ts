import { IPCHandler } from '../tauriMock';
import { emit } from '@tauri-apps/api/event';
import {
  BinaryCheckPayload,
  BinaryDownloadCompletePayload,
  BinaryDownloadErrorPayload,
  BinaryDownloadProgressPayload,
  BinaryDownloadStartPayload,
} from '../../../src/tauri/types/binaries';

export type BinariesMockData = {
  check?: string[];
  ensure?: boolean;
  /** binaries_ensure reports a download error for every tool. */
  fail?: boolean;
};

export const binaryHandlers: Record<string, IPCHandler> = {
  binaries_check: (): BinaryCheckPayload => {
    return { tools: window.E2E.binaries?.check || [] };
  },
  binaries_ensure: async (): Promise<void> => {
    if (window.E2E.binaries?.fail) {
      for (const tool of window.E2E.binaries.check ?? []) {
        await emit<BinaryDownloadErrorPayload>('binary_download_error', { tool, version: '1.2.3', stage: 'download', error: 'connection reset' });
      }
      return;
    }
    if (window.E2E.binaries?.ensure) {
      const tools = window.E2E.binaries?.check?.length ? window.E2E.binaries.check : ['yt-dlp'];
      return new Promise((resolve): void => {
        for (const tool of tools) void emit<BinaryDownloadStartPayload>('binary_download_start', { tool, version: '1.2.3' });
        setTimeout((): void => {
          for (const tool of tools) {
            void emit<BinaryDownloadProgressPayload>('binary_download_progress', { tool, received: 1024 * 1024 * 5, total: 1024 * 1024 * 10 });
          }
        }, 1000);
        setTimeout((): void => {
          for (const tool of tools) void emit<BinaryDownloadCompletePayload>('binary_download_complete', { tool });
        }, 2000);
        setTimeout(resolve, 3000);
      });
    } else {
      return Promise.resolve();
    }
  },
};
