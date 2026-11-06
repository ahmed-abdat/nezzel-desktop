import { fireAndForget } from './fireAndForget';

import type { UiUsageKey } from '@/generated/contract';

/** Counts one UI action for the daily usage totals. Never blocks or fails the action.
 *  Keys come from Rust `usage::keys::UI_KEYS`; Rust drops any other (spec: docs/specs/usage-counts.md).
 *  `funnel.batch`: once per multi-link Download; each link's `media_download` then says origin 'batch'. */
export function countUsage(key: UiUsageKey) {
  fireAndForget('usage_count', { key });
}
