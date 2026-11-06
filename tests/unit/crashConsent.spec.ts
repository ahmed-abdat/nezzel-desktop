import { flushPromises } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import { crashConsent, useReportStore } from '../../src/stores/report';
import { useToastStore } from '../../src/stores/toast';
import { installTauriMock } from '../utils/tauriMock';

/** A previous run's crash: sent on its own only once the user has seen that reports are on. */
describe('crash consent', () => {
  it.each([
    ['first launch with the welcome sheet, reports on', true, { reportsNoticeDone: false }, true, { showNotice: false, autoSend: true }],
    ['existing user who has not seen the notice', false, { reportsNoticeDone: false }, true, { showNotice: true, autoSend: false }],
    ['existing user with reports on', false, { reportsNoticeDone: true }, true, { showNotice: false, autoSend: true }],
    ['reports off: offered, never sent', false, { reportsNoticeDone: true }, false, { showNotice: false, autoSend: false }],
    ['preferences failed to load: offered, never sent', false, undefined, true, { showNotice: false, autoSend: false }],
  ] as const)('%s', (_name, sawReportsSwitch, onboarding, sendReports, expected) => {
    expect(crashConsent({ sawReportsSwitch, onboarding, sendReports })).toEqual(expected);
  });

  it('a crash that was not delivered is offered again', async () => {
    let sends = 0;
    installTauriMock({
      diagnostics_crash_check: () => ({ sent: false }),
      diagnostics_send_crash: () => (++sends === 1 ? Promise.reject('notDelivered') : null),
    });
    const toasts = useToastStore();
    await useReportStore().checkCrash(false);
    const offer = toasts.toasts.at(-1);
    expect(offer?.style).toBe('warning');
    offer?.action?.run();
    await flushPromises();
    const failed = toasts.toasts.at(-1);
    expect(failed?.style).toBe('error');
    failed?.action?.run();
    await flushPromises();
    expect(sends).toBe(2);
    expect(toasts.toasts.at(-1)?.style).toBe('success');
  });

  it('offers no retry that cannot work', async () => {
    installTauriMock({
      diagnostics_crash_check: () => ({ sent: false }),
      diagnostics_send_crash: () => Promise.reject('notConfigured'),
    });
    const toasts = useToastStore();
    await useReportStore().checkCrash(false);
    toasts.toasts.at(-1)?.action?.run();
    await flushPromises();
    const failed = toasts.toasts.at(-1);
    expect(failed?.style).toBe('error');
    expect(failed?.action).toBeUndefined();
  });

  it('shows nothing when Rust could not stage the crash (the file stays for next launch)', async () => {
    installTauriMock({ diagnostics_crash_check: () => null });
    const toasts = useToastStore();
    const before = toasts.toasts.length;
    await useReportStore().checkCrash(true);
    expect(toasts.toasts.length).toBe(before);
  });
});
