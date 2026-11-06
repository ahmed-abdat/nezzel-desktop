import { clearMocks } from '@tauri-apps/api/mocks';
import { installTauriMock } from '../tauriMock';
import { mediaHandlers } from './mediaHandlers';
import { configHandlers } from './configHandlers';
import { binaryHandlers } from './binaryHandlers';
import { updateHandlers } from './updateHandlers';
import { strongholdHandlers } from './strongholdHandlers';
import { onboardingHandlers } from './onboardingHandlers';
import { diagnosticsHandlers } from './diagnosticsHandlers';

/** The main window's e2e backend. Loaded only by `mainWindow.ts` behind `__E2E__`, so no build ships it. */
export function installMainWindowMocks(): void {
  clearMocks();
  installTauriMock({
    ...mediaHandlers,
    ...configHandlers,
    ...binaryHandlers,
    ...updateHandlers,
    ...strongholdHandlers,
    ...onboardingHandlers,
    ...diagnosticsHandlers,
  });
}
