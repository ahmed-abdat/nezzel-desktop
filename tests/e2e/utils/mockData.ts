import { BinariesMockData } from '../../utils/mocks/binaryHandlers';
import { UpdaterMockData } from '../../utils/mocks/updateHandlers';
import { StrongholdMockData } from '../../utils/mocks/strongholdHandlers';
import { CaptureMockData } from '../../utils/mocks/captureHandlers';
import { SettingsMockData } from '../../utils/mocks/configHandlers';
import { PreferencesMockData } from '../../utils/mocks/onboardingHandlers';

export type MockData = {
  binaries?: BinariesMockData;
  updater?: UpdaterMockData;
  stronghold?: StrongholdMockData;
  capture?: CaptureMockData;
  settings?: SettingsMockData;
  preferences?: PreferencesMockData;
  /** Last run closed unexpectedly: `sent` = reported automatically, `pending` = reports off. */
  crash?: 'sent' | 'pending';
  /** `fail`: diagnostics_send rejects (no DSN in this build). */
  reportSend?: 'fail';
  /** What the UI asked Rust to send with "Report a problem". */
  reports?: { note?: string | null; download?: unknown; url?: string | null; includeLink?: boolean }[];
  /** What get_platform answers (default linux). */
  platform?: 'macos' | 'windows' | 'linux';
};
