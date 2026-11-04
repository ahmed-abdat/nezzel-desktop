import { mockWindows } from '@tauri-apps/api/mocks';
import { emit } from '@tauri-apps/api/event';
import { IPCHandler, installTauriMock } from '../tauriMock';
import { configHandlers } from './configHandlers';
import { emptyDetails, type CaptureOffer } from '../../../src/captureTypes';

export type CaptureMockData = {
  /** Offer returned by capture_snapshot; missing fields take the empty offer's values. */
  offer?: Partial<CaptureOffer>;
  /** capture_action: 'pending' never settles (busy button), 'fail' rejects (action error). */
  action?: 'pending' | 'fail';
};

const offer = (): CaptureOffer => ({
  id: 1, url: '', site: '', title: '', details: emptyDetails(), failed: false, resolving: false, feed: false,
  ...window.E2E.capture?.offer,
});

const captureHandlers: Record<string, IPCHandler> = {
  capture_snapshot: () => offer(),
  capture_edit: () => undefined,
  capture_action: () => {
    if (window.E2E.capture?.action === 'pending') return new Promise(() => {});
    if (window.E2E.capture?.action === 'fail') return Promise.reject(new Error('capture_action failed'));
    return undefined;
  },
};

/** E2E only: the capture window believes it is `label` and talks to mocked commands. */
export function installCaptureMock(label: string): void {
  installTauriMock({ ...configHandlers, ...captureHandlers });
  mockWindows(label);
}

/** Push a new offer to a mounted capture panel, as the Rust side does. */
export function pushCaptureOffer(value: Partial<CaptureOffer>): Promise<void> {
  return emit('capture_update', { ...offer(), ...value });
}
