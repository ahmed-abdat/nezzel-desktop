import { describe, expect, it } from 'vitest';
import { isPermanentError } from '../../src/helpers/errorOutcome';

/** Same split as outcome.rs: site decisions are permanent, anything a retry, login or tools update can fix is not. */
describe('isPermanentError', () => {
  it.each([
    ['unknownUrlType', true], ['urlNotSupported', true], ['drmProtected', true], ['geoBlocked', true],
    ['videoNotFoundOrPrivate', true], ['membersOnly', true], ['noMediaFound', true], ['fileNotFile', true],
    ['signInRequired', false], ['rateLimited429', false], ['networkNameResolutionFailed', false],
    ['livestreamNotStarted', false], ['embedOnly', false], ['imageSourceUnavailable', false], ['unknown', false], [undefined, false],
  ])('%s → %s', (code, permanent) => {
    expect(isPermanentError(code)).toBe(permanent);
  });
});
