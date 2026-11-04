import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { useSettingsStore } from '../../stores/settings';
import type { EngineAvailability, LoginNeeded, LoginUsing, MediaTransport } from '../types/login';

export const listenForLogin = (handler: (payload: LoginNeeded) => void) =>
  listen<LoginNeeded>('media_login_needed', event => handler(event.payload));

export const listenForTransport = (handler: (payload: MediaTransport) => void) =>
  listen<MediaTransport>('media_transport', event => handler(event.payload));

export const acceptBrowserLogin = async (requestId: string, browserId: string) => {
  await invoke<void>('media_login_accept', { requestId, browserId });
  await useSettingsStore().load();
};

export const dismissBrowserLogin = (requestId: string) =>
  invoke<void>('media_login_dismiss', { requestId });

export const browserLoginChoices = (groupId: string) =>
  invoke<LoginNeeded>('media_login_choices', { groupId });

export const engineAvailability = () => invoke<EngineAvailability>('engine_availability');

/** One app-wide listener: the login store attaches requests to rows by group id. */
export async function registerLoginListeners() {
  const { useLoginStore } = await import('../../stores/login');
  const { useMediaStore } = await import('../../stores/media/media');
  const loginStore = useLoginStore();
  useMediaStore().$onAction(({ name, args }) => {
    if (name === 'deleteGroup') loginStore.forgetGroup(args[0] as string);
  });
  void listenForLogin(payload => loginStore.receive(payload));
  void listen<LoginUsing>('media_login_using', event => loginStore.receiveUsing(event.payload));
  void listen<{ id: string; groupId: string }>('media_fatal', event => loginStore.onFatal(event.payload));
}
