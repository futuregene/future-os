package main

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"time"

	agentsession "github.com/huichen/xihu/internal/agentsession"
	"github.com/huichen/xihu/internal/auth"
	"github.com/huichen/xihu/internal/commands"
	"github.com/huichen/xihu/internal/compaction"
	"github.com/huichen/xihu/internal/engine"
	"github.com/huichen/xihu/internal/prompt"
	"github.com/huichen/xihu/internal/rpc"
	"github.com/huichen/xihu/internal/session"
	"github.com/huichen/xihu/internal/settings"
	"github.com/huichen/xihu/internal/skills"
	"github.com/huichen/xihu/internal/tui"
	"github.com/huichen/xihu/internal/utils"
	"github.com/huichen/xihu/pkg/types"
)

func main() {
	args := parseArgs(os.Args[1:])

	// Print diagnostics for invalid flags
	for _, d := range args.Diagnostics {
		fmt.Fprintf(os.Stderr, "⚠ %s\n", d)
	}

	if args.Help {
		printHelp()
	}
	if args.Version {
		printVersion()
	}

	cwd, err := os.Getwd()
	if err != nil {
		fmt.Fprintf(os.Stderr, "Error: %v\n", err)
		os.Exit(1)
	}

	// ── Settings ──────────────────────────────────────────────────────────
	cfg, cfgErr := settings.LoadAll()
	if cfg == nil {
		cfg = &settings.Settings{}
	}
	var settingsLoadErr string
	if cfgErr != nil {
		settingsLoadErr = cfgErr.Error()
	}

	// ── Auth ──────────────────────────────────────────────────────────────
	authStore, _ := auth.LoadAuth()

	// ── Session manager ───────────────────────────────────────────────────
	sessDir := args.SessionDir
	if sessDir == "" {
		sessDir = session.DefaultDir(cwd)
	}
	sessMgr := session.NewManager(sessDir)

	// ── List models ───────────────────────────────────────────────────────
	if args.ListModels != "" {
		listModels(args.ListModels)
		return
	}

	// ── Parse --model for provider/model:thinking format ──────────────────
	resolvedModel, resolvedProvider, resolvedThinking := parseModelString(
		args.Model,
		cfg.DefaultModel,
		cfg.DefaultProvider,
		cfg.DefaultThinkingLevel,
	)

	// ── Resolve provider (CLI flag > model prefix > settings default) ─────
	provider := firstNonEmpty(args.Provider, resolvedProvider, cfg.DefaultProvider)

	// ── Resolve base URL ─────────────────────────────────────────────────
	baseURL := firstNonEmpty(os.Getenv("LLM_BASE_URL"),
		providerBaseURL(provider, args.Provider))

	// ── Resolve API key (CLI > env vars > auth.json by provider) ──────────
	apiKey := firstNonEmpty(args.APIKey,
		os.Getenv("LLM_API_KEY"),
		os.Getenv("ANTHROPIC_API_KEY"),
		os.Getenv("OPENAI_API_KEY"))
	if apiKey == "" && authStore != nil {
		apiKey = authStore.Get(provider)
	}
	if apiKey == "" && authStore != nil {
		apiKey = authStore.DefaultKey()
	}

	// ── Resolve model ────────────────────────────────────────────────────
	model := firstNonEmpty(resolvedModel,
		os.Getenv("LLM_MODEL"),
		cfg.DefaultModel,
		defaultModelForURL(baseURL))

	// ── Resolve thinking (CLI > model suffix > settings) ─────────────────
	thinking := firstNonEmpty(args.Thinking, resolvedThinking)
	if thinking == "" && !args.NoThinkOverride() {
		thinking = cfg.DefaultThinkingLevel
	}

	// ── Startup banner ───────────────────────────────────────────────────
	if args.Verbose {
		bannerModel := model
		if provider != "" {
			bannerModel = provider + "/" + model
		}
		fmt.Fprintf(os.Stderr, "\033[33m[model]\033[0m %s", bannerModel)
		if thinking != "" && thinking != "off" {
			fmt.Fprintf(os.Stderr, "  thinking: %s", thinking)
		}
		fmt.Fprintln(os.Stderr)
	}

	// ── Session: --fork / --session / --resume / --continue / new ─────────
	var sess *session.Session

	if args.Fork != "" {
		parent, err := sessMgr.Load(args.Fork, cwd)
		if err != nil {
			fmt.Fprintf(os.Stderr, "Error loading fork source %s: %v\n", args.Fork, err)
			os.Exit(1)
		}
		sess = session.ForkSession(parent, "")
		sess.ID = session.GenerateID()
		sess.CreatedAt = time.Now()
		if err := sessMgr.Save(sess); err != nil {
			fmt.Fprintf(os.Stderr, "Error saving forked session: %v\n", err)
			os.Exit(1)
		}
		fmt.Fprintf(os.Stderr, "Forked session %s from %s\n", sess.ID, args.Fork)
	} else if args.Session != "" {
		var err error
		sess, err = sessMgr.Load(args.Session, cwd)
		if err != nil {
			fmt.Fprintf(os.Stderr, "Error loading session %s: %v\n", args.Session, err)
			os.Exit(1)
		}
		// Restore model/baseURL from session if not overridden via CLI
		if args.Model == "" && os.Getenv("LLM_MODEL") == "" && sess.Model != "" {
			model = sess.Model
		}
		if baseURL == "" && sess.BaseURL != "" {
			baseURL = sess.BaseURL
		}
	} else if args.Resume {
		sessions, err := sessMgr.List(cwd)
		if err != nil || len(sessions) == 0 {
			fmt.Fprintf(os.Stderr, "No sessions to resume.\n")
			os.Exit(1)
		}
		sess = &sessions[0]
		if args.Model == "" && os.Getenv("LLM_MODEL") == "" && sess.Model != "" {
			model = sess.Model
		}
		if baseURL == "" && sess.BaseURL != "" {
			baseURL = sess.BaseURL
		}
		fmt.Fprintf(os.Stderr, "Continuing session %s (%d messages, model %s)\n\n",
			sess.ID, len(sess.Entries), model)
	} else if args.Continue {
		sess = &session.Session{
			ID:        session.GenerateID(),
			CWD:       cwd,
			Model:     model,
			BaseURL:   baseURL,
			CreatedAt: time.Now(),
		}
	} else {
		sess = &session.Session{
			ID:        session.GenerateID(),
			CWD:       cwd,
			Model:     model,
			BaseURL:   baseURL,
			CreatedAt: time.Now(),
		}
	}

	// ── Export session ─────────────────────────────────────────────────────
	if args.Export != "" {
		exportSession(sess, args.Export)
		return
	}

	// ── Discover skills ────────────────────────────────────────────────────
	searchDirs := []string{skills.UserSkillsDir, skills.ProjectSkillsDir, skills.AgentsSkillsDir, skills.PiSkillsDir}
	allSkills, _ := skills.DiscoverSkills(searchDirs, "user")
	resolvedSkills, skillCollisions := skills.ResolveCollisionsWithDiagnostics(allSkills)

	var promptSkills []prompt.Skill
	for _, s := range resolvedSkills {
		promptSkills = append(promptSkills, prompt.Skill{
			Name:        s.Name,
			Description: s.Description,
			Location:    s.Path,
		})
	}

	// ── Discover context files (AGENTS.md, CLAUDE.md) ────────────────────
	agentDir := filepath.Join(os.Getenv("HOME"), ".xihu")
	var cfPaths []string
	agentsContent := ""
	if !args.NoContextFiles {
		contextFiles := prompt.DiscoverContextFiles(agentDir, cwd)
		cfPaths = make([]string, len(contextFiles))
		for i, cf := range contextFiles {
			cfPaths[i] = cf.Path
			if agentsContent != "" {
				agentsContent += "\n\n"
			}
			agentsContent += cf.Content
		}
	}

	// ── Build system prompt ────────────────────────────────────────────────
	customPrompt := firstNonEmpty(args.SystemPrompt, cfg.SystemPrompt)
	appendText := strings.Join(args.AppendSystemPrompt, "\n")
	builtPrompt := prompt.BuildPrompt(prompt.PromptOptions{
		CustomPrompt:     customPrompt,
		WorkingDirectory: cwd,
		Date:             time.Now().Format("2006-01-02"),
		Tools:            engine.CodingTools(),
		Skills:           promptSkills,
		AGENTSContent:    agentsContent,
		AppendPrompt:     appendText,
	})

	// ── Build engine ───────────────────────────────────────────────────────
	if baseURL == "" {
		baseURL = "https://api.openai.com"
	}

	eng, err := engine.NewEngine(engine.EngineOptions{
		BaseURL:        baseURL,
		APIKey:         apiKey,
		Model:          model,
		CWD:            cwd,
		Settings:       cfg,
		SessionManager: sessMgr,
		MaxTurns:       cfg.MaxTurns,
		ThinkingLevel:  thinking,
		SystemPrompt:   builtPrompt,
		NoTools:        args.NoTools,
		Verbose:        args.Verbose,
		ExtensionPaths: args.Extensions,
		NoExtensions:   args.NoExtensions,
	})
	if err != nil {
		fmt.Fprintf(os.Stderr, "Engine error: %v\n", err)
		os.Exit(1)
	}
	eng.Session = sess
	eng.Loop.Model = model

	// Apply CLI overrides to engine
	if args.NoBuiltinTools {
		eng.Loop.Tools = nil
	}

	// ── Create AgentSession ─────────────────────────────────────────────
	as, err := agentsession.New(agentsession.AgentSessionConfig{
		Engine:       eng,
		CWD:          cwd,
		MaxRetries:   3,
	})
	if err != nil {
		fmt.Fprintf(os.Stderr, "AgentSession error: %v\n", err)
		os.Exit(1)
	}

	// ── Build commands context ─────────────────────────────────────────────
	settingsGlobal, _ := settings.GetDefaultPaths()
	cmdCtx := &commands.Context{
		CWD:              cwd,
		SessionDir:       sessDir,
		SettingsDir:      filepath.Join(os.Getenv("HOME"), ".xihu"),
		CurrentSessionID: sess.ID,
		SettingsPath:     settingsGlobal,
		Model:            model,
		BaseURL:          baseURL,
		SystemPrompt:     eng.Loop.SystemPrompt,
	}

	// ── Build user prompt (messages + @files) ──────────────────────────────
	var promptParts []string
	for _, f := range args.FileArgs {
		if mime := utils.DetectImageMimeTypeFromExtension(f); mime != "" {
			if confirmed, _ := utils.DetectImageMimeType(f); confirmed != "" || mime == "image/svg+xml" {
				data, err := os.ReadFile(f)
				if err == nil {
					imageTag := fmt.Sprintf("<file name=\"%s\" type=\"%s\">[Image: %s]</file>",
						f, mime, filepath.Base(f))
					promptParts = append(promptParts, imageTag)
					_ = data
					continue
				}
			}
		}
		data, err := os.ReadFile(f)
		if err == nil {
			promptParts = append(promptParts, fmt.Sprintf("<file name=\"%s\">\n%s\n</file>", f, string(data)))
		}
	}
	promptParts = append(promptParts, args.Messages...)
	userPrompt := strings.Join(promptParts, "\n")

	// ── RPC Mode ───────────────────────────────────────────────────────
	if args.Mode == "rpc" {
		srv := rpc.NewServer(as)
		err := srv.Run()
		if err != nil {
			fmt.Fprintf(os.Stderr, "RPC error: %v\n", err)
			os.Exit(1)
		}
		return
	}

	// ── Interactive REPL (no prompt + TTY) ─────────────────────────────────
	if userPrompt == "" && isTerminal() {
		availableModels := []string(nil)
		if eng.Settings != nil {
			availableModels = eng.Settings.EnabledModels
		}
		// Parse prompt templates from standard directories
		var promptTemplates []prompt.PromptTemplate
		templateDirs := []string{
			filepath.Join(agentDir, "prompts"),
			filepath.Join(cwd, ".xihu", "prompts"),
		}
		for _, dir := range templateDirs {
			templates, err := prompt.ParseTemplates(dir)
			if err == nil {
				promptTemplates = append(promptTemplates, templates...)
			}
		}
		err := tui.Run(as, sess, "", model, baseURL, resolvedSkills, nil, thinking, availableModels, cfg, eng.ExtensionRunner, promptTemplates, cfPaths, skillCollisions, settingsLoadErr)
		if err != nil {
			fmt.Fprintf(os.Stderr, "TUI error: %v\n", err)
			os.Exit(1)
		}
		return
	}

	if userPrompt == "" {
		fmt.Fprintf(os.Stderr, "Usage: xihu [options] [@files...] [messages...]\n")
		fmt.Fprintf(os.Stderr, "Try 'xihu --help' for more information.\n")
		os.Exit(1)
	}

	// ── Slash command ──────────────────────────────────────────────────────
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

	// ── Run agent ─────────────────────────────────────────────────────────
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

	if !args.NoSession {
		saveSession(sessMgr, sess, finalMessages, model, baseURL)
	}
}

// ─── Helpers ───────────────────────────────────────────────────────────────

func isTerminal() bool {
	fi, _ := os.Stdin.Stat()
	return (fi.Mode() & os.ModeCharDevice) != 0
}

func firstNonEmpty(ss ...string) string {
	for _, s := range ss {
		if s != "" {
			return s
		}
	}
	return ""
}

func providerBaseURL(provider, fallbackProvider string) string {
	p := firstNonEmpty(provider, fallbackProvider)
	switch p {
	case "anthropic":
		return "https://api.anthropic.com"
	case "openai":
		return "https://api.openai.com"
	case "deepseek":
		return "https://api.deepseek.com"
	case "dashscope", "dashscope-coding":
		return "https://dashscope.aliyuncs.com/compatible-mode/v1"
	case "google", "gemini":
		return "https://generativelanguage.googleapis.com/v1beta/openai"
	case "groq":
		return "https://api.groq.com/openai/v1"
	case "xai":
		return "https://api.x.ai/v1"
	case "openrouter":
		return "https://openrouter.ai/api/v1"
	case "together":
		return "https://api.together.xyz/v1"
	case "cerebras":
		return "https://api.cerebras.ai/v1"
	case "mistral":
		return "https://api.mistral.ai/v1"
	case "perplexity":
		return "https://api.perplexity.ai"
	case "fireworks":
		return "https://api.fireworks.ai/inference/v1"
	case "github-copilot":
		return "https://api.githubcopilot.com"
	case "azure-openai":
		return "" // requires full endpoint; user must set LLM_BASE_URL
	case "azure-openai-responses":
		return "" // requires full endpoint; user must set LLM_BASE_URL
	default:
		return ""
	}
}

func defaultModelForURL(baseURL string) string {
	if strings.Contains(baseURL, "anthropic.com") {
		return "claude-sonnet-4-20250514"
	}
	return "gpt-4o"
}

// parseModelString parses a --model flag into model, provider, and thinking parts.
func parseModelString(raw, defaultModel, defaultProvider, defaultThinking string) (model, provider, thinking string) {
	if raw == "" {
		return defaultModel, defaultProvider, defaultThinking
	}

	// Check for :thinking suffix
	if idx := strings.LastIndex(raw, ":"); idx > 0 {
		candidate := raw[idx+1:]
		if validThinkingLevels[candidate] {
			thinking = candidate
			raw = raw[:idx]
		}
	}

	// Check for provider/model prefix
	if idx := strings.Index(raw, "/"); idx > 0 {
		provider = raw[:idx]
		model = raw[idx+1:]
	} else {
		model = raw
	}

	return model, provider, thinking
}

func listModels(search string) {
	cfg, _ := settings.LoadAll()
	_ = search

	if cfg != nil && len(cfg.EnabledModels) > 0 {
		fmt.Fprintf(os.Stderr, "%-24s %s\n", "provider", "model")
		fmt.Fprintf(os.Stderr, "%s\n", strings.Repeat("-", 60))
		for _, m := range cfg.EnabledModels {
			parts := strings.SplitN(m, "/", 2)
			if len(parts) == 2 {
				fmt.Fprintf(os.Stderr, "%-24s %s\n", parts[0], parts[1])
			} else {
				fmt.Fprintf(os.Stderr, "%-24s %s\n", "unknown", m)
			}
		}
		if search != "" && search != "true" {
			fmt.Fprintf(os.Stderr, "\nFiltered by: %s\n", search)
		}
		return
	}

	fmt.Println("Available models:")
	fmt.Println("  gpt-4o          (OpenAI)")
	fmt.Println("  gpt-4o-mini     (OpenAI)")
	fmt.Println("  claude-sonnet   (Anthropic)")
	fmt.Println("  claude-haiku    (Anthropic)")
}

func exportSession(sess *session.Session, path string) {
	html := exportSessionHTML(sess)
	if err := os.WriteFile(path, []byte(html), 0644); err != nil {
		fmt.Fprintf(os.Stderr, "Export error: %v\n", err)
		os.Exit(1)
	}
	fmt.Fprintf(os.Stderr, "Session exported to: %s\n", path)
}

func exportSessionHTML(sess *session.Session) string {
	var sb strings.Builder
	sb.WriteString("<!DOCTYPE html>\n<html><head><meta charset=\"utf-8\">")
	sb.WriteString(fmt.Sprintf("<title>xihu session %s</title>", sess.ID))
	sb.WriteString("<style>body{font-family:system-ui;max-width:800px;margin:auto;padding:20px;background:#1a1a2e;color:#e0e0e0}")
	sb.WriteString(".entry{padding:10px;margin:5px 0;border-radius:8px}")
	sb.WriteString(".user{background:#16213e}.assistant{background:#0f3460}.tool{background:#1a1a2e}")
	sb.WriteString("</style></head><body>\n")
	sb.WriteString(fmt.Sprintf("<h1>xihu Session: %s</h1>\n", sess.ID))
	sb.WriteString(fmt.Sprintf("<p>Model: %s | CWD: %s</p>\n", sess.Model, sess.CWD))
	for _, e := range sess.Entries {
		cls := "entry "
		switch e.Type {
		case "assistant":
			cls += "assistant"
		default:
			cls += "user"
		}
		sb.WriteString(fmt.Sprintf("<div class=\"%s\"><strong>%s</strong><pre>%s</pre></div>\n",
			cls, e.Type, escapeHTML(string(e.Content))))
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

func newUserMsg(content string) types.Message {
	tc := types.TextContent{Type: "text", Text: content}
	b, _ := json.Marshal([]types.TextContent{tc})
	return types.Message{Role: "user", Content: b}
}

// processSentinel — keep existing sentinel handling
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
		fmt.Fprintf(os.Stderr, "Compaction: %d messages (%d tokens) → %d messages (%d tokens)\n",
			len(messages), tokensBefore, len(compacted), tokensAfter)
		summary := fmt.Sprintf("Compacted: %d→%d msgs (%d→%d tokens)",
			len(messages), len(compacted), tokensBefore, tokensAfter)
		sessMgr.AddEntry(sess, session.CompactionEntry(summary, "", ""))

	case strings.HasPrefix(result, "IMPORT:"):
		destPath := strings.TrimPrefix(result, "IMPORT:")
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
		if cmdCtx.Model != newCfg.DefaultModel && newCfg.DefaultModel != "" {
			cmdCtx.Model = newCfg.DefaultModel
			eng.Loop.Model = newCfg.DefaultModel
		}

	case result == "QUIT":
		fmt.Fprintf(os.Stderr, "Goodbye.\n")
		os.Exit(0)

	default:
		fmt.Println(result)
	}
	return true
}
