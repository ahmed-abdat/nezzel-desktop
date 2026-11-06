import { listen } from '@tauri-apps/api/event';
import { ROW_EVENTS } from '../../generated/contract';
import { invoke } from '@tauri-apps/api/core';
import { useSettingsStore } from '../../stores/settings';
import type { LoginNeeded, LoginUsing } from '../types/login';

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

/** One app-wide listener: the login store attaches requests to rows by group id (media store's deleteGroup forgets them). */
export async function registerLoginListeners() {
  const { useLoginStore } = await import('../../stores/login');
  const loginStore = useLoginStore();
  void listenForLogin(payload => loginStore.receive(payload));
  void listen<LoginUsing>(ROW_EVENTS.mediaLoginUsing, event => loginStore.receiveUsing(event.payload));
  void listen<{ id: string; groupId: string }>(ROW_EVENTS.mediaFatal, event => loginStore.onFatal(event.payload));
}
