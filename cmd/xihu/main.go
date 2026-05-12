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

	// ── Config subcommand ──────────────────────────────────────────────
	if args.Config {
		runConfigCommand(cwd)
		return
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
	searchDirs := []string{skills.UserSkillsDir, skills.ProjectSkillsDir, skills.AgentsSkillsDir}
	allSkills, _ := skills.DiscoverSkills(searchDirs, "user")
	resolvedSkills, skillCollisions := skills.ResolveCollisionsWithDiagnostics(allSkills)

	var promptSkills []prompt.Skill
	for _, s := range resolvedSkills {
		promptSkills = append(promptSkills, prompt.Skill{
			Name:                   s.Name,
			Description:            s.Description,
			Location:               s.Path,
			DisableModelInvocation: s.DisableModelInvocation,
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

	// ── Build engine ───────────────────────────────────────────────────────
	if baseURL == "" {
		baseURL = "https://api.openai.com"
	}

	noToolsMode := resolveNoTools(args)
	eng, err := engine.NewEngine(engine.EngineOptions{
		BaseURL:        baseURL,
		APIKey:         apiKey,
		Model:          model,
		CWD:            cwd,
		Settings:       cfg,
		SessionManager: sessMgr,
		MaxTurns:       cfg.MaxTurns,
		ThinkingLevel:  thinking,
		SystemPrompt:   "", // will be set below after resolving active tools
		NoTools:        noToolsMode,
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

	// Apply --tools allowlist filter
	if len(args.Tools) > 0 {
		allowSet := make(map[string]bool, len(args.Tools))
		for _, name := range args.Tools {
			allowSet[name] = true
		}
		filtered := make([]types.AgentTool, 0, len(args.Tools))
		for _, t := range eng.Loop.Tools {
			if allowSet[t.Def.Function.Name] {
				filtered = append(filtered, t)
			}
		}
		eng.Loop.Tools = filtered
	}

	// ── Build system prompt (after tools are resolved) ─────────────────────
	// Respect --no-tools / --no-builtin-tools / --no-skills / --tools in the prompt
	customPrompt := firstNonEmpty(args.SystemPrompt, cfg.SystemPrompt)
	appendText := strings.Join(args.AppendSystemPrompt, "\n")

	// Determine which tools to show in the prompt
	var promptTools []types.AgentTool
	if noToolsMode == "all" {
		promptTools = nil // no tools shown
	} else {
		promptTools = eng.Loop.Tools // actual active tools (after filtering)
	}

	// Determine which skills to show in the prompt
	var promptSkillsFiltered []prompt.Skill
	if !args.NoSkills {
		promptSkillsFiltered = promptSkills
	}

	builtPrompt := prompt.BuildPrompt(prompt.PromptOptions{
		CustomPrompt:     customPrompt,
		WorkingDirectory: cwd,
		Date:             time.Now().Format("2006-01-02"),
		Tools:            promptTools,
		Skills:           promptSkillsFiltered,
		AGENTSContent:    agentsContent,
		AppendPrompt:     appendText,
	})
	eng.Loop.SystemPrompt = builtPrompt
	eng.Loop.Config.SystemPrompt = builtPrompt

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
		// Use model registry for available models, filtered by settings.EnabledModels if set
		if eng.ModelRegistry != nil {
			all := eng.ModelRegistry.GetAll()
			// If settings has enabledModels, filter using pattern matching (pi: resolveModelScope)
			if eng.Settings != nil && len(eng.Settings.EnabledModels) > 0 {
				for _, m := range all {
					fullID := m.Provider + "/" + m.ID
					for _, pat := range eng.Settings.EnabledModels {
						if matchModelPattern(pat, fullID, m.ID) {
							availableModels = append(availableModels, fullID)
							break
						}
					}
				}
			} else {
				for _, m := range all {
					availableModels = append(availableModels, m.Provider+"/"+m.ID)
				}
			}
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
		// Print model scope line before TUI takeover (TS pi-mono: console.log before InteractiveMode.run)
	if len(availableModels) > 0 && (cfg.QuietStartup == nil || !*cfg.QuietStartup) {
		var scopeList []string
		for _, m := range availableModels {
			scopeList = append(scopeList, m)
		}
	}

	// Preserve provider prefix in model string so NewAppModel can parse it
	modelWithProvider := model
	if provider != "" && !strings.Contains(model, "/") {
		modelWithProvider = provider + "/" + model
	}
	err := tui.Run(as, sess, "", modelWithProvider, baseURL, resolvedSkills, nil, thinking, availableModels, cfg, eng.ExtensionRunner, promptTemplates, cfPaths, skillCollisions, settingsLoadErr)
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

	result, finalMessages, err := eng.Loop.RunStreamingWithMessages(ctx, types.ConvertFromLLM(allMessages), func(text string) {
		fmt.Print(text)
	})
	if err != nil {
		fmt.Fprintf(os.Stderr, "\nError: %v\n", err)
		os.Exit(1)
	}
	_ = result
	fmt.Println()

	if !args.NoSession {
		saveSession(sessMgr, sess, types.ConvertToLLM(finalMessages), model, baseURL)
	}
}

func newUserMsg(content string) types.Message {
	tc := types.TextContent{Type: "text", Text: content}
	b, _ := json.Marshal([]types.TextContent{tc})
	return types.Message{Role: "user", Content: b}
}

