import type { App } from 'vue';

/**
 * The webview Sentry SDK is not started: its breadcrumbs and request data carry URLs and it
 * would bypass the Settings switch. Frontend errors go to Rust instead (tauri/errorLog.ts →
 * diagnostics_log), where one scrubber and the switch apply before anything is sent.
 */
export function createSentry(_app: App<Element>): void {}
