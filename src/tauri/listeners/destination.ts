import { listen } from '@tauri-apps/api/event';
import { useMediaStore } from '../../stores/media/media';
import { MediaDestinationPayload } from '../types/destination';

export function registerDestinationListeners() {
  const mediaStore = useMediaStore();

  void listen<MediaDestinationPayload>('media_destination', (event) => {
    mediaStore.processMediaDestinationPayload(event.payload);
  });
}
