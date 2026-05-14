/**
 * xihu TypeScript TUI entry point.
 * Usage: node dist/index.js --grpc-addr <addr> [options]
 *
 * Options:
 *   --grpc-addr <addr>     gRPC server address (default: localhost:50051)
 *   --session <id>        Connect to specific session
 *   --continue, -c         Continue most recent session
 *   --resume, -r          Resume a session (show picker)
 *   --fork <id>           Fork from a session
 *
 * Examples:
 *   node dist/index.js --grpc-addr localhost:50051
 *   node dist/index.js --grpc-addr localhost:50051 --session 20260514-140838-1a064f
 *   node dist/index.js --continue
 *   node dist/index.js --fork 20260514-140838-1a064f
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
export { GrpcClient } from "./rpc/grpc-client.js";

// ─── CLI Arguments ─────────────────────────────────────────────────────────

interface CliArgs {
  grpcAddr: string;
  session: string | null;
  continue: boolean;
  resume: boolean;
  fork: string | null;
}

function parseArgs(args: string[]): CliArgs {
  const result: CliArgs = {
    grpcAddr: process.env.XIHU_GRPC_ADDR ?? "localhost:50051",
    session: null,
    continue: false,
    resume: false,
    fork: null,
  };

  for (let i = 0; i < args.length; i++) {
    const arg = args[i];
    switch (arg) {
      case "--grpc-addr":
        if (i + 1 < args.length) {
          result.grpcAddr = args[++i];
        }
        break;
      case "--session":
        if (i + 1 < args.length) {
          result.session = args[++i];
        }
        break;
      case "--continue":
      case "-c":
        result.continue = true;
        break;
      case "--resume":
      case "-r":
        result.resume = true;
        break;
      case "--fork":
        if (i + 1 < args.length) {
          result.fork = args[++i];
        }
        break;
      case "--help":
      case "-h":
        console.log(`xihu TUI

Usage: node dist/index.js [options]

Options:
  --grpc-addr <addr>   gRPC server address (default: localhost:50051)
  --session <id>       Connect to specific session
  --continue, -c       Continue most recent session
  --resume, -r         Resume a session (show picker)
  --fork <id>           Fork from a session
  --help, -h            Show this help
`);
        process.exit(0);
        break;
    }
  }

  return result;
}

// ─── Main entry point ──────────────────────────────────────────────────────

import { App } from "./app.js";

const args = parseArgs(process.argv.slice(2));

console.log(`Connecting to gRPC server at ${args.grpcAddr}`);

const app = new App(args.grpcAddr, {
  session: args.session,
  continue: args.continue,
  resume: args.resume,
  fork: args.fork,
});

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
