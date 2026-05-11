package commands

import (
	"fmt"
	"strings"
)

// Context holds all context needed by slash commands.
type Context struct {
	CWD              string
	SessionDir       string
	SettingsDir      string // ~/.pi directory for changelog, config
	CurrentSessionID string
	SettingsPath     string
	SettingsJSON     string
	Model            string
	BaseURL          string
	SystemPrompt     string

	// Stats for /session
	SessionName string
	Messages    []Message
	TokenUsage  *TokenUsage
	TotalCost   float64

	// Guards for concurrent operations
	IsStreaming  bool
	IsCompacting bool

	// Session entries for /fork /clone /tree
	SessionEntries []SessionEntry
}

// Message holds a single session message for stats and /copy.
type Message struct {
	Role    string
	Content string
}

// TokenUsage holds cumulative token counts.
type TokenUsage struct {
	Input      int
	Output     int
	CacheRead  int
	CacheWrite int
	Total      int
}

// SessionEntry is a tree entry for fork/tree/clone operations.
type SessionEntry struct {
	ID      string
	Type    string
	Content string
	ModelID string
}

// ---------------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------------

func Handle(input string, ctx *Context) (string, error) {
	parts := strings.Fields(input)
	if len(parts) == 0 {
		return "", nil
	}
	cmd := strings.ToLower(parts[0])
	args := parts[1:]

	switch cmd {
	case "/model":
		return handleModel(args, ctx)
	case "/baseurl":
		return handleBaseURL(args, ctx)
	case "/memory":
		return handleMemory(ctx)
	case "/clear":
		return handleClear(args, ctx)
	case "/settings":
		return handleSettings(ctx)
	case "/scoped-models":
		return handleScopedModels(ctx)
	case "/export":
		return handleExport(args, ctx)
	case "/import":
		return handleImport(args, ctx)
	case "/share":
		return handleShare(ctx)
	case "/copy":
		return handleCopy(ctx)
	case "/name":
		return handleName(args, ctx)
	case "/session":
		return handleSession(ctx)
	case "/changelog":
		return handleChangelog(ctx)
	case "/hotkeys":
		return handleHotkeys()
	case "/help":
		return handleHelp()
	case "/fork":
		return handleFork(args, ctx)
	case "/clone":
		return handleClone(ctx)
	case "/tree":
		return handleTree(ctx)
	case "/login":
		return handleLogin()
	case "/logout":
		return handleLogout()
	case "/new":
		return handleNew(ctx)
	case "/compact":
		return handleCompact(ctx)
	case "/resume":
		return handleResume(args, ctx)
	case "/reload":
		return handleReload(ctx)
	case "/quit":
		return handleQuit()
	default:
		return "", fmt.Errorf("unknown command: %s (try /hotkeys for list)", cmd)
	}
}
