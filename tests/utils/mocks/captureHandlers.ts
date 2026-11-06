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
  id: 1, url: '', site: '', title: '', details: emptyDetails(), failed: false, reason: null, resolving: false, feed: false,
  ...window.E2E.capture?.offer,
});

/** Calls the panel made, for e2e assertions (`page.evaluate(() => window.__captureCalls)`). */
export type CaptureCall = { cmd: string; args?: unknown };
const record = (cmd: string, args?: unknown) => {
  const target = window as unknown as { __captureCalls?: CaptureCall[] };
  (target.__captureCalls ??= []).push({ cmd, args });
};

const captureHandlers: Record<string, IPCHandler> = {
  capture_snapshot: () => offer(),
  capture_edit: () => undefined,
  capture_retry: () => undefined,
  capture_fit: (cmd, args) => {
    record(cmd, args);
  },
  capture_action: (cmd, args) => {
    record(cmd, args);
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

/** Give a mounted capture panel a new offer, as the Rust side does: change the snapshot, then wake the panel. */
export function pushCaptureOffer(value: Partial<CaptureOffer>): Promise<void> {
  window.E2E.capture = { ...window.E2E.capture, offer: { ...window.E2E.capture?.offer, ...value } };
  return emit('capture_changed');
}
