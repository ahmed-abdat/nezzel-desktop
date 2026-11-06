/**
 * A failed read in the quick bar or card: say why in plain words and offer only actions that can work.
 * Reasons come from the Rust preview (`preview::FailReason`), rendered into the generated contract.
 */
import { FAIL_REASONS, type FailReason } from '@/generated/contract';

export type { FailReason };
export type FailAction = 'retry' | 'options' | 'report';

export function failReason(reason: string | null | undefined): FailReason {
  return (FAIL_REASONS as readonly string[]).includes(reason ?? '') ? reason as FailReason : 'unknown';
}

/** Primary (filled) action first; Report is always reachable, quietly, unless it is the primary. */
const ACTIONS: Record<FailReason, { primary: FailAction | null; secondary: FailAction[] }> = {
  login: { primary: 'options', secondary: ['retry'] }, // Options can use a browser where the user is logged in
  unavailable: { primary: 'retry', secondary: [] },
  unsupported: { primary: 'report', secondary: [] },
  region: { primary: 'retry', secondary: [] },
  protected: { primary: null, secondary: [] }, // nothing can save DRM: no Download, no Retry
  busy: { primary: 'retry', secondary: [] },
  network: { primary: 'retry', secondary: [] },
  slow: { primary: 'options', secondary: ['retry'] }, // the list keeps reading with no time limit
  unknown: { primary: 'retry', secondary: ['options'] },
};

export function failActions(reason: FailReason): { primary: FailAction | null; secondary: FailAction[] } {
  const { primary, secondary } = ACTIONS[reason];
  return { primary, secondary: primary === 'report' ? secondary : [...secondary, 'report'] };
}
