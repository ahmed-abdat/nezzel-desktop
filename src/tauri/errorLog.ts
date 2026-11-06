import type { App } from 'vue';
import { fireAndForget } from './fireAndForget';

/** Keep `assets/x.js:12:3` from stack frames; drop the webview origin (Rust scrubs the rest). */
export function stripOrigins(text: string): string {
  return text.replace(/\b(?:tauri|https?):\/\/(?:tauri\.)?localhost(?::\d+)?\//g, '');
}

function describe(error: unknown): string {
  if (error instanceof Error) return `${error.name}: ${error.message}\n${error.stack ?? ''}`;
  if (typeof error === 'string') return error;
  try {
    return JSON.stringify(error) ?? String(error);
  } catch {
    return String(error);
  }
}

/** Rust caps how many one run takes (`diagnostics::MAX_FRONTEND_ERRORS`), scrubs and logs them. */
export function logFrontendError(kind: string, error: unknown): void {
  const message = `${kind}: ${stripOrigins(describe(error))}`;
  fireAndForget('diagnostics_log', { level: 'error', message });
}

/** Vue errorHandler, window.onerror and unhandledrejection go to the app log, redacted in Rust. */
export function installErrorLog(app: App): void {
  const previous = app.config.errorHandler;
  app.config.errorHandler = (error, instance, info) => {
    logFrontendError(`vue ${info}`, error);
    if (previous) previous(error, instance, info);
    else console.error(error);
  };
  window.addEventListener('error', event => logFrontendError('window', event.error ?? event.message));
  window.addEventListener('unhandledrejection', event => logFrontendError('promise', event.reason));
}
