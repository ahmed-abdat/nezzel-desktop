import { listen } from '@tauri-apps/api/event';
import { ROW_EVENTS } from '../../generated/contract';
import { useMediaStore } from '../../stores/media/media';
import { MediaDestinationPayload } from '../types/destination';

export function registerDestinationListeners() {
  const mediaStore = useMediaStore();

  void listen<MediaDestinationPayload>(ROW_EVENTS.mediaDestination, (event) => {
    mediaStore.processMediaDestinationPayload(event.payload);
  });
}
