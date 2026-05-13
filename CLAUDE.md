# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

Go 1.26.1+, module `github.com/huichen/xihu`.

## Build/Run/Test

```bash
make build          # Build both CLI (xihu) and web (xihu-web) binaries to bin/
make build-cli      # Build CLI only (faster iteration)
make build-web      # Build web server only
make run            # Build and run CLI (pass ARGS="--help" for flags)
make run-web        # Build and run web server (pass PORT=9090 for custom port)
make test           # All tests (timeout 120s)
make test-verbose   # All tests with verbose output
make test-race      # All tests with race detector
make test-cover     # All tests with coverage profile
make test-cover-html # Coverage in browser
make lint           # go vet
make fmt            # go fmt
make fmt-check      # Check formatting (useful for CI/pre-commit)
make generate-models # Regenerate model catalog from external APIs
make install        # Install binaries to GOPATH/bin
make help           # Show all targets
```

Run a single package's tests:
```bash
go test -count=1 -v ./internal/skills/
```

Builds use `CGO_ENABLED=0` for static binaries with no libc dependency.

### TypeScript TUI (`tui/`)

```bash
cd tui && npm run build   # Type-check with tsc, output to dist/
cd tui && npm run dev     # Run directly with bun (no pre-build needed)
cd tui && npm start       # Run compiled JS with bun
```

The TUI is aligned with pi-mono's TUI architecture. Reference: `tui/pi-vs-xihu-comparison.md` tracks all differences across 12 categories.

## Architecture

**xihu** is a Go AI coding assistant CLI (similar to Claude Code). The Go binary is a backend — the TypeScript TUI (`tui/`) provides the terminal interface. Two frontend modes:

1. **TypeScript TUI** (`tui/`) — the primary terminal UI. The Go binary runs in server mode (default when no prompt + TTY, or explicitly with `--mode server --socket <path>`), and the TS TUI connects via RPC over Unix socket or HTTP.
2. **Web UI** (`internal/webtui/`) — browser-based chat interface with SSE streaming, served by `xihu-web`.

Two entry points:

- `cmd/xihu/main.go` — CLI entry point: parses 30+ flags, resolves model/provider/API key/auth, configures session management (fork/resume/continue), discovers skills, builds system prompt. In a TTY with no prompt, auto-starts server mode on `/tmp/xihu.sock`. Also supports non-interactive print/RPC/server mode. The `config` subcommand (`cmd/xihu/config.go`) prints resource configuration as text.
- `cmd/xihu-web/main.go` — Web server entry point (minimal, port via `PORT` env, defaults to 8080)

### Frontend modes and data flow

```
                    ┌──────────────────────────────┐
                    │     AgentSession + Engine     │
                    │  (agentsession / engine pkgs) │
                    └──────────┬───────────────────┘
                               │
               ┌───────────────┼───────────────┐
               │                               │
        TypeScript TUI                   Web UI (SSE)
        (tui/ via RPC)              (internal/webtui/)
        default mode               xihu-web binary
```

### TUI architecture (`tui/src/`)

The TUI follows pi's `Component`/`Container`/`Focusable` pattern with overlay stack and input pipeline:

**Core framework** (`tui.ts`):
- `Component` interface: `render(width)`, `handleInput?(data)`, `invalidate()`, `wantsKeyRelease?`
- `Focusable` interface: `focused: boolean` for IME cursor positioning via `CURSOR_MARKER`
- `Container` class: `addChild`/`removeChild`/`clear`, cascading `invalidate()`, `App extends Container`
- `OverlayHandle`: `hide()`, `setHidden()`, `focus()`, `unfocus()` — returned by `showOverlay()`
- `InputListener` pipeline: chainable input interceptors (consume/modify/pass-through)
- `NodeTerminal`: raw-mode stdin with `StdinBuffer` (10ms timeout, paste re-wrapping), Kitty CSI-u keyboard protocol detection, bracketed paste, SGR mouse tracking, synchronized output (`\x1b[?2026h/l`)

**Keyboard** (`keys.ts`, `keybindings.ts`):
- `parseKey()`: unified parser for Kitty CSI-u, xterm modifyOtherKeys, legacy escape sequences
- `KeyId` type + `Key` const object for compile-time autocomplete on key names
- `isKeyRelease()` / `isKeyRepeat()` detection
- `KeybindingManager`: key→action dispatch with context filtering, conflict detection, add/remove

**Rendering pipeline** (`app.ts` `doRender()`):
- Dual-phase scheduling: `process.nextTick` + `setTimeout` coalescing at ~60fps
- Differential rendering: computes changed-line ranges, sends minimal ANSI updates
- Viewport tracking: `previousViewportTop`, `maxLinesRendered`, `cursorRow`, `hardwareCursorRow`
- Overlay compositing: overlays merged into lines before diff, with anchor/percentage positioning
- Kitty image lifecycle: image ID tracking, orphan deletion, changed-line expansion for image rows
- Line resets: `\x1b[0m` appended per-line to prevent ANSI bleed

**Text processing** (`utils.ts`):
- `visibleWidth()`: Intl.Segmenter-based, emoji detection, CJK/east-asian-width, cache
- `wrapTextWithAnsi()`: word-boundary wrap preserving ANSI codes with `AnsiCodeTracker` state machine
- `applyBackgroundToLine()`: full-width background color with ANSI-reset-safe padding
- `truncateToWidth()`, `sliceByColumn()`: ANSI-aware column-based extraction
- `graphemeWidth()`: zero-width, emoji (flags/keycap/modifier/extended-pictographic), CJK
- `normalizeTerminalOutput()`: tab→3 spaces, Thai/Lao AM decomposition

**Components** (15 in `components/`):

| Component | Role |
|-----------|------|
| `ChatArea` | Scrollable chat view: user/assistant/tool/system messages, thinking blocks, streaming |
| `Editor` | Multi-line editor (750 lines): undo/redo, kill ring, word navigation, paste markers, scroll indicators, border, padding, history |
| `Footer` | Status bar: pwd, model, thinking, token stats, context usage with color thresholds |
| `MarkdownRenderer` | Full markdown: StrictStrikethroughTokenizer, OSC 8 hyperlinks, style prefix tracking, word wrap, render cache, cell-wrapping tables |
| `AutocompleteManager` | Provider-based: `SlashCommandProvider` (commands + model/session args), `FilePathProvider` (fs.readdir), debounce + AbortController |
| `SelectList` | Scrollable selection list with keyboard navigation |
| `SettingsList` | Settings list with value cycling, search filtering, submenu support |
| `Image` | Kitty/iTerm2 image rendering with fallback to text placeholder |
| `Box` | Container with padding + background color for child components |
| `Text` | Multi-line text with word wrap, padding, background, render cache |
| `TruncatedText` | Single-line truncated text with ellipsis + padding |
| `Spacer` | Configurable empty-line vertical spacing |
| `Loader` | Animated braille spinner, configurable frames/interval, callback-based updates |
| `CancellableLoader` | Loader + Escape-to-cancel + AbortSignal |
| `Container` | Generic container with `addChild`/`removeChild` (used by Box and App) |

**Theme** (`theme.ts`):
- 256-color constants (`C` object) + `Theme` interface with 20+ fields
- Style functions: `fg()`, `bg()`, `bold()`, `dim()`, `italic()`, `underline()`, `strikethrough()` — each auto-appends RESET
- Raw variants: `fgRaw()`, `boldRaw()`, etc. — no auto-RESET for composable theme building
- `style(text, ...fns)` composer: chains multiple style functions, single trailing RESET

**RPC** (`rpc/client.ts`):
- HTTP + SSE transport to Go backend; Unix socket support via `--socket` flag
- Methods: `prompt()`, `abort()`, `getState()`, `getAvailableModels()`, `setModel()`, `cycleModel()`, `cycleThinkingLevel()`, `listSessions()`, `switchSession()`, `newSession()`, `compact()`

### Go core components

**`internal/engine/engine.go`** — Unifies provider detection, settings merging, session creation, tool config, and agent loop into a single `Engine` struct. `NewEngine()` auto-detects provider from base URL (Anthropic vs OpenAI-compatible), resolves thinking budgets, and wires auto-compaction as a `TransformContext` hook on the agent loop.

**`internal/agentsession/`** — AgentSession: central abstraction for agent lifecycle shared across CLI, web, and RPC modes. Wraps the Engine and adds session-level control: event subscription, prompt/steer/followUp, model management, compaction, and session statistics.

**`internal/agent/loop.go`** — The agentic loop: call LLM → receive stream events (text/thinking/tool calls) → execute tools → repeat until no tool calls or max turns. Supports interrupt/abort via context cancellation, steering messages via channel queues, and parallel tool execution.

**`internal/llm/client.go`** — OpenAI-compatible streaming client using the official `openai-go` SDK. Handles thinking/reasoning content extraction from `ExtraFields` (DeepSeek `reasoning_content`), cache token parsing, tool call accumulation from streaming chunks, and context cancellation for interrupt support.

**`internal/llm/anthropic.go`** — Anthropic-specific client using `anthropic-sdk-go` (parallel to OpenAI client, selected when base URL contains `anthropic.com`).

**`internal/rpc/`** — Headless RPC server using JSONL over stdin/stdout (`--mode rpc`) and Unix socket / TCP (`--mode server`). Commands in (message, new_session, set_model, compact, etc.), responses and AgentSessionEvents out. The socket server (`server_socket.go`) handles multiple concurrent client connections with per-session agent state.

**`pkg/rpcclient/`** — Go client library for programmatically consuming xihu in RPC mode. Talks JSONL over stdin/stdout to an `xihu --mode rpc` process. Provides typed methods for all RPC commands (message, set_model, compact, etc.) and parses streamed AgentSessionEvents.

**`internal/session/session.go`** — Conversation persistence as JSONL files in `~/.xihu/sessions/<encoded-cwd>/`. Sessions are tree-structured (each entry has a ParentID), enabling forks and branches. `BuildContext()` walks leaf-to-root through the tree, handling compaction entries (replaced with summary system messages). Supports migration between format versions.

**`internal/compaction/compaction.go`** — Context compaction: estimates tokens (chars/4 heuristic), finds safe cut points (user/assistant message boundaries, never tool results), summarizes file operations (reads/writes), and replaces older messages with a compacted user message.

**`internal/tools/`** — Default coding tool set is 4 tools (bash, read, write, edit). grep/ls/find also exist as optional tools (used by `AllTools()` or `ReadOnlyTools()`). Each returns an `AgentTool` with a JSON Schema definition, handler, and guidelines. The edit tool uses Unicode NFKC normalization + smart quote replacement for fuzzy matching, supports single-edit and multi-edit (array) modes, overlap detection, and no-change skipping. `internal/engine/engine_tools.go` defines `CodingTools()` (the 4 defaults) and `ReadOnlyTools()` (read, grep, ls, find).

**`internal/commands/slash.go`** — 25 slash commands dispatched by the `/command` handler. Some return sentinel strings (e.g. `NEW_SESSION`, `RESUME:<id>`, `FORK:<id>:<entry>`, `COMPACT:`, `QUIT`) that the main loop interprets to trigger session lifecycle operations.

**`internal/skills/skills.go`** — Discovers skills by walking directories (`~/.xihu/skills/`, `.xihu/skills/`, `~/.agents/skills/`, `~/.pi/agent/skills/`) for `SKILL.md` files with YAML frontmatter (name, description, disable-model-invocation). Resolves naming collisions (project > user priority).

**`internal/prompt/`** — System prompt builder with template support and skill injection. Also discovers context files (CLAUDE.md, AGENTS.md) from project and global directories.

**`internal/settings/settings.go`** — Deep-merge settings from `~/.xihu/settings.json` (global) and `.xihu/settings.json` (project), with project overriding global. Supports settings locking (O_EXCL .lock files) and migration between format versions.

**`internal/extensions/`** — Plugin architecture: extensions can register tools, slash commands, and prompts. Supports Go plugins (.so) and JSON config-based extensions, with an internal event bus for pub/sub between extensions. Go plugin loading is guarded by build tags (`linux || darwin` only; `plugin_loader_unsupported.go` for other platforms).

**`internal/events/`** — Event types and EventBus for bridging agent streaming events to frontends (thinking deltas, tool calls, tool results, usage stats).

**`internal/modelregistry/`** — Model discovery and registration. Embeds a built-in model catalog, supports runtime overrides via configuration, and resolves provider→model chains.

**`internal/models/`** — Parses `models.json` provider configuration files (provider-centric format with model lists and capabilities). Includes builtin model catalog (`models_builtin.go`) — **generated** by `make generate-models` (via `internal/modelregistry/generate_models.go`) from external APIs (models.dev, OpenRouter, Vercel AI Gateway). Also fuzzy model matching via Levenshtein distance.

**`internal/config/`** — Resource configuration manager: discovers and watches `models.json` files across global/project locations, auto-reloads on changes, resolves resource patterns for provider endpoints. The `xihu config` subcommand prints discovered resources as text.

**`internal/auth/`** — Reads API credentials from `~/.xihu/auth.json` (keyed by provider), with fallback to `~/.pi/agent/auth.json` for migration compatibility.

**`internal/exec/`** — Standalone bash executor with ANSI stripping, binary sanitization, tail truncation, process tree killing, and AbortSignal support. Used by the bash tool.

**`internal/diagnostic/`** — Diagnostic events (warnings, errors, file collisions) emitted during operations.

**`internal/apiregistry/`** — Known API endpoint registry: maps provider names to base URLs for common providers (Anthropic, OpenAI, OpenRouter, DeepSeek, etc.).

**`internal/utils/`** — Shared utilities: version info (`version.go`), changelog display, MIME type detection.

**`pkg/types/`** — Shared types: `Message`, `ToolCall`, `StreamEvent`, `AgentTool`, `AgentConfig`, `LLMProvider` interface. Also `jsonschema.go` — generates JSON Schema from Go structs with `jsonschema:` struct tags via `types.SchemaOf[T]()`, mirroring pi-mono's TypeBox pattern. Used by all tools for parameter schema generation.

### Provider model

The `LLMProvider` interface (`StreamChat`) is the abstraction. Provider is auto-detected from the base URL:
- URL contains `anthropic.com` → `llm.NewAnthropicClient` (native Anthropic SDK)
- Everything else → `llm.NewClient` (OpenAI-compatible SDK)

API key resolution order: `--api-key` flag → `LLM_API_KEY` env → `ANTHROPIC_API_KEY` env → `OPENAI_API_KEY` env → `auth.json` (by provider) → `auth.json` default key.

### Session lifecycle

Sessions are JSONL files with tree-structured entries (each entry has an ID and optional ParentID). Entry types: `session_info`, `user`, `assistant`, `tool`, `compaction`, `model_change`, `thinking_level_change`, `branch_summary`, `label`, `custom`, `custom_message`. The `--session` flag can take a full ID, a path to a JSONL file, or a directory path. `--continue` resumes the most recent session. `--fork` creates a new session from a specific entry in a parent session.
