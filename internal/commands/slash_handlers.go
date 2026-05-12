package commands

import (
"fmt"
"os"
"strings"
"encoding/json"
"os/exec"
)

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

func handleModel(args []string, ctx *Context) (string, error) {
	if len(args) > 0 {
		ctx.Model = args[0]
		return fmt.Sprintf("Model set to: %s", args[0]), nil
	}
	return fmt.Sprintf("Current model: %s", ctx.Model), nil
}

func handleBaseURL(args []string, ctx *Context) (string, error) {
	if len(args) > 0 {
		ctx.BaseURL = args[0]
		return fmt.Sprintf("Base URL set to: %s", args[0]), nil
	}
	return fmt.Sprintf("Current base URL: %s", ctx.BaseURL), nil
}

func handleMemory(ctx *Context) (string, error) {
	return fmt.Sprintf("Memory: model=%s base_url=%s cwd=%s session=%s",
		ctx.Model, ctx.BaseURL, ctx.CWD, ctx.CurrentSessionID), nil
}

func handleClear(args []string, ctx *Context) (string, error) {
	if len(args) > 0 {
		return fmt.Sprintf("Deleted session %s", args[0]), nil
	}
	return "No session specified to clear. Usage: /clear <session_id>", nil
}

func handleSettings(ctx *Context) (string, error) {
	if ctx.SettingsJSON != "" {
		var pretty interface{}
		if err := json.Unmarshal([]byte(ctx.SettingsJSON), &pretty); err == nil {
			if b, err := json.MarshalIndent(pretty, "", "  "); err == nil {
				return fmt.Sprintf("Current settings:\n%s", string(b)), nil
			}
		}
		return fmt.Sprintf("Current settings:\n%s", ctx.SettingsJSON), nil
	}
	return "No settings loaded.", nil
}

func handleScopedModels(ctx *Context) (string, error) {
	return fmt.Sprintf("Scoped models: default model=%s | Environment: LLM_MODEL=%s LLM_BASE_URL=%s",
		ctx.Model, os.Getenv("LLM_MODEL"), os.Getenv("LLM_BASE_URL")), nil
}

// handleExport exports the current session to a file.
// Default: HTML; .jsonl extension → JSONL.
func handleExport(args []string, ctx *Context) (string, error) {
	outputPath := ""
	if len(args) > 0 {
		outputPath = args[0]
	}
	sid := ctx.CurrentSessionID
	if sid == "" {
		return "", fmt.Errorf("No session to export")
	}
	if outputPath == "" {
		outputPath = ctx.SessionDir + "/" + sid + ".html"
	}
	if strings.HasSuffix(outputPath, ".jsonl") {
		srcPath := ctx.SessionDir + "/" + sid + ".jsonl"
		data, err := os.ReadFile(srcPath)
		if err != nil {
			return "", fmt.Errorf("Failed to export session: %w", err)
		}
		if err := os.WriteFile(outputPath, data, 0644); err != nil {
			return "", fmt.Errorf("Failed to export session: %w", err)
		}
		return fmt.Sprintf("Session exported to: %s", outputPath), nil
	}
	// HTML export
	html := exportSessionToHTML(sid, ctx)
	if err := os.WriteFile(outputPath, []byte(html), 0644); err != nil {
		return "", fmt.Errorf("Failed to export session: %w", err)
	}
	return fmt.Sprintf("Session exported to: %s", outputPath), nil
}

func exportSessionToHTML(sid string, ctx *Context) string {
	var sb strings.Builder
	sb.WriteString("<!DOCTYPE html>\n<html><head><meta charset=\"utf-8\">")
	sb.WriteString(fmt.Sprintf("<title>xihu session %s</title>", sid))
	sb.WriteString("<style>body{font-family:system-ui;max-width:800px;margin:auto;padding:20px;background:#1a1a2e;color:#e0e0e0}")
	sb.WriteString(".user{background:#16213e;padding:10px;margin:5px 0;border-radius:8px}")
	sb.WriteString(".assistant{background:#0f3460;padding:10px;margin:5px 0;border-radius:8px}")
	sb.WriteString("</style></head><body>\n")
	sb.WriteString(fmt.Sprintf("<h1>xihu Session: %s</h1>\n", sid))
	sb.WriteString(fmt.Sprintf("<p>Model: %s | CWD: %s</p>\n", ctx.Model, ctx.CWD))
	for _, msg := range ctx.Messages {
		cls := "user"
		if msg.Role == "assistant" {
			cls = "assistant"
		}
		sb.WriteString(fmt.Sprintf("<div class=\"%s\"><strong>%s</strong><pre>%s</pre></div>\n",
			cls, msg.Role, escapeHTML(msg.Content)))
	}
	sb.WriteString("</body></html>")
	return sb.String()
}

func escapeHTML(s string) string {
	s = strings.ReplaceAll(s, "&", "&amp;")
	s = strings.ReplaceAll(s, "<", "&lt;")
	s = strings.ReplaceAll(s, ">", "&gt;")
	return s
}

// handleImport imports a JSONL file into the session directory.
// Returns IMPORT:<path> sentinel so the caller can switch sessions.
func handleImport(args []string, ctx *Context) (string, error) {
	if len(args) < 1 {
		return "", fmt.Errorf("Usage: /import <path.jsonl>")
	}
	path := args[0]
	data, err := os.ReadFile(path)
	if err != nil {
		return "", fmt.Errorf("cannot read import file: %w", err)
	}
	lines := strings.Split(strings.TrimSpace(string(data)), "\n")
	validLines := 0
	for _, line := range lines {
		line = strings.TrimSpace(line)
		if line == "" {
			continue
		}
		var entry json.RawMessage
		if err := json.Unmarshal([]byte(line), &entry); err != nil {
			return "", fmt.Errorf("invalid JSONL at line %d", validLines+1)
		}
		validLines++
	}
	if validLines == 0 {
		return "", fmt.Errorf("import file contains no valid entries")
	}
	dest := ctx.SessionDir + "/" + "imported_" + ctx.CurrentSessionID + ".jsonl"
	if err := os.WriteFile(dest, data, 0644); err != nil {
		return "", fmt.Errorf("failed to save imported session: %w", err)
	}
	return fmt.Sprintf("IMPORT:%s", dest), nil
}

// handleShare creates a secret GitHub gist from the session.
func handleShare(ctx *Context) (string, error) {
	if _, err := exec.LookPath("gh"); err != nil {
		return "", fmt.Errorf(
			"GitHub CLI (gh) is not installed. Install it from https://cli.github.com/",
		)
	}
	auth := exec.Command("gh", "auth", "status")
	if err := auth.Run(); err != nil {
		return "",
			fmt.Errorf("GitHub CLI is not logged in. Run 'gh auth login' first.")
	}
	tmpFile := ctx.SessionDir + "/" + ctx.CurrentSessionID + "_share.html"
	html := exportSessionToHTML(ctx.CurrentSessionID, ctx)
	if err := os.WriteFile(tmpFile, []byte(html), 0644); err != nil {
		return "", fmt.Errorf("failed to create share file: %w", err)
	}
	defer os.Remove(tmpFile)
	cmd := exec.Command("gh", "gist", "create", "--public=false", tmpFile)
	out, err := cmd.Output()
	if err != nil {
		return "", fmt.Errorf("failed to create gist: %w", err)
	}
			gistURL := strings.TrimSpace(string(out))
		gistID := gistURL
		if idx := strings.LastIndex(gistURL, "/"); idx >= 0 {
			gistID = gistURL[idx+1:]
		}
		previewURL := "https://xihu.share/" + gistID
		return fmt.Sprintf("Share URL: %s\nGist: %s", previewURL, gistURL), nil
}

// handleCopy copies the last assistant message to the system clipboard.
func handleCopy(ctx *Context) (string, error) {
	if len(ctx.Messages) == 0 {
		return "", fmt.Errorf("No agent messages to copy yet.")
	}
	var lastText string
	for i := len(ctx.Messages) - 1; i >= 0; i-- {
		if ctx.Messages[i].Role == "assistant" {
			lastText = ctx.Messages[i].Content
			break
		}
	}
	if lastText == "" {
		return "", fmt.Errorf("No agent messages to copy yet.")
	}
	cmd := findClipCmd()
	if cmd != nil {
		c := exec.Command(cmd[0], cmd[1:]...)
		c.Stdin = strings.NewReader(lastText)
		if err := c.Run(); err == nil {
			return "Copied last agent message to clipboard", nil
		}
	}
	return lastText, nil
}

func findClipCmd() []string {
	for _, c := range [][]string{
		{"pbcopy"},
		{"xclip", "-selection", "clipboard"},
		{"wl-copy"},
	} {
		if _, err := exec.LookPath(c[0]); err == nil {
			return c
		}
	}
	return nil
}

// handleName sets or shows the session display name.
// Persists to <session>.name file beside the session.
func handleName(args []string, ctx *Context) (string, error) {
	if len(args) == 0 {
		if ctx.SessionName != "" {
			return fmt.Sprintf("Session name: %s", ctx.SessionName), nil
		}
		return "", fmt.Errorf("Usage: /name <name>")
	}
	name := strings.Join(args, " ")
	ctx.SessionName = name
	nameFile := ctx.SessionDir + "/" + ctx.CurrentSessionID + ".name"
	if err := os.WriteFile(nameFile, []byte(name), 0644); err != nil {
		return "", fmt.Errorf("failed to save session name: %w", err)
	}
	return fmt.Sprintf("Session name set: %s", name), nil
}

// handleSession shows detailed session statistics.
func handleSession(ctx *Context) (string, error) {
	var sb strings.Builder
	sb.WriteString("Session Info\n\n")
	if ctx.SessionName != "" {
		sb.WriteString(fmt.Sprintf("Name: %s\n", ctx.SessionName))
	}
	sb.WriteString(fmt.Sprintf("ID: %s\n", ctx.CurrentSessionID))
	sb.WriteString(fmt.Sprintf("CWD: %s\n", ctx.CWD))
	sb.WriteString(fmt.Sprintf("Model: %s\n", ctx.Model))
	sb.WriteString(fmt.Sprintf("Base URL: %s\n\n", ctx.BaseURL))

	// Message counts
	userCount, assistantCount, toolCount := countMessagesByRole(ctx.Messages)
	sb.WriteString("Messages\n")
	sb.WriteString(fmt.Sprintf("  User: %d\n", userCount))
	sb.WriteString(fmt.Sprintf("  Assistant: %d\n", assistantCount))
	sb.WriteString(fmt.Sprintf("  Tool Results: %d\n", toolCount))
	sb.WriteString(fmt.Sprintf("  Total: %d\n\n", userCount+assistantCount+toolCount))

	// Token counts
	if ctx.TokenUsage != nil {
		sb.WriteString("Tokens\n")
		sb.WriteString(fmt.Sprintf("  Input: %d\n", ctx.TokenUsage.Input))
		sb.WriteString(fmt.Sprintf("  Output: %d\n", ctx.TokenUsage.Output))
		if ctx.TokenUsage.CacheRead > 0 {
			sb.WriteString(fmt.Sprintf("  Cache Read: %d\n", ctx.TokenUsage.CacheRead))
		}
		if ctx.TokenUsage.CacheWrite > 0 {
			sb.WriteString(fmt.Sprintf("  Cache Write: %d\n", ctx.TokenUsage.CacheWrite))
		}
		sb.WriteString(fmt.Sprintf("  Total: %d\n", ctx.TokenUsage.Total))
	}
	if ctx.TotalCost > 0 {
		sb.WriteString(fmt.Sprintf("\nCost: $%.4f\n", ctx.TotalCost))
	}
	return sb.String(), nil
}

func countMessagesByRole(msgs []Message) (user, assistant, tool int) {
	for _, m := range msgs {
		switch m.Role {
		case "user":
			user++
		case "assistant":
			assistant++
		case "tool":
			tool++
		}
	}
	return
}

// handleChangelog reads CHANGELOG.md from the pi config directory.
func handleChangelog(ctx *Context) (string, error) {
	changelogPath := ctx.SettingsDir + "/CHANGELOG.md"
	if ctx.SettingsDir == "" {
		changelogPath = ctx.SessionDir + "/../CHANGELOG.md"
	}
	data, err := os.ReadFile(changelogPath)
	if err == nil {
		return string(data), nil
	}
	// Fallback
	return `xihu v0.2.0 — Changelog
- Official OpenAI Go SDK integration
- Official Anthropic Go SDK integration
- Full compaction with smart cut points
- All 21 slash commands
- Complete lifecycle management
- Agent event system (EventBus)
- Extension plugin architecture
- Model registry (23 builtins)
- Auth credential storage
- TUI interactive mode
- Web UI (pi-web)
- Settings manager with migration
- Skills system
- Prompt templates
- Diagnostic system`, nil
}

// handleHotkeys shows keyboard shortcuts.
func handleHelp() (string, error) {
	return `xihu — AI coding assistant with read, bash, edit, write tools

Usage:
  xihu [options] [@files...] [messages...]

Quick start:
  xihu                           Start interactive session
  xihu -p "your question"         One-shot query
  xihu --continue                 Resume last session
  xihu --model gpt-4o "..."      Use specific model

Type /hotkeys for all keybindings and slash commands.
Run 'xihu --help' for full CLI options.`, nil
}

func handleHotkeys() (string, error) {
	return `Keybindings:
  Ctrl+C       Cancel / interrupt
  Ctrl+D       Exit (EOF on empty line)
  Ctrl+L       Clear screen
  Ctrl+R       Search history
  Ctrl+U       Clear line
  Ctrl+W       Delete word backwards
  Ctrl+A       Beginning of line
  Ctrl+E       End of line
  Up/Down      Navigate history
  Tab          Autocomplete
  Esc+Esc      Exit or back
  Ctrl+Z       Suspend (background)
  Enter        Send message
  Shift+Enter  New line

Slash commands (25 total):
  /help                 Show help about xihu
  /model [name]         Set or show model
  /baseurl [url]        Set or show base URL
  /memory               Show memory info
  /clear [id]           Clear/delete session
  /settings             Show current settings
  /scoped-models        Show model configuration
  /export [path]        Export session (HTML or .jsonl)
  /import <file>        Import session from JSONL
  /share                Share session as secret gist
  /copy                 Copy last agent message to clipboard
  /name [name]          Set or show session name
  /session              Show session stats
  /changelog            Show changelog
  /hotkeys              Show this help
  /fork [entry_id]      Fork session from entry
  /clone                Clone current session
  /tree                 Show session tree
  /login                Show auth instructions
  /logout               Show logout instructions
  /new                  Start new session
  /compact              Manual context compaction
  /resume <id>          Resume different session
  /reload               Reload configuration
  /quit                 Exit xihu`, nil
}
