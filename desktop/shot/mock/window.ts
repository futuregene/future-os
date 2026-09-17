/** Mock of `@tauri-apps/api/window`. */
export enum UserAttentionType {
  Critical = 1,
  Informational = 2,
}

async function noop() {}
const unlisten = async () => () => {};

export function getCurrentWindow() {
  return {
    label: "main",
    setTitle: noop,
    startDragging: noop,
    isFullscreen: async () => false,
    isMaximized: async () => false,
    requestUserAttention: noop,
    setFocus: noop,
    show: noop,
    close: noop,
    onResized: unlisten,
    onCloseRequested: unlisten,
    onFocusChanged: unlisten,
    onMoved: unlisten,
    onScaleChanged: unlisten,
    scaleFactor: async () => 2,
    innerSize: async () => ({ width: 1440, height: 900, toLogical: () => ({ width: 1440, height: 900 }) }),
    outerSize: async () => ({ width: 1440, height: 930, toLogical: () => ({ width: 1440, height: 930 }) }),
  };
}

export function getAllWindows() {
  return [getCurrentWindow()];
}

export function currentMonitor() {
  return Promise.resolve({ name: "Mock", size: { width: 2880, height: 1800 }, scaleFactor: 2 });
}
