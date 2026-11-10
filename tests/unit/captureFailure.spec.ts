import { describe, expect, it } from 'vitest';
import { FAIL_REASONS } from '../../src/generated/contract';
import { failActions, failReason } from '../../src/helpers/captureFailure';
import en from '../../src/locales/en.json';
import ar from '../../src/locales/ar.json';
import fr from '../../src/locales/fr.json';

describe('a failed read offers only actions that can work', () => {
  it.each([
    ['login', 'options', ['retry']], // a login wall is not a bug: no Report
    ['unavailable', 'retry', ['report']],
    ['unsupported', 'report', []],
    ['protected', null, ['report']],
    ['slow', 'options', ['retry', 'report']],
    ['unknown', 'retry', ['options', 'report']],
  ] as const)('%s', (reason, primary, secondary) => {
    expect(failActions(reason)).toEqual({ primary, secondary });
  });

  it('never offers Download, and an unknown or missing reason reads as unknown', () => {
    for (const reason of FAIL_REASONS) {
      const { primary, secondary } = failActions(reason);
      expect([primary, ...secondary]).not.toContain('download');
    }
    expect(failReason('rm -rf')).toBe('unknown');
    expect(failReason(null)).toBe('unknown');
  });

  it.each([['en', en], ['ar', ar], ['fr', fr]] as const)('every reason says what happened and what to do (%s)', (_locale, messages) => {
    const fail = (messages as { capture: { fail: Record<string, { title?: string; hint?: string }> } }).capture.fail;
    for (const reason of FAIL_REASONS) {
      expect(fail[reason]?.title, reason).toBeTruthy();
      expect(fail[reason]?.hint, reason).toBeTruthy();
    }
  });
});
