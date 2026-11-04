/** Load only the code authorized for this native window. Unknown windows stay inert. */
export async function mountWindow(label: string) {
  if (label === 'main') {
    const main = await import('./mainWindow');
    main.mountMainWindow();
    return undefined;
  }
  if (label === 'close-note') {
    const note = await import('./closeNote');
    return note.mountCloseNote();
  }
  if (label === 'capture-card' || label === 'capture-bar') {
    const capture = await import('./capturePanel');
    return capture.mountCapturePanel();
  }
  return undefined;
}
