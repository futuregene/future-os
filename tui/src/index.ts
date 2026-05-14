/**
 * xihu TypeScript TUI entry point.
 * Usage: node dist/index.js [--socket <path>] [--port <port>] [--url <url>]
 *
 * Examples:
 *   node dist/index.js --socket /tmp/xihu.sock
 *   node dist/index.js --port 7890
 */

// ─── Public API re-exports ────────────────────────────────────────────────

// Core types
export { type Component, type Focusable, type OverlayHandle, type OverlayOptions, type OverlayLayout, type InputListener, Container, resolveOverlayLayout, isFocusable, CURSOR_MARKER } from "./tui.js";

// Components
export { App } from "./app.js";
export { ChatArea, type ChatMessage } from "./components/chat-area.js";
export { Editor } from "./components/editor.js";
export { Footer, type FooterData } from "./components/footer.js";
export { SelectList, type SelectItem } from "./components/select-list.js";
export { AutocompletePopup, type AutocompleteItem, AutocompleteManager, SlashCommandProvider, FilePathProvider, type SlashCommand, type AutocompleteProvider, type AutocompleteContext } from "./components/autocomplete.js";
export { MarkdownRenderer, type MarkdownTheme, type DefaultTextStyle } from "./components/markdown.js";
export { Image, type ImageOptions, type ImageTheme } from "./components/image.js";
export { Box } from "./components/box.js";
export { Text } from "./components/text.js";
export { TruncatedText } from "./components/truncated-text.js";
export { Spacer } from "./components/spacer.js";
export { Loader, type LoaderIndicatorOptions } from "./components/loader.js";
export { CancellableLoader } from "./components/cancellable-loader.js";
export { SettingsList, type SettingItem, type SettingsListTheme, type SettingsListOptions } from "./components/settings-list.js";

// Theme
export { C, DARK_THEME, type Theme, fg, bg, bold, dim, italic, underline, strikethrough, reset, fgRaw, bgRaw, boldRaw, dimRaw, italicRaw, underlineRaw, strikethroughRaw, reverseRaw, style, thinkingColor } from "./theme.js";

// Key handling
export { parseKey, isKeyRelease, isKeyRepeat, decodePrintableKey, Key, type KeyId, modifiedKey, ctrlKey } from "./keys.js";
export { KeybindingManager, type KeybindingContext, type KeybindingEntry } from "./keybindings.js";

// Utilities
export { visibleWidth, wrapTextWithAnsi, applyBackgroundToLine, truncateToWidth, sliceByColumn, stripAnsiCodes, extractAnsiCode, AnsiCodeTracker, normalizeTerminalOutput } from "./utils.js";

// Terminal
export { NodeTerminal, SYNC_BEGIN, SYNC_END, MOUSE_TRACK_ON, MOUSE_TRACK_OFF } from "./tui.js";

// RPC
export { RpcClient } from "./rpc/client.js";

// ─── Main entry point ────────────────────────────────────────────────────

import { App } from "./app.js";

const args = process.argv.slice(2);
let grpcAddr = "localhost:50051";

for (let i = 0; i < args.length; i++) {
  if (args[i] === "--grpc-addr" && i + 1 < args.length) {
    grpcAddr = args[i + 1];
    i++;
  }
}

const app = new App(grpcAddr);

process.on("SIGINT", async () => {
  await app.stop();
  process.exit(0);
});

process.on("SIGTERM", async () => {
  await app.stop();
  process.exit(0);
});

app.start().catch((err) => {
  console.error("Fatal error:", err);
  process.exit(1);
});
