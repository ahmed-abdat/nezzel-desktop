/**
 * Diagnostic codes whose failure is decided by the site, so retrying the same link fails the same way.
 * Mirrors the code → Outcome table in src-tauri/src/extract/outcome.rs (`diagnostic_from_code`):
 * Unsupported, Removed, Private, Geo, Drm, Paid, KindMismatch and NoMedia. Keep the two in step.
 * Not here: Login/AgeRestricted (a login fixes them), LiveNotStarted (later works), RateLimited,
 * Network, Forbidden/Blocked, ExtractFailed (a tools update fixes them), and imageSourceUnavailable,
 * which is NoMedia in Rust but its message tells the user to try again.
 */
const PERMANENT_CODES: ReadonlySet<string> = new Set([
  // Unsupported
  'urlNotSupported', 'urlUnsupported', 'unknownUrlType', 'fileNotFile',
  // Removed / Private
  'notFound404', 'copyrightClaimed', 'channelUnavailable', 'videoNotFoundOrPrivate', 'playlistPrivateOrMissing',
  // Geo, Drm, Paid
  'geoBlocked', 'drmProtected', 'membersOnly',
  // KindMismatch, NoMedia
  'photosOnly', 'noMediaFound', 'fileEmpty',
]);

/** True when retrying cannot help until something on the site changes. */
export function isPermanentError(code: string | undefined): boolean {
  return !!code && PERMANENT_CODES.has(code);
}
