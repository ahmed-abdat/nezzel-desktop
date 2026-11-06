import { listen } from '@tauri-apps/api/event';
import { ROW_EVENTS } from '../../generated/contract';
import { useMediaProgressStore } from '../../stores/media/progress';
import { MediaProgressCompletePayload, MediaProgressPayload, MediaProgressStagePayload } from '../types/progress';

export function registerProgressListeners() {
  const progressStore = useMediaProgressStore();

  void listen<MediaProgressPayload>(ROW_EVENTS.mediaProgress, (event) => {
    progressStore.processMediaProgressPayload(event.payload);
  });

  void listen<MediaProgressStagePayload>(ROW_EVENTS.mediaProgressStage, (event) => {
    progressStore.processMediaProgressStagePayload(event.payload);
  });

  void listen<MediaProgressCompletePayload>(ROW_EVENTS.mediaComplete, (event) => {
    progressStore.processMediaCompletePayload(event.payload);
  });
}
