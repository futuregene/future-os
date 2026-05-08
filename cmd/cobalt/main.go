package main

import (
	"context"
	"encoding/json"
	"flag"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/huichen/cobalt/internal/commands"
	"github.com/huichen/cobalt/internal/compaction"
	"github.com/huichen/cobalt/internal/engine"
	"github.com/huichen/cobalt/internal/session"
	"github.com/huichen/cobalt/internal/settings"
	"github.com/huichen/cobalt/internal/tui"
	"github.com/huichen/cobalt/pkg/types"
)

func main() {
	cont := flag.Bool("continue", false, "Continue the most recent session")
	resume := flag.String("resume", "", "Resume a specific session by ID")
	listSessions := flag.Bool("list-sessions", false, "List saved sessions")
	flag.Parse()

	cwd, err := os.Getwd()
	if err != nil {
		fmt.Fprintf(os.Stderr, "Error getting working directory: %v\n", err)
		os.Exit(1)
	}

	cfg, _ := settings.LoadAll()
	if cfg == nil {
		cfg = &settings.Settings{}
	}

	sessMgr := session.NewManager(session.DefaultDir(cwd))

	if *listSessions {
		sessions, err := sessMgr.List(cwd)
		if err != nil {
			fmt.Fprintf(os.Stderr, "Error listing sessions: %v\n", err)
			os.Exit(1)
		}
		if len(sessions) == 0 {
			fmt.Println("No sessions found.")
			return
		}
		fmt.Printf("%-20s  %-15s  %-10s  %s\n", "ID", "MODEL", "MSGS", "UPDATED")
		for _, s := range sessions {
			fmt.Printf("%-20s  %-15s  %-10d  %s\n",
				s.ID, s.Model, len(s.Entries), s.UpdatedAt.Format("01-02 15:04"))
		}
		return
	}

	apiKey := os.Getenv("LLM_API_KEY")
	baseURL := os.Getenv("LLM_BASE_URL")
	model := os.Getenv("LLM_MODEL")

	if baseURL == "" {
		if cfg.DefaultProvider == "anthropic" {
			baseURL = "https://api.anthropic.com"
		} else {
			baseURL = "https://api.openai.com"
		}
	}
	if model == "" {
		if cfg.DefaultModel != "" {
			model = cfg.DefaultModel
		} else if strings.Contains(baseURL, "anthropic.com") {
			model = "claude-sonnet-4-20250514"
		} else {
			model = "gpt-4o"
		}
	}

	// Resolve settings/pi dirs
	settingsDir := filepath.Join(os.Getenv("HOME"), ".pi")
	settingsGlobal, _ := settings.GetDefaultPaths()
	settingsPath := settingsGlobal

	var sess *session.Session
	if *resume != "" {
		var err error
		sess, err = sessMgr.Load(*resume, cwd)
		if err != nil {
			fmt.Fprintf(os.Stderr, "Error loading session %s: %v\n", *resume, err)
			os.Exit(1)
		}
		if os.Getenv("LLM_MODEL") == "" && sess.Model != "" {
			model = sess.Model
		}
		if os.Getenv("LLM_BASE_URL") == "" && sess.BaseURL != "" {
			baseURL = sess.BaseURL
		}
	} else if *cont {
		sessions, err := sessMgr.List(cwd)
		if err != nil || len(sessions) == 0 {
			fmt.Fprintf(os.Stderr, "No sessions to continue.\n")
			os.Exit(1)
		}
		sess = &sessions[0]
		if os.Getenv("LLM_MODEL") == "" && sess.Model != "" {
			model = sess.Model
		}
		if os.Getenv("LLM_BASE_URL") == "" && sess.BaseURL != "" {
			baseURL = sess.BaseURL
		}
		fmt.Fprintf(os.Stderr, "Continuing session %s (%d messages, model %s)\n\n",
			sess.ID, len(sess.Entries), model)
	} else {
		sess = &session.Session{
			ID:        session.GenerateID(),
			CWD:       cwd,
			Model:     model,
			BaseURL:   baseURL,
			CreatedAt: time.Now(),
		}
	}

	eng, err := engine.NewEngine(engine.EngineOptions{
		BaseURL:        baseURL,
		APIKey:         apiKey,
		Model:          model,
		CWD:            cwd,
		Settings:       cfg,
		SessionManager: sessMgr,
		MaxTurns:       cfg.MaxTurns,
	})
	if err != nil {
		fmt.Fprintf(os.Stderr, "Engine error: %v\n", err)
		os.Exit(1)
	}
	eng.Session = sess
	eng.Loop.Model = model

	cmdCtx := &commands.Context{
		CWD:              cwd,
		SessionDir:       session.DefaultDir(cwd),
		SettingsDir:      settingsDir,
		CurrentSessionID: sess.ID,
		SettingsPath:     settingsPath,
		Model:            model,
		BaseURL:          baseURL,
		SystemPrompt:     eng.Loop.SystemPrompt,
	}

	userPrompt := strings.Join(flag.Args(), " ")

	if userPrompt == "" && isTerminal() {
		err := tui.Run(eng.Loop, sessMgr, sess, "", cmdCtx.Model, cmdCtx.BaseURL)
		if err != nil {
			fmt.Fprintf(os.Stderr, "TUI error: %v\n", err)
			os.Exit(1)
		}
		return
	}

	if userPrompt == "" {
		fmt.Fprintf(os.Stderr, "Usage: pi [flags] <prompt>\n")
		fmt.Fprintf(os.Stderr, "Flags: --continue, --resume <id>, --list-sessions\n")
		fmt.Fprintf(os.Stderr, "\nEnvironment: LLM_API_KEY, LLM_BASE_URL, LLM_MODEL\n")
		os.Exit(1)
	}

	if strings.HasPrefix(userPrompt, "/") {
		result, err := commands.Handle(userPrompt, cmdCtx)
		if err != nil {
			fmt.Fprintf(os.Stderr, "Error: %v\n", err)
			os.Exit(1)
		}
		if handled := processSentinel(result, eng, sessMgr, sess, cmdCtx); handled {
			return
		}
		fmt.Println(result)
		return
	}

	ctx := context.Background()
	var allMessages []types.Message
	if len(sess.Entries) > 0 {
		allMessages = session.BuildContext(sess.Entries)
	}
	allMessages = append(allMessages, newUserMsg(userPrompt))

	result, finalMessages, err := eng.Loop.RunStreamingWithMessages(ctx, allMessages, func(text string) {
		fmt.Print(text)
	})
	if err != nil {
		fmt.Fprintf(os.Stderr, "\nError: %v\n", err)
		os.Exit(1)
	}
	_ = result
	fmt.Println()

	saveSession(sessMgr, sess, finalMessages, model, baseURL)
}

// ---------------------------------------------------------------------------
// Interactive REPL
// ---------------------------------------------------------------------------

func isTerminal() bool {
	fi, _ := os.Stdin.Stat()
	return (fi.Mode() & os.ModeCharDevice) != 0
}

// ---------------------------------------------------------------------------
// Sentinel processing
// ---------------------------------------------------------------------------

func processSentinel(result string, eng *engine.Engine, sessMgr *session.Manager, sess *session.Session, cmdCtx *commands.Context) bool {
	switch {
	case result == "NEW_SESSION":
		*sess = session.Session{
			ID:        session.GenerateID(),
			CWD:       sess.CWD,
			Model:     sess.Model,
			BaseURL:   sess.BaseURL,
			CreatedAt: time.Now(),
		}
		cmdCtx.CurrentSessionID = sess.ID
		fmt.Fprintf(os.Stderr, "New session started: %s\n", sess.ID)

	case strings.HasPrefix(result, "RESUME:"):
		id := strings.TrimPrefix(result, "RESUME:")
		loaded, err := sessMgr.Load(id, sess.CWD)
		if err != nil {
			fmt.Fprintf(os.Stderr, "Error loading session %s: %v\n", id, err)
			return true
		}
		*sess = *loaded
		cmdCtx.CurrentSessionID = sess.ID
		if sess.Model != "" {
			eng.Loop.Model = sess.Model
			cmdCtx.Model = sess.Model
		}
		fmt.Fprintf(os.Stderr, "Resumed session %s (%d entries, model %s)\n",
			sess.ID, len(sess.Entries), sess.Model)

	case result == "RESUME_SELECTOR":
		sessions, err := sessMgr.List(sess.CWD)
		if err != nil {
			fmt.Fprintf(os.Stderr, "Error listing sessions: %v\n", err)
			return true
		}
		if len(sessions) == 0 {
			fmt.Fprintf(os.Stderr, "No sessions available to resume.\n")
			return true
		}
		fmt.Fprintf(os.Stderr, "Available sessions:\n")
		for i, s := range sessions {
			const max = 20
			if i >= max {
				fmt.Fprintf(os.Stderr, "  ... (%d more)\n", len(sessions)-max)
				break
			}
			fmt.Fprintf(os.Stderr, "  %s  [%s]  %d msgs\n", s.ID, s.Model, len(s.Entries))
		}
		fmt.Fprintf(os.Stderr, "Use /resume <id> to switch.\n")

	case strings.HasPrefix(result, "FORK:"):
		parts := strings.SplitN(strings.TrimPrefix(result, "FORK:"), ":", 2)
		sourceID := parts[0]
		entryID := "latest"
		if len(parts) > 1 {
			entryID = parts[1]
		}
		// Load parent and fork
		parent, err := sessMgr.Load(sourceID, sess.CWD)
		if err != nil {
			fmt.Fprintf(os.Stderr, "Fork failed: parent session %s not found\n", sourceID)
			return true
		}
		newSess := session.ForkSession(parent, entryID)
		if err := sessMgr.Save(newSess); err != nil {
			fmt.Fprintf(os.Stderr, "Fork failed to save: %v\n", err)
			return true
		}
		*sess = *newSess
		cmdCtx.CurrentSessionID = sess.ID
		fmt.Fprintf(os.Stderr, "Forked new session %s from %s @ %s\n", sess.ID, sourceID, entryID)

	case strings.HasPrefix(result, "CLONE:"):
		sourceID := strings.TrimPrefix(result, "CLONE:")
		parent, err := sessMgr.Load(sourceID, sess.CWD)
		if err != nil {
			fmt.Fprintf(os.Stderr, "Clone failed: parent session %s not found\n", sourceID)
			return true
		}
		newSess := session.ForkSession(parent, "")
		newSess.ID = session.GenerateID()
		newSess.CreatedAt = time.Now()
		if err := sessMgr.Save(newSess); err != nil {
			fmt.Fprintf(os.Stderr, "Clone failed to save: %v\n", err)
			return true
		}
		*sess = *newSess
		cmdCtx.CurrentSessionID = sess.ID
		fmt.Fprintf(os.Stderr, "Cloned session %s → %s\n", sourceID, sess.ID)

	case strings.HasPrefix(result, "COMPACT:"):
		if len(sess.Entries) == 0 {
			fmt.Fprintf(os.Stderr, "No messages to compact.\n")
			return true
		}
		messages := session.BuildContext(sess.Entries)
		tokensBefore := compaction.EstimateContextTokens(messages)
		reserveTokens := eng.Config.CompactionReserveTokens
		keepTokens := eng.Config.CompactionKeepRecentTokens
		if reserveTokens <= 0 {
			reserveTokens = 160000
		}
		if keepTokens <= 0 {
			keepTokens = reserveTokens / 2
		}
		compacted, _, err := compaction.Compact(messages, compaction.CompactOptions{
			ReserveTokens:    reserveTokens,
			KeepRecentTokens: keepTokens,
		})
		if err != nil {
			fmt.Fprintf(os.Stderr, "Compaction error: %v\n", err)
			return true
		}
		tokensAfter := compaction.EstimateContextTokens(compacted)
		fmt.Fprintf(os.Stderr, "Compaction: %d messages (est. %d tokens) → %d messages (est. %d tokens)\n",
			len(messages), tokensBefore, len(compacted), tokensAfter)
		summary := fmt.Sprintf("Compacted %d messages → %d messages (%d → %d tokens).",
			len(messages), len(compacted), tokensBefore, tokensAfter)
		entry := session.CompactionEntry(summary, "", "")
		sessMgr.AddEntry(sess, entry)

	case strings.HasPrefix(result, "IMPORT:"):
		destPath := strings.TrimPrefix(result, "IMPORT:")
		// The import creates a new session file; load it
		baseName := filepath.Base(destPath)
		sid := strings.TrimSuffix(strings.TrimPrefix(baseName, "imported_"), ".jsonl")
		imported, err := sessMgr.Load(sid, sess.CWD)
		if err != nil {
			fmt.Fprintf(os.Stderr, "Import saved to: %s (manual resume needed)\n", destPath)
			return true
		}
		*sess = *imported
		cmdCtx.CurrentSessionID = sess.ID
		fmt.Fprintf(os.Stderr, "Imported and resumed session %s (%d entries)\n", sess.ID, len(sess.Entries))

	case result == "RELOAD":
		newCfg, err := settings.LoadAll()
		if err != nil {
			fmt.Fprintf(os.Stderr, "Error reloading settings: %v\n", err)
			return true
		}
		fmt.Fprintf(os.Stderr, "Settings reloaded.\n")
		fmt.Fprintf(os.Stderr, "  Model:    %s\n", nonempty(newCfg.DefaultModel, "(none)"))
		fmt.Fprintf(os.Stderr, "  Provider: %s\n", nonempty(newCfg.DefaultProvider, "(none)"))
		fmt.Fprintf(os.Stderr, "  Theme:    %s\n", nonempty(newCfg.Theme, "(none)"))
		if cmdCtx.Model != newCfg.DefaultModel && newCfg.DefaultModel != "" {
			cmdCtx.Model = newCfg.DefaultModel
			eng.Loop.Model = newCfg.DefaultModel
			fmt.Fprintf(os.Stderr, "  → Model updated to: %s\n", newCfg.DefaultModel)
		}

	case result == "QUIT":
		fmt.Fprintf(os.Stderr, "Goodbye.\n")
		os.Exit(0)

	default:
		// Not a sentinel — just print the result
		fmt.Println(result)
	}
	return true
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

func saveSession(sessMgr *session.Manager, sess *session.Session, finalMessages []types.Message, model, baseURL string) {
	if len(finalMessages) == 0 {
		return
	}
	newEntries := session.MessagesToEntries(finalMessages, "")
	sess.Entries = append(sess.Entries, newEntries...)
	sess.Model = model
	sess.BaseURL = baseURL
	if err := sessMgr.Save(sess); err != nil {
		fmt.Fprintf(os.Stderr, "Warning: failed to save session: %v\n", err)
	}
}

func nonempty(s, fallback string) string {
	if s != "" {
		return s
	}
	return fallback
}

func newUserMsg(content string) types.Message {
	tc := types.TextContent{Type: "text", Text: content}
	b, _ := json.Marshal([]types.TextContent{tc})
	return types.Message{Role: "user", Content: b}
}
