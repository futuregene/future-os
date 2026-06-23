---
name: future-subagent
version: 1.0.0
description: >
  Spawn parallel subagents via `future run` to perform work concurrently in isolated sessions.
  Use when tasks benefit from parallelism (multiple independent searches, multi-file analysis,
  parallel code reviews), when work should be isolated with different models/tools/permissions,
  when forking a session to explore alternative approaches, or when decomposing large tasks
  into independently executable pieces. Trigger phrases: "run in parallel", "spawn subagent",
  "fork and try", "delegate to subagent", "fan out", "do these concurrently",
  "use a subagent to", "create a workflow".
allowed-tools: Bash(future:*)
category: orchestration
---

# Subagent — Parallel & Delegated Execution

Spawn independent `future run` subagents for parallel, isolated, or delegated work. Each
subagent runs in its own session (or fork) with independently configurable model, thinking
level, tools, and permission level.

## When to Use Subagents

**Use subagents when:**

| Situation | Rationale |
|-----------|-----------|
| Multiple independent searches or lookups | Parallel subagents complete faster than sequential |
| Multi-file analysis (each file independent) | No shared state needed between analyses |
| Alternative approaches to same problem | Fork and try different models/strategies |
| Dangerous or high-permission work | Isolate in a restricted subagent |
| Large decomposition (each piece self-contained) | Pieces can execute concurrently |
| Cross-session exploration | Explore without polluting current session context |
| Adversarial review | Independent reviewer with fresh perspective |
| Budget-sensitive exploration | Short-lived subagents with different cost profiles |

**Do NOT use subagents when:**

- Work depends on current conversation context (use inline tools)
- Tasks are trivial (< 3 tool calls) — overhead not worth it
- Sequential dependency requires output of previous step
- User explicitly asked for single-session work

## Core Commands

### Basic Subagent

```bash
future run "prompt text here"
```

This creates a new ephemeral session, runs the prompt, streams output to stdout,
and exits. No session is saved by default.

### Configured Subagent

```bash
future run \
  --model sonnet:high \
  --thinking high \
  --tools read,bash,grep \
  --permission workspace \
  --no-session \
  "Analyze the codebase structure and report findings"
```

### Fork-Based Subagent

To continue work from the current session at a specific point:

```bash
# First, fork from the current session
future run --fork <entry-id> \
  --model sonnet:high \
  "Try a different approach: refactor the auth module using JWT instead"
```

Get entry IDs from the TUI (`/fork` command shows forkable user messages).

### JSON Output Mode

For structured results that need parsing:

```bash
result=$(future run --mode json "List all typescript files and their exports")
echo "$result" | jq '.text'
```

### Continue Recent Session

```bash
future run --continue "Continue the previous analysis with a different perspective"
```

## Parallel Execution Patterns

### Pattern 1: Simple Fan-Out

Run multiple independent analyses in parallel:

```bash
# Launch all in background, collect results
future run "Analyze auth.ts for security issues" > /tmp/audit-auth.txt &
future run "Analyze api.ts for security issues" > /tmp/audit-api.txt &
future run "Analyze db.ts for security issues" > /tmp/audit-db.txt &
wait

# Synthesize findings
cat /tmp/audit-*.txt
```

### Pattern 2: Multi-Model Comparison

Compare outputs from different models on the same prompt:

```bash
PROMPT="Review src/auth.rs for security vulnerabilities"

future run --model sonnet:high "$PROMPT" > /tmp/review-sonnet.txt &
future run --model haiku "$PROMPT" > /tmp/review-haiku.txt &
future run --model opus:xhigh "$PROMPT" > /tmp/review-opus.txt &
wait

echo "=== Sonnet Review ===" && cat /tmp/review-sonnet.txt
echo "=== Haiku Review ===" && cat /tmp/review-haiku.txt
echo "=== Opus Review ===" && cat /tmp/review-opus.txt
```

### Pattern 3: Fork-and-Compare

Fork from the same session to explore alternative approaches:

```bash
# Get the current entry ID from state first
future run --fork <entry-1> \
  "Implement feature X using approach A: inheritance-based" \
  > /tmp/approach-a.txt &

future run --fork <entry-1> \
  "Implement feature X using approach B: composition-based" \
  > /tmp/approach-b.txt &

wait
```

### Pattern 4: Scoped Isolation

Isolate dangerous or high-permission work:

```bash
# Safe: read-only analysis with workspace permission
future run \
  --tools read,grep,ls \
  --permission workspace \
  "Audit all SQL queries for injection vulnerabilities" \
  > /tmp/audit.txt

# Risky: file modification in isolated subagent
future run \
  --tools read,write,edit,bash \
  --permission workspace \
  --no-session \
  "Migrate all console.log to structured logging in src/"
```

### Pattern 5: Depth-First Decomposition

For large tasks, decompose and process in waves:

```bash
# Wave 1: Understand (parallel readers)
future run --tools read,ls "Map the module structure of src/" > /tmp/module-map.txt &
future run --tools read,ls "Map the routing structure" > /tmp/routes-map.txt &
future run --tools read,ls "Map the data models" > /tmp/models-map.txt &
wait

# Wave 2: Analyze (parallel, using Wave 1 results)
future run --tools read "@$(cat /tmp/module-map.txt) Find coupling issues" > /tmp/coupling.txt &
future run --tools read "@$(cat /tmp/routes-map.txt) Find missing auth guards" > /tmp/auth-gaps.txt &
wait

# Wave 3: Synthesize
future run --tools read \
  "@/tmp/coupling.txt @/tmp/auth-gaps.txt Synthesize into a single refactoring plan" \
  > /tmp/refactor-plan.md
```

### Pattern 6: Adversarial Review

Use independent subagents for quality assurance:

```bash
# Primary implementation (in current session or subagent)
future run --model sonnet:high --tools read,write,edit,bash \
  "Implement the payment processing module" \
  > /tmp/implementation.txt

# Independent reviews from different angles
future run --model haiku \
  "Review /tmp/implementation.txt for correctness issues" \
  > /tmp/review-correctness.txt &

future run --model haiku \
  "Review /tmp/implementation.txt for security issues" \
  > /tmp/review-security.txt &

future run --model sonnet:high \
  "Review /tmp/implementation.txt for performance issues" \
  > /tmp/review-performance.txt &
wait

# Check if any review found critical issues
grep -i "critical\|vulnerability\|bug" /tmp/review-*.txt
```

## Configuration Reference

### Model & Thinking

```
--model <id>            Model ID, e.g., sonnet, haiku, opus, deepseek-v4-flash
--model sonnet:high     Model:thinking shorthand (colon-separated)
--thinking <level>      off | minimal | low | medium | high | xhigh
```

### Tool Scoping

```
--tools read,bash       Enable only listed tools
--no-tools, -nt         Disable all tools (read-only analysis)
--no-builtin-tools      Disable built-in tools, keep extensions
```

Common tool sets:

| Purpose | Tool Set |
|---------|----------|
| Code analysis | `read,grep,ls` |
| Code modification | `read,write,edit,bash` |
| Full development | `read,write,edit,bash,grep,ls` |
| Web research | `--no-builtin-tools` (uses future-web skill) |
| Safe read-only | `read,ls` |

### Permission Levels

```
--permission all        Full access (default) — approve all tools
--permission workspace   Workspace-scoped — restrict write outside workspace
--permission none       No tool execution — thinking/analysis only
```

### Session Management

```
--session <id>          Run in specific existing session
--continue, -c          Continue most recent session
--fork <entry-id>       Fork new session from entry point
--no-session            Ephemeral (don't save) — default for subagents
--cwd <dir>             Working directory override
```

### Output Modes

```
--mode text             Stream text to stdout (default)
--mode json             Output structured JSON with events array
--verbose               Progress info to stderr (useful for debugging subagents)
```

## Output Collection

### Capture to File

```bash
future run "Explain the database schema" > /tmp/schema-explanation.md
```

### Capture with Error Handling

```bash
if future run --mode json "$PROMPT" > /tmp/result.json 2>/tmp/error.log; then
  text=$(jq -r '.text' /tmp/result.json)
  echo "Success: $text"
else
  echo "Subagent failed: $(cat /tmp/error.log)"
fi
```

### JSON Output Structure

```json
{
  "sessionId": "abc123",
  "model": "sonnet",
  "thinkingLevel": "high",
  "text": "The full accumulated text output...",
  "messages": [
    { "type": "text_chunk", "text": "..." },
    { "type": "tool_call", "tool_name": "read", "tool_input": "..." },
    { "type": "agent_end" }
  ]
}
```

Use `jq` to extract specific fields:
```bash
text=$(future run --mode json "$PROMPT" | jq -r '.text')
model=$(future run --mode json "$PROMPT" | jq -r '.model')
```

## Concurrency Limits

Be mindful of system resources when spawning subagents:

- **Default safe limit**: 4-8 concurrent subagents (each runs an LLM call)
- **Memory**: Each `future run` is a lightweight Node.js process
- **API rate limits**: Check your provider's rate limits before large fan-outs
- **Token costs**: Parallel subagents consume tokens concurrently — faster but not cheaper

For bounded parallelism with a progress indicator:

```bash
max_jobs=4
running=0

run_subagent() {
  future run "$1" > "$2" 2>/dev/null
}

for prompt_file in prompts/*.txt; do
  output="${prompt_file%.txt}.out"
  run_subagent "$(cat "$prompt_file")" "$output" &
  running=$((running + 1))

  if [ $running -ge $max_jobs ]; then
    wait -n  # Wait for any one to finish
    running=$((running - 1))
  fi
done
wait  # Wait for remaining
```

## Skill Interaction

Subagents can leverage other skills via the agent's skill discovery:

```bash
# Subagent with deep-research capability
future run \
  --model sonnet:high \
  --thinking high \
  "Using deep-research, investigate the current state of WebAssembly GC support"
```

Skills are automatically available to subagents (discovered from
`~/.agents/skills/`, `.future/agent/skills/`, `~/.future/agent/skills/`).

## File I/O Between Subagents

Subagents communicate through the filesystem:

```bash
# Producer subagent
future run --tools read,write \
  "Generate a comprehensive API specification and write to /tmp/api-spec.md" \
  > /tmp/producer.log

# Consumer subagent (reads the file via @file syntax)
future run --tools read,write,edit \
  "@/tmp/api-spec.md Implement the TypeScript types for this API spec" \
  > /tmp/consumer.log
```

Use `@file` syntax to include file contents in prompts, or use `/tmp/` for
intermediate artifacts that don't need to persist.

## Pitfalls & Best Practices

### Do

- **Use `--no-session`** for throwaway subagents (default behavior — keeps session list clean)
- **Set appropriate `--permission`** — never give a subagent more access than it needs
- **Use `/tmp/` for intermediate outputs** — avoids cluttering the workspace
- **Set a generous `--model`** for complex reasoning tasks
- **Use `--thinking low` or `minimal`** for simple mechanical tasks to save cost
- **Verify subagent output** before acting on it — subagents can hallucinate too
- **Use `--verbose`** when debugging subagent failures
- **Collect exit codes** — `$?` tells you if the subagent succeeded

### Don't

- **Don't spawn subagents for trivial work** (< 3 tool calls expected)
- **Don't nest subagents deeply** — each level adds latency and cost
- **Don't assume subagent output is correct** — always verify critical findings
- **Don't use the same session for concurrent subagents** — each needs its own
- **Don't forget to `wait`** after backgrounding subagents
- **Don't use subagents when the user expects interactive feedback**

## Troubleshooting

| Symptom | Likely Cause | Fix |
|---------|-------------|-----|
| `future run` hangs | Agent not running | `future agent status` → `future agent start` |
| Subagent returns empty | No tools available | Add `--tools read` at minimum |
| "session does not exist" | Wrong session ID | Use `--continue` or omit `--session` |
| Permission denied errors | Permission level too strict | Use `--permission workspace` or `all` |
| Slow subagent response | Thinking level too high | Reduce `--thinking` for simple tasks |
| Output truncated | Stream closed early | Use `--mode json` for complete capture |

## Reference: Common Subagent Profiles

Save these as shell aliases or reference patterns:

```bash
# Lightweight code reviewer
alias future-review='future run --model haiku --tools read,grep,ls --permission workspace --no-session'

# Deep analyzer
alias future-analyze='future run --model sonnet:high --thinking high --tools read,grep,ls,bash --no-session'

# Safe implementer (workspace-only)
alias future-implement='future run --model sonnet:high --tools read,write,edit,bash --permission workspace --no-session'

# Quick fact-checker
alias future-check='future run --model haiku --thinking off --tools read --no-session'

# Full researcher
alias future-research='future run --model sonnet:high --thinking high --tools read,grep,ls,bash,write --no-session'
```
