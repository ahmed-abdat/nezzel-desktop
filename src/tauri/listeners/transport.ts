import { listen } from '@tauri-apps/api/event';
import { ROW_EVENTS } from '../../generated/contract';
import { invoke } from '@tauri-apps/api/core';
import type { EngineAvailability, MediaTransport } from '../types/transport';

export const listenForTransport = (handler: (payload: MediaTransport) => void) =>
  listen<MediaTransport>(ROW_EVENTS.mediaTransport, event => handler(event.payload));

export const engineAvailability = () => invoke<EngineAvailability>('engine_availability');
