# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

Go 1.26.1+, module `github.com/huichen/xihu`.

## Build/Run/Test

```bash
make build          # Build both CLI (xihu) and web (xihu-web) binaries to bin/
make run            # Build and run CLI (pass ARGS="--help" for flags)
make test           # All tests (24 test files, timeout 120s)
make test-verbose   # All tests with verbose output
make test-race      # All tests with race detector
make test-cover     # All tests with coverage profile
make test-cover-html # Coverage in browser
make lint           # go vet
make fmt            # go fmt
make help           # Show all targets
```

Run a single package's tests:
```bash
go test -count=1 -v ./internal/skills/
```

## Architecture

**xihu** is a Go AI coding assistant CLI (similar to Claude Code) with a Bubble Tea TUI. Two entry points:

- `cmd/xihu/main.go` — CLI entry point: parses 30+ flags, resolves model/provider/API key/auth, configures session management (fork/resume/continue), discovers skills, builds system prompt, and either launches the TUI or runs in non-interactive print/RPC/JSON mode. The `config` subcommand (`cmd/xihu/config.go`) launches a resource-configuration TUI for managing models.json and other resources.
- `cmd/xihu-web/main.go` — Web server entry point (minimal, port via `PORT` env, defaults to 8080)

### Core components

**`internal/engine/engine.go`** — Unifies provider detection, settings merging, session creation, tool config, and agent loop into a single `Engine` struct. `NewEngine()` auto-detects provider from base URL (Anthropic vs OpenAI-compatible), resolves thinking budgets, and wires auto-compaction as a `TransformContext` hook on the agent loop.

**`internal/agent/loop.go`** — The agentic loop: call LLM → receive stream events (text/thinking/tool calls) → execute tools → repeat until no tool calls or max turns (default 50). Supports interrupt/abort via context cancellation, steering messages via channel queues, and parallel tool execution.

**`internal/llm/client.go`** — OpenAI-compatible streaming client using the official `openai-go` SDK. Handles thinking/reasoning content extraction from `ExtraFields` (DeepSeek `reasoning_content`), cache token parsing, tool call accumulation from streaming chunks, and context cancellation for interrupt support.

**`internal/llm/anthropic.go`** — Anthropic-specific client using `anthropic-sdk-go` (parallel to OpenAI client, selected when base URL contains `anthropic.com`).

**`internal/tui/app.go`** — Bubble Tea TUI model. Sub-components: `ChatViewport` (message display), `Editor` (input with slash-command autocomplete), `Footer` (status bar with model/tokens/cost), `Overlay`, `Autocomplete`. Runs the agent in a goroutine with EventBus bridging for streaming text/thinking/tool events. Supports Enter to submit, Esc to abort, Shift+Tab to cycle thinking level, Ctrl+T to toggle thinking visibility, and slash commands.

**`internal/session/session.go`** — Conversation persistence as JSONL files in `~/.xihu/sessions/<encoded-cwd>/`. Sessions are tree-structured (each entry has a ParentID), enabling forks and branches. `BuildContext()` walks leaf-to-root through the tree, handling compaction entries (replaced with summary system messages). Supports migration between format versions.

**`internal/compaction/compaction.go`** — Context compaction: estimates tokens (chars/4 heuristic), finds safe cut points (user/assistant message boundaries, never tool results), summarizes file operations (reads/writes), and replaces older messages with a compacted user message.

**`internal/tools/`** — Default coding tool set is 4 tools (bash, read, write, edit). grep/ls/find also exist as optional tools (used by `AllTools()` or `ReadOnlyTools()`). Each returns an `AgentTool` with a JSON Schema definition, handler, and guidelines. The edit tool uses Unicode NFKC normalization + smart quote replacement for fuzzy matching, supports single-edit and multi-edit (array) modes, overlap detection, and no-change skipping. `internal/engine/engine_tools.go` defines `CodingTools()` (the 4 defaults) and `ReadOnlyTools()` (read, grep, ls, find).

**`internal/settings/settings.go`** — Deep-merge settings from `~/.xihu/settings.json` (global) and `.xihu/settings.json` (project), with project overriding global. Supports settings locking (O_EXCL .lock files) and migration between format versions.

**`internal/commands/slash.go`** — 25 slash commands dispatched by the `/command` handler. Some return sentinel strings (e.g. `NEW_SESSION`, `RESUME:<id>`, `FORK:<id>:<entry>`, `COMPACT:`, `QUIT`) that the main loop interprets to trigger session lifecycle operations.

**`internal/skills/skills.go`** — Discovers skills by walking directories (`~/.xihu/skills/`, `.xihu/skills/`, `~/.agents/skills/`, `~/.pi/agent/skills/`) for `SKILL.md` files with YAML frontmatter (name, description, disable-model-invocation). Resolves naming collisions (project > user priority).

**`internal/extensions/`** — Plugin architecture: extensions can register tools, slash commands, and prompts. Supports Go plugins (.so) and JSON config-based extensions, with an internal event bus for pub/sub between extensions. Go plugin loading is guarded by build tags (`linux || darwin` only; `plugin_loader_unsupported.go` for other platforms).

**`internal/events/`** — Event types and EventBus for bridging agent streaming events to the TUI (thinking deltas, tool calls, tool results, usage stats).

**`internal/prompt/`** — System prompt builder with template support and skill injection.

**`internal/agentsession/`** — AgentSession: central abstraction for agent lifecycle shared across CLI, web, and RPC modes. Wraps the Engine and adds session-level control: event subscription, prompt/steer/followUp, model management, compaction, and session statistics. The Go equivalent of pi-mono's AgentSession class.

**`internal/modelregistry/`** — Model discovery and registration (mirrors TS pi-mono's ModelRegistry). Embeds a built-in model catalog, supports runtime overrides via configuration, and resolves provider→model chains.

**`internal/auth/`** — Reads API credentials from `~/.xihu/auth.json` (keyed by provider), with fallback to `~/.pi/agent/auth.json` for migration compatibility.

**`internal/exec/`** — Standalone bash executor with ANSI stripping, binary sanitization, tail truncation, process tree killing, and AbortSignal support. Used by the bash tool.

**`internal/diagnostic/`** — Diagnostic events (warnings, errors, file collisions) emitted during operations.

**`internal/models/`** — Parses `models.json` provider configuration files (provider-centric format with model lists and capabilities). Includes builtin model catalog (`models_builtin.go`) and fuzzy model matching via Levenshtein distance.

**`internal/config/`** — Resource configuration manager: discovers and watches `models.json` files across global/project locations, auto-reloads on changes, resolves resource patterns for provider endpoints.

**`internal/apiregistry/`** — Known API endpoint registry: maps provider names to base URLs for common providers (Anthropic, OpenAI, OpenRouter, DeepSeek, etc.).

**`internal/utils/`** — Shared utilities: version info (`version.go`), changelog display, MIME type detection.

**`internal/webui/`** — Web UI served by `xihu-web`: an HTTP server that provides a browser-based chat interface, with static assets under `internal/webui/static/`.

**`internal/rpc/`** — Headless RPC server using JSONL over stdin/stdout. Mirrors pi-mono's RPC protocol: commands in (message, new_session, set_model, compact, etc.), responses and AgentSessionEvents out. Framing is strict `\n`-delimited JSONL.

**`pkg/types/`** — Shared types: `Message`, `ToolCall`, `StreamEvent`, `AgentTool`, `AgentConfig`, `LLMProvider` interface. Also `jsonschema.go` — generates JSON Schema from Go structs with `jsonschema:` struct tags via `types.SchemaOf[T]()`, mirroring pi-mono's TypeBox pattern. Used by all tools for parameter schema generation.

**`pkg/rpcclient/`** — Go client library for programmatically consuming xihu in RPC mode. Talks JSONL over stdin/stdout to an `xihu --mode rpc` process. Provides typed methods for all RPC commands (message, set_model, compact, etc.) and parses streamed AgentSessionEvents.

### Provider model

The `LLMProvider` interface (`StreamChat`) is the abstraction. Provider is auto-detected from the base URL:
- URL contains `anthropic.com` → `llm.NewAnthropicClient` (native Anthropic SDK)
- Everything else → `llm.NewClient` (OpenAI-compatible SDK)

API key resolution order: `--api-key` flag → `LLM_API_KEY` env → `ANTHROPIC_API_KEY` env → `OPENAI_API_KEY` env → `auth.json` (by provider) → `auth.json` default key.

### Session lifecycle

Sessions are JSONL files with tree-structured entries (each entry has an ID and optional ParentID). Entry types: `session_info`, `user`, `assistant`, `tool`, `compaction`, `model_change`, `thinking_level_change`, `branch_summary`, `label`, `custom`, `custom_message`. The `--session` flag can take a full ID, a path to a JSONL file, or a directory path. `--continue` resumes the most recent session. `--fork` creates a new session from a specific entry in a parent session.
