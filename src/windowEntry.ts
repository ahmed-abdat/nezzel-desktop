import { CAPTURE_WINDOW, CLOSE_NOTE_WINDOW } from './generated/contract';

/** Load only the code authorized for this native window. Unknown windows stay inert. */
export async function mountWindow(label: string) {
  if (label === 'main') {
    const main = await import('./mainWindow');
    await main.mountMainWindow();
    return undefined;
  }
  if (label === CLOSE_NOTE_WINDOW) {
    const note = await import('./closeNote');
    return note.mountCloseNote();
  }
  if (label === CAPTURE_WINDOW.card || label === CAPTURE_WINDOW.bar) {
    const capture = await import('./capturePanel');
    return capture.mountCapturePanel();
  }
  return undefined;
}
