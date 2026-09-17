/** Mock of `@tauri-apps/api/webview`. */
async function noop() {}
const unlisten = async () => () => {};

export function getCurrentWebview() {
  return {
    label: "main",
    onDragDropEvent: async (handler: (event: { payload: any }) => void) => {
      // No native drag-and-drop in a browser; the listener stays subscribed so
      // the real component wiring is exercised, it just never fires.
      void handler;
      return unlisten();
    },
    listen: unlisten,
    setZoom: noop,
  };
}

export function getAllWebviews() {
  return [getCurrentWebview()];
}
