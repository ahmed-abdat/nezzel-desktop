import { listen } from '@tauri-apps/api/event';
import { ROW_EVENTS } from '../../generated/contract';
import { invoke } from '@tauri-apps/api/core';
import { useSettingsStore } from '../../stores/settings';
import type { LoginNeeded, LoginNotice, LoginUsing } from '../types/login';

export const listenForLogin = (handler: (payload: LoginNeeded) => void) =>
  listen<LoginNeeded>(ROW_EVENTS.mediaLoginNeeded, event => handler(event.payload));

export const acceptBrowserLogin = async (requestId: string, browserId: string) => {
  await invoke<void>('media_login_accept', { requestId, browserId });
  await useSettingsStore().load();
};

export const dismissBrowserLogin = (requestId: string) =>
  invoke<void>('media_login_dismiss', { requestId });

export const browserLoginChoices = (groupId: string) =>
  invoke<LoginNeeded>('media_login_choices', { groupId });

export const acknowledgeLoginNotice = (token: string) =>
  invoke<void>('media_login_notice', { seenToken: token });

/** Await listener setup, then replay any actual-use notice emitted during startup. */
export async function registerLoginListeners() {
  const { useLoginStore } = await import('../../stores/login');
  const loginStore = useLoginStore();
  await Promise.all([
    listenForLogin(payload => loginStore.receive(payload)),
    listen<LoginUsing>(ROW_EVENTS.mediaLoginUsing, event => loginStore.receiveUsing(event.payload)),
    listen<LoginNotice>(ROW_EVENTS.mediaLoginNotice, event => loginStore.receiveNotice(event.payload)),
    listen<{ id: string; groupId: string }>(ROW_EVENTS.mediaFatal, event => loginStore.onFatal(event.payload)),
  ]);
  const pending = await invoke<LoginNotice | null>('media_login_notice', { seenToken: null }).catch(() => null);
  if (pending) loginStore.receiveNotice(pending);
}
