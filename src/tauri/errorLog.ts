import { invoke } from '@tauri-apps/api/core';
import type { App } from 'vue';

/** A render loop must not flood the log: cap what one session sends. */
const MAX_PER_SESSION = 50;
let sent = 0;

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

export function logFrontendError(kind: string, error: unknown): void {
  if (sent >= MAX_PER_SESSION) return;
  sent++;
  const message = `${kind}: ${stripOrigins(describe(error))}`;
  void invoke('diagnostics_log', { level: 'error', message }).catch(() => {});
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
