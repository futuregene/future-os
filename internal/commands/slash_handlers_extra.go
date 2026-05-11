package commands

import (
"fmt"
"strings"
)

func handleLogin() (string, error) {
	return `Authentication:
Set the LLM_API_KEY environment variable to authenticate with your provider.
For Anthropic: set ANTHROPIC_API_KEY
For custom providers: set LLM_BASE_URL

Example:
  export LLM_API_KEY=***
  export LLM_BASE_URL=https://api.openai.com
  export LLM_MODEL=gpt-4o`, nil
}

func handleLogout() (string, error) {
	return `To logout, unset API keys:
  unset LLM_API_KEY
  unset ANTHROPIC_API_KEY
  unset OPENAI_API_KEY

Or use the auth tool to clear stored credentials.`, nil
}

// Sentinel-based commands: return specially-formatted strings that the
// caller (main.go / engine) interprets to trigger actual operations.

func handleNew(ctx *Context) (string, error) {
	return "NEW_SESSION", nil
}

func handleCompact(ctx *Context) (string, error) {
	if len(ctx.Messages) < 2 {
		return "", fmt.Errorf("Nothing to compact (no messages yet)")
	}
	if ctx.IsCompacting {
		return "", fmt.Errorf("compaction already in progress")
	}
	return fmt.Sprintf("COMPACT:%s", ctx.CurrentSessionID), nil
}

func handleResume(args []string, ctx *Context) (string, error) {
	if len(args) == 0 {
		return "RESUME_SELECTOR", nil
	}
	return fmt.Sprintf("RESUME:%s", args[0]), nil
}

func handleReload(ctx *Context) (string, error) {
	if ctx.IsStreaming {
		return "", fmt.Errorf("Wait for the current response to finish before reloading.")
	}
	if ctx.IsCompacting {
		return "", fmt.Errorf("Wait for compaction to finish before reloading.")
	}
	return "RELOAD", nil
}

func handleFork(args []string, ctx *Context) (string, error) {
	if ctx.CurrentSessionID == "" {
		return "", fmt.Errorf("no active session to fork")
	}
	entryID := "latest"
	if len(args) > 0 {
		entryID = args[0]
	}
	return fmt.Sprintf("FORK:%s:%s", ctx.CurrentSessionID, entryID), nil
}

func handleClone(ctx *Context) (string, error) {
	if ctx.CurrentSessionID == "" {
		return "", fmt.Errorf("no active session to clone")
	}
	return "CLONE:" + ctx.CurrentSessionID, nil
}

// handleTree builds a tree view from session entries.
func handleTree(ctx *Context) (string, error) {
	if ctx.CurrentSessionID == "" {
		return "", fmt.Errorf("no active session")
	}
	tree := buildSessionTree(ctx.SessionEntries, ctx.CurrentSessionID)
	return tree, nil
}

func buildSessionTree(entries []SessionEntry, rootID string) string {
	if len(entries) == 0 {
		return fmt.Sprintf("Session tree for %s:\n  (no entries)", rootID)
	}
	var sb strings.Builder
	sb.WriteString(fmt.Sprintf("Session tree for %s:\n", rootID))
	sb.WriteString(fmt.Sprintf("  %d entries\n", len(entries)))

	// Group by type
	counts := make(map[string]int)
	for _, e := range entries {
		counts[e.Type]++
	}
	for typ, count := range counts {
		sb.WriteString(fmt.Sprintf("    %s: %d\n", typ, count))
	}
	return sb.String()
}

func handleQuit() (string, error) {
	return "QUIT", nil
}
