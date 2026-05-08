# Go Test Coverage Report

**Module:** `github.com/earendil-works/pi-go`
**Go version:** go1.26.1
**Generated:** 2026-05-08
**Overall coverage:** 17.3% of statements

---

## Executive Summary

| Category | Count | Packages |
|----------|-------|----------|
| **0% Coverage (no tests)** | 15 | cmd/pi, cmd/pi-web, internal/agent, internal/auth, internal/commands, internal/engine, internal/events, internal/exec, internal/extensions, internal/llm, internal/models, internal/session, internal/tui, internal/webui, pkg/types |
| **Partial coverage** | 4 | internal/prompt, internal/settings, internal/skills, internal/tools |
| **Full coverage (100%)** | 1 | internal/compaction |
| **Total packages** | 20 | (17.3% overall statement coverage) |

---

## 1. Packages with 0% Coverage

### 1.1 `cmd/pi` — CLI entrypoint
- **File:** `cmd/pi/main.go` (lines 22–491)
- **Functions (all 0%):** `main`, `handleSlashCommand`, `handleExport`, `handleCompact`, `handleNew`, `handleResume`, `handleReload`, `handleSession`, `handleCopy`, `handleName`, `handleScopedModels`, `handleChangelog`, `handleLogin`, `nonempty`, `newUserMsg`
- **Why uncovered:** No test file exists. This package contains CLI argument parsing and command dispatch logic that would require integration-level testing with mock flags and stdin/stdout capture.
- **Risk:** Medium — CLI entrypoint bugs could prevent the tool from starting.

### 1.2 `cmd/pi-web` — Web UI entrypoint
- **File:** `cmd/pi-web/main.go` (line 12)
- **Functions (all 0%):** `main`
- **Why uncovered:** No test file exists. Web server startup.
- **Risk:** Low — thin wrapper that calls into `internal/webui`.

### 1.3 `internal/agent` — Core agent loop
- **File:** `internal/agent/loop.go` (lines 31–200)
- **Functions (all 0%):** `NewLoop`, `RunStreamingWithMessages`, `RunStreaming`, `drainSteering`, `executeTool`, `newSystemMessage`, `newUserMessage`, `newToolResult`, `truncate`
- **Why uncovered:** No test file. This is the central orchestrator tying together LLM, tools, session, and events — making it the most critical untested package.
- **Risk:** **HIGH** — All agent reasoning and tool execution flows through this package. Bugs here affect every interaction.

### 1.4 `internal/auth` — API key management
- **File:** `internal/auth/auth.go` (lines 30–164)
- **Functions (all 0%):** `IsExpired`, `LoadAuthStorage`, `SaveAuthStorage`, `save`, `DefaultAuthPath`, `GetKey`, `SetKey`, `ListProviders`
- **Why uncovered:** No test file. Involves filesystem I/O for JSON credential storage.
- **Risk:** Medium — credential corruption could prevent API access.

### 1.5 `internal/commands` — Slash command handler
- **File:** `internal/commands/slash.go` (lines 37–222)
- **Functions (all 0%):** `Handle`, `handleImport`, `handleFork`, `handleClone`, `handleTree`, `handleHotkeys`, `handleSettings`, `handleShare`, `handleQuit`
- **Why uncovered:** No test file. Slash command router for the TUI and CLI.
- **Risk:** Medium — broken slash commands degrade UX.

### 1.6 `internal/engine` — Engine configuration and tool sets
- **File:** `internal/engine/engine.go` (lines 61–337)
- **Functions (all 0%):** `Default`, `applyDefaults`, `NewEngine`, `detectProvider`, `thinkingLevelToBudget`, `CodingTools`, `ReadOnlyTools`, `Getwd`, `Chdir`
- **Why uncovered:** No test file. Engine wires together models, tools, and configuration.
- **Risk:** Medium — misconfiguration could break tool availability or provider detection.

### 1.7 `internal/events` — Event bus system
- **File:** `internal/events/events.go` (lines 41–276)
- **Functions (all 0%):** `NewEventBus`, `Subscribe`, `Unsubscribe`, `Emit`, `Close`, `AgentStart`, `AgentEnd`, `TurnStart`, `TurnEnd`, `MessageStart`, `MessageEnd`, `TextStart`, `TextDelta`, `TextEnd`, `ThinkingStart`, `ThinkingDelta`, `ThinkingEnd`, `ToolCallStart`, `ToolCallDelta`, `ToolCallEnd`, `ToolStart`, `ToolEnd`, `UsageEvent`, `ErrorEvent`, `EmitStreamingEvents`
- **Why uncovered:** No test file. Pure in-memory pub/sub system — easily testable with no external dependencies.
- **Risk:** **HIGH** — The event bus is the backbone of all streaming UI updates. Race conditions or missing events cause silent failures.

### 1.8 `internal/exec` — Bash execution
- **File:** `internal/exec/bash.go` (lines 92–346)
- **Functions (all 0%):** `applyDefaults`, `ExecuteBash`, `sanitizeBinary`, `KillProcessTree`, `GetShellEnv`, `FormatBashResult`
- **Why uncovered:** No test file. Shell execution requires careful mocking.
- **Risk:** **HIGH** — Shell command execution is the most security-sensitive code path.

### 1.9 `internal/extensions` — Plugin/extension system
- **Files:** `extensions.go`, `loader.go`, `plugin_loader.go`, `plugin_loader_unsupported.go`, `registry.go`, `runner.go`
- **Functions (all 0%, ~30 functions):** `Info`, `Warn`, `Error`, `Debug`, `NewEventBus`, `Subscribe`, `Unsubscribe`, `Publish`, `NewExtensionContext`, `RegisterTool`/`RegisterSlashCommand`/`RegisterPrompt` (multiple overloads), `LoadExtensions`, `scanDirectory`, `loadExtensionFile`, `loadManifestFile`, `loadPluginFile`, `loadGoPlugin`, `NewExtensionRunner`, `Load`, `Add`, `InitAll`, `initOne`, `DeinitAll`, `Run`, `RunExtension`, `Initialized`, `SortExtensionsByName`, `GetAllTools`/`GetAllSlashCommands`/`GetAllPrompts`, `GetTool`, `HasTool`, `GetSlashCommand`, `HasSlashCommand`, `GetPrompt`
- **Why uncovered:** No test file. Complex multi-file package with registry pattern, plugin loading, and manifest parsing.
- **Risk:** **HIGH** — Extension system failures could cause crashes or missing tools.

### 1.10 `internal/llm` — LLM client providers
- **Files:** `client.go`, `anthropic.go`, `options.go`
- **Functions (all 0%):** `NewClient`, `applyCacheControl`, `StreamChat`, `doOpenAIRequest`, `streamSSE` (x2), `Chat`, `NewAnthropicClient`, `resolveBudget`, `convertMessages`, `extractPlainTextFromContent`, `extractPlainText`, `convertTools`, `StreamChatWithOptions`, `doAnthropicRequest`, `DefaultStreamOptions`, `merge`, `EstimateCost`
- **Why uncovered:** No test file. HTTP streaming requires mocking HTTP servers.
- **Risk:** **HIGH** — LLM client is the core integration point. Failures break all AI interactions.

### 1.11 `internal/models` — Model registry
- **File:** `internal/models/models.go` (lines 52–415)
- **Functions (all 0%):** `LoadRegistry`, `SaveRegistry`, `save`, `DefaultRegistryPath`, `BuiltinModels`, `builtinMap`, `ResolveModel`, `levenshteinDistance`, `min3`
- **Why uncovered:** No test file. Model resolution with fuzzy matching.
- **Risk:** Medium — wrong model resolution could cause API errors.

### 1.12 `internal/session` — Session persistence
- **File:** `internal/session/session.go` (lines 63–638)
- **Functions (all 0%):** `GetBaseURL`, `SetBaseURL`, `NewManager`, `DefaultDir`, `GenerateID`, `GenerateEntryID`, `encodeCWD`, `sessionDir`, `sessionPath`, `ModelChangeEntry`, `CompactionEntry`, `sessionInfoEntry`, `parseSessionInfo`, `List`, `Load`, `loadFromPath`, `Save`, `Delete`, `AddEntry`, `ForEachEntry`, `BuildContext`, `entryToMessage`, `MessageToEntry`, `MessagesToEntries`, `ForkSession`, `MigrateSessionV1ToV2`, `MigrateSessionV2ToV3`, `MigrateSession`, `SetVersion`
- **Why uncovered:** No test file. Session filesystem I/O and JSON serialization — testable with temp directories.
- **Risk:** **HIGH** — Session corruption loses entire conversation history.

### 1.13 `internal/tui` — Terminal UI
- **Files:** `tui.go`, `autocomplete.go`, `keybindings.go`, `render.go`, `run.go`
- **Functions (all 0%):** `Run`, `runREPL`, `printPrompt`, `readInput`, `RunInteractive`, `redrawLine`, `redrawDropdown`, `submit`, `displayStream`, `handleSlashCommand`, `readKey`, `runCLI`, `newUserMsg`, `mustMarshal`, `NewAutoComplete`, `SetSlashCommands`, `Activate`, `Deactivate`, `Filter`, `Next`, `Prev`, `Selected`, `FilteredCount`, `View`, `Complete`, `completeSlashCommand`, `completeFilePath`, `filterByPrefix`, `commonPrefix`, `maxLen`, `padRight`, `Match`, `DefaultKeybindings`, `ConfigurableKeybindings`, `style`/color vars, `RenderDiff`, `setOf`, `HighlightSyntax`, `highlightLine`, `isWordChar`, `findStringEnd`, `RenderToolCall`, `RenderToolResult`, `truncateStr`, `RenderKeyBar`
- **Why uncovered:** No test file. Terminal I/O is hard to test but render/logic functions are testable.
- **Risk:** Medium — TUI bugs affect UX but not core functionality. CLI mode may still work.

### 1.14 `internal/webui` — Web UI server
- **File:** `internal/webui/server.go` (lines 36–220)
- **Functions (all 0%):** `NewServer`, `ServeHTTP`, `handleStatic`, `handleChat`, `handleSessions`, `handleSessionByID`, `handleSettings`
- **Why uncovered:** No test file. HTTP handlers — testable with `httptest`.
- **Risk:** Medium — broken web UI degrades user experience.

### 1.15 `pkg/types` — Shared type definitions
- **File:** `pkg/types/types.go` (99 lines)
- **Functions:** Pure data types — `Message`, `TextContent`, `ToolCall`, `ToolCallFn`, `Usage`, `StreamEvent`, `ToolDef`, `FunctionDef`, `AgentTool`, `AgentConfig`, `Model`, `LLMProvider` interface
- **Why uncovered:** No test file. No executable functions, only types/structs/interfaces.
- **Risk:** Low — type correctness is verified at compile time across all consumers.

---

## 2. Packages with Partial Coverage

### 2.1 `internal/prompt` — System prompt builder
**Overall:** ~93.3% (3/4 functions fully or near-fully covered)

| Function | Coverage | Lines | Purpose | Why Partially Uncovered |
|----------|----------|-------|---------|------------------------|
| `BuildPrompt` | 100% | 31–106 | Assembles system prompt from options | ✔ Fully tested |
| `deduplicateGuidelines` | 100% | 109–123 | Removes duplicate guidelines | ✔ Fully tested |
| `extractFirstSentence` | 66.7% | 127–132 | Returns first sentence (up to `.`) | Only the "no period" branch (line 131) uncovered — input without a period never tested |
| `escapeXML` | 100% | 135–142 | XML entity escaping | ✔ Fully tested |

**Test file:** `internal/prompt/prompt_test.go`

---

### 2.2 `internal/settings` — Settings management
**Overall:** ~54% (many functions with partial coverage)

| Function | Coverage | Lines | Purpose | Why Partially Uncovered |
|----------|----------|-------|---------|------------------------|
| `LoadSettings` | 88.9% | 199 | Load settings from JSON file | Missing edge case (likely file read error path) |
| `SaveSettings` | 75.0% | 217 | Save settings to JSON file | Missing error paths in write/lock flow |
| `MergeSettings` | 76.6% | 250 | Merge global + project settings | Complex merge not exhaustively tested |
| `GetDefaultPaths` | 77.8% | 396 | Returns global/project settings paths | `os.UserHomeDir()` or `os.Getwd()` error paths untested |
| `LoadAll` | 75.0% | 415 | Load both global and project, merge | Error paths for missing files |
| `clone` | 82.1% | 436 | Deep copy Settings struct | Some pointer fields not covered in clone path |
| `cloneTerminalSettings` | 75.0% | 509 | Deep copy terminal settings | One or both pointer fields untested |
| `cloneRetrySettings` | 83.3% | 523 | Deep copy retry settings | Null path not covered |
| `cloneBranchSummarySettings` | 83.3% | 534 | Deep copy branch summary settings | Null path not covered |
| `mergeThinkingBudgets` | 14.3% | 568 | Merge thinking budget overrides | Only one of 5 fields tested |
| `mergeImageSettings` | 90.0% | 591 | Merge image settings | Minor edge case missing |
| `mergeTerminalSettings` | 16.7% | 608 | Merge terminal settings | Only null-override path tested |
| `mergeRetrySettings` | 68.8% | 628 | Merge retry settings | Some merge paths untested |
| `mergeBranchSummarySettings` | 20.0% | 654 | Merge branch summary settings | Most merge logic untested |
| `mergeMarkdownSettings` | 25.0% | 671 | Merge markdown settings | Most paths untested |
| `mergeWarningSettings` | 25.0% | 685 | Merge warning settings | Most paths untested |
| `lockPath` | 0.0% | 723 | Derives lock file path | **Completely untested** |
| `LockSettings` | 0.0% | 730 | Acquires file lock | **Completely untested** |
| `UnlockSettings` | 0.0% | 746 | Releases file lock | **Completely untested** |
| `IsLocked` | 0.0% | 755 | Checks lock status | **Completely untested** |
| `MigrateSettings` | 0.0% | 768 | Migrates settings format | **Completely untested** |
| `Reload` | 0.0% | 783 | Reloads from disk | **Completely untested** |
| `ApplyOverrides` | 0.0% | 803 | Applies CLI overrides | **Completely untested** |
| `boolPtr` | 100% | 704 | Helper to get bool pointer | ✔ Fully tested |
| `copyStringSlice` | 100% | 709 | Helper to copy strings | ✔ Fully tested |

**Test file:** `internal/settings/settings_test.go`

**Key gaps:** File locking (`LockSettings`, `UnlockSettings`, `IsLocked`, `lockPath`) — 0% covered, risk of concurrent access bugs. Settings migration and reload also untested. Merge functions for nested configs are barely covered (14–25%).

---

### 2.3 `internal/skills` — Skill discovery and management
**Overall:** ~87% (most functions high coverage)

| Function | Coverage | Lines | Purpose | Why Partially Uncovered |
|----------|----------|-------|---------|------------------------|
| `DiscoverSkills` | 90.0% | 39–80 | Walks directories for SKILL.md | Error path for `filepath.WalkDir` unwalkable dir |
| `parseSkillFile` | 87.5% | 84–107 | Parses SKILL.md frontmatter | File read error path untested |
| `parseFrontmatter` | 100% | 112–150 | Extracts YAML frontmatter | ✔ Fully tested |
| `matchKeyValue` | 100% | 154–174 | Key-value line parser | ✔ Fully tested |
| `ValidateSkill` | 92.3% | 178–207 | Validates skill metadata | Missing one validation branch (name-too-long or consecutive hyphens check) |
| `FormatSkillsXML` | 100% | 211–225 | Formats skills as XML | ✔ Fully tested |
| `ResolveCollisions` | 100% | 232–276 | Resolves skill name conflicts | ✔ Fully tested |
| `sourceRank` | 100% | 279–288 | Priority ranking by source | ✔ Fully tested |
| `escapeXML` | 100% | 291–298 | XML escaping | ✔ Fully tested |
| `expandHome` | 25.0% | 301–313 | Expands `~` to home dir | **Only the non-tilde path tested** — all home-expansion branches (lines 303–311) uncovered |

**Test file:** `internal/skills/skills_test.go`

**Key gap:** `expandHome` is 25% covered — only the fallthrough path (line 312) is tested. The actual home-directory expansion including the `os.UserHomeDir()` error path is completely missed.

---

### 2.4 `internal/tools` — Tool definitions and handlers
**Overall:** ~42% (highly variable, several 0% functions)

| Function | Coverage | Lines | Purpose | Why Partially Uncovered |
|----------|----------|-------|---------|------------------------|
| `BashTool` | 78.9% | 19–109 | Bash execution tool handler | Timeout/spill paths untested |
| `ReadTool` | **0.0%** | 112–209 | File read tool (text + image) | **Completely untested** |
| `WriteTool` | **0.0%** | 212–250 | File write tool | **Completely untested** |
| `normalize` | **0.0%** | 268–274 | Fuzzy matching normalization | **Completely untested** |
| `buildByteMapper` | **0.0%** | 279–335 | Byte index mapper for fuzzy matching | **Completely untested** |
| `generateUnifiedDiff` | **0.0%** | 342–429 | Unified diff generator | **Completely untested** |
| `EditTool` | **0.0%** | 449–641 | Enhanced edit with fuzzy matching | **Completely untested** |
| `sortMatches` | **0.0%** | 641–663 | Sort match regions | **Completely untested** |
| `GrepTool` | 72.7% | 664–710 | Grep tool handler (top-level) | Some branches uncovered |
| `grepViaRipgrep` | 95.2% | 711–748 | Ripgrep-based grep | Very well tested |
| `parseRipgrepJSON` | 92.1% | 749–835 | Parse ripgrep JSON output | Well tested |
| `grepViaSystem` | **0.0%** | 836–875 | Fallback system grep | **Completely untested** |
| `parseSystemGrepOutput` | **0.0%** | 876–955 | Parse system grep output | **Completely untested** |
| `truncateLine` | 66.7% | 956–962 | Truncate long output lines | No-truncation branch untested |
| `AllTools` | **0.0%** | 964–974 | Returns all available tools | **Completely untested** |
| `FindTool` | 71.4% | tools_find.go:15 | Find tool by name | Some branches uncovered |
| `LsTool` | 81.6% | tools_ls.go:16 | Directory listing tool | Some branches uncovered |
| `NewFileMutationQueue` | **0.0%** | file_mutation_queue.go:23 | Create mutation queue | **Completely untested** |
| `Enqueue` | **0.0%** | file_mutation_queue.go:32 | Queue file mutation | **Completely untested** |
| `worker` | **0.0%** | file_mutation_queue.go:51 | Background worker | **Completely untested** |
| `Close` | **0.0%** | file_mutation_queue.go:59 | Close mutation queue | **Completely untested** |
| `NewOutputAccumulator` | **0.0%** | output_accumulator.go:21 | Create output accumulator | **Completely untested** |
| `Write` | **0.0%** | output_accumulator.go:27 | Write to accumulator | **Completely untested** |
| `Snapshot` | **0.0%** | output_accumulator.go:60 | Snapshot accumulator | **Completely untested** |
| `Close` (accum) | **0.0%** | output_accumulator.go:73 | Close accumulator | **Completely untested** |
| `ResolvePath` | **0.0%** | path_utils.go:14 | Resolve relative paths | **Completely untested** |
| `IsWithin` | **0.0%** | path_utils.go:39 | Check if path is within dir | **Completely untested** |

**Test files:** `bash_test.go`, `tools_grep_test.go`, `tools_ls_test.go`

**Key gaps:**
- **Read tool** and **Write tool** — 0% coverage. Core I/O operations completely untested.
- **Edit tool** — 0% coverage. The most complex tool with fuzzy matching, unified diffs, and multi-edit support has zero test coverage.
- **Fuzzy matching helpers** (`normalize`, `buildByteMapper`, `generateUnifiedDiff`, `sortMatches`) — all 0%. These underpin the edit tool.`
- **File mutation queue** and **output accumulator** — entire files 0%. Background processing infrastructure untested.
- **System grep fallback** — 0%. The fallback path when ripgrep is unavailable is untested.
- **Path utilities** (`ResolvePath`, `IsWithin`) — 0%. Security-relevant path validation.

---

## 3. Fully Covered Package

### 3.1 `internal/compaction` — Context compaction
- **File:** `compaction.go`
- **Coverage:** 100% on all 8 functions
- **Functions:** `EstimateTokens`, `Compact`, `ExtractFileOperations`, `FormatFileOpsForSummary`, `estimateMessageTokens`, `extractTextContent`, `extractFilePath`, `summarize`, `formatCompactionEntry`
- **Test file:** `internal/compaction/compaction_test.go`
- ✅ This is the model package for test coverage in this project.

---

## 4. Compilation & Static Analysis

- **`go build ./...`**: ✅ PASSED — All packages compile successfully.
- **`go vet ./...`**: ✅ PASSED — No vet warnings.
- **Undefined symbols**: None found.
- **Import issues**: None found.

---

## 5. Risk Assessment Summary

| Risk Level | Packages |
|------------|----------|
| **CRITICAL** | `internal/agent`, `internal/llm`, `internal/session`, `internal/events`, `internal/exec`, `internal/extensions`, `internal/tools` (Read/Write/Edit) |
| **HIGH** | `internal/settings` (locking), `internal/tools` (path utils, mutation queue) |
| **MEDIUM** | `cmd/pi`, `internal/auth`, `internal/commands`, `internal/engine`, `internal/models`, `internal/tui`, `internal/webui` |
| **LOW** | `cmd/pi-web`, `pkg/types` |

---

## 6. Recommended Priority for Test Coverage

1. **`internal/tools`** — Write tests for `ReadTool`, `WriteTool`, and `EditTool` handlers first (core user-facing tools, 0% covered)
2. **`internal/llm`** — Add mock HTTP server tests for `doOpenAIRequest`, `doAnthropicRequest`, and SSE streaming
3. **`internal/session`** — Filesystem-based tests using `t.TempDir()` for save/load/delete/context building
4. **`internal/events`** — Pure in-memory tests for pub/sub, ordering, and close behavior
5. **`internal/agent`** — Mock LLM + mock tools to test the agent loop end-to-end
6. **`internal/settings`** — Complete merge function coverage and add lock tests
7. **`internal/skills`** — Add `expandHome` tests with mocked `os.UserHomeDir`
8. **`internal/prompt`** — Add test case for `extractFirstSentence` with no period
