import { defineStore } from 'pinia';
import { ref } from 'vue';
import { acceptBrowserLogin, browserLoginChoices, dismissBrowserLogin } from '../tauri/listeners/login';
import type { LoginNeeded, LoginUsing } from '../tauri/types/login';
import { useMediaDiagnosticsStore } from './media/diagnostics';
import { useMediaGroupStore } from './media/group';
import { useToastStore } from './toast';
import { i18n } from '../i18n';
import { LOGIN_APP_BOUND_REASON, LOGIN_TTL_MS } from '@/generated/contract';

/**
 * Why a row that needed a login ended up failed. Drives the plain failure line.
 * `refused` is UI-only: the site refused and the engine did not ask (a fixed browser, or "never").
 */
export type LoginFailure = 'exhausted' | 'dismissed' | 'expired' | 'refused';

export interface FailedLogin {
  groupId: string;
  host: string;
  reason: LoginFailure;
  /** Browser id that was tried and refused, if any. */
  tried?: string;
  /** Browser family from candidate metadata; candidate ids are opaque. */
  browser?: string;
  hint?: string;
  blockedBrowser?: string;
}

interface TriedProfile {
  id: string;
  browser?: string;
}

export type LoginCard
  = | { kind: 'needed'; request: LoginNeeded; preselect: string }
    | { kind: 'failed'; failure: FailedLogin };

/**
 * Login requests from the engine (`media_login_needed`), attached to list rows by group id.
 * One listener for the whole app; rows only read `cardFor(groupId)`.
 */
export const useLoginStore = defineStore('login', () => {
  /** Live requests by requestId, oldest first (insertion order). */
  const pending = ref<Record<string, LoginNeeded>>({});
  /** Terminal state per group, until the group is removed or asks again. */
  const failed = ref<Record<string, FailedLogin>>({});
  /** Last profile the user picked per host (opaque id + browser family), so "Try another browser" does not offer it first. */
  const tried = ref<Record<string, TriedProfile>>({});
  const using = ref<Record<string, LoginUsing>>({});
  const timers = new Map<string, ReturnType<typeof setTimeout>>();

  function receive(request: LoginNeeded) {
    if (request.status === 'exhausted') {
      dropGroup(request.groupId);
      const candidate = using.value[request.groupId]?.candidate;
      const last = tried.value[request.host];
      const browser = candidate?.browser ?? last?.browser;
      failed.value[request.groupId] = { groupId: request.groupId, host: request.host, reason: 'exhausted', tried: candidate?.id ?? last?.id, ...(browser ? { browser } : {}), ...(request.reason === LOGIN_APP_BOUND_REASON ? { hint: request.reason, blockedBrowser: request.blockedBrowser } : {}) };
      return;
    }
    if (!request.changeOnly) delete failed.value[request.groupId];
    pending.value[request.requestId] = request;
    clearTimer(request.requestId);
    // The engine forgets a request after LOGIN_TTL_MS; drop it here at the same time.
    timers.set(request.requestId, setTimeout(() => expire(request.requestId), LOGIN_TTL_MS));
  }

  function receiveUsing(payload: LoginUsing) {
    using.value[payload.groupId] = payload;
    if (!payload.selectedOnly) delete failed.value[payload.groupId];
  }

  async function change(groupId: string) {
    receive(await browserLoginChoices(groupId));
  }

  function cardFor(groupId: string): LoginCard | null {
    const request = Object.values(pending.value).find(r => r.groupId === groupId);
    if (request) return { kind: 'needed', request, preselect: preselectFor(request) };
    const failure = failed.value[groupId];
    return failure ? { kind: 'failed', failure } : null;
  }

  function preselectFor(request: LoginNeeded): string {
    return request.candidates.find(candidate => candidate.signedIn && !candidate.appBound)?.id ?? '';
  }

  /** Same group and host: one answer covers every waiting item (e.g. a playlist). */
  function siblings(request: LoginNeeded, browserId?: string): LoginNeeded[] {
    return Object.values(pending.value).filter(r => r.groupId === request.groupId && r.host === request.host
      && (!browserId || r.candidates.some(c => c.id === browserId)));
  }

  async function accept(requestId: string, browserId: string) {
    const request = pending.value[requestId];
    if (!request) return;
    const batch = siblings(request, browserId);
    batch.forEach(r => forget(r.requestId));
    const candidate = request.candidates.find(c => c.id === browserId);
    tried.value[request.host] = { id: browserId, ...(candidate ? { browser: candidate.browser } : {}) };
    const results = await Promise.allSettled(batch.map(r => acceptBrowserLogin(r.requestId, browserId)));
    if (results.some(r => r.status === 'rejected')) {
      useToastStore().showToast(i18n.global.t('downloads.login.expired'), { style: 'error' });
    }
  }

  async function dismiss(requestId: string) {
    const request = pending.value[requestId];
    if (!request) return;
    const batch = siblings(request);
    batch.forEach(r => forget(r.requestId));
    if (!request.changeOnly) failed.value[request.groupId] = { groupId: request.groupId, host: request.host, reason: 'dismissed' };
    await Promise.allSettled(batch.map(r => dismissBrowserLogin(r.requestId)));
    // The engine only frees the slot on dismiss; the row must still leave "reading"/"downloading".
    markFailed(batch.filter(r => !r.changeOnly));
  }

  function expire(requestId: string) {
    const request = pending.value[requestId];
    if (!request) return;
    forget(requestId);
    if (!request.changeOnly) failed.value[request.groupId] = { groupId: request.groupId, host: request.host, reason: 'expired' };
  }

  /** The engine failed this item (timeout, cancel): its request is gone. */
  function onFatal(fatal: { id: string; groupId: string }) {
    Object.values(pending.value)
      .filter(r => r.id === fatal.id && r.groupId === fatal.groupId)
      .forEach(r => forget(r.requestId));
  }

  /** Row removed or retried: drop everything we hold for it. */
  function forgetGroup(groupId: string) {
    dropGroup(groupId);
    delete failed.value[groupId];
    delete using.value[groupId];
  }

  function dropGroup(groupId: string) {
    Object.values(pending.value).filter(r => r.groupId === groupId).forEach(r => forget(r.requestId));
  }

  function forget(requestId: string) {
    clearTimer(requestId);
    delete pending.value[requestId];
  }

  function clearTimer(requestId: string) {
    const timer = timers.get(requestId);
    if (timer) clearTimeout(timer);
    timers.delete(requestId);
  }

  function markFailed(batch: LoginNeeded[]) {
    const diagnostics = useMediaDiagnosticsStore();
    const groups = useMediaGroupStore();
    for (const r of batch) {
      if (!groups.findGroupById(r.groupId)) continue;
      diagnostics.processMediaFatalPayload({
        id: r.id, groupId: r.groupId, exitCode: null, internal: true, details: null,
        message: 'Login required', timestamp: Date.now(),
      });
    }
  }

  return { pending, failed, tried, using, receive, receiveUsing, change, cardFor, accept, dismiss, expire, onFatal, forgetGroup };
});
