// Package engine provides the unified Engine — a high-level session manager
// that wires together provider detection, settings, sessions, tool config,
// and the agent loop into a single ready-to-use struct.
package engine

import (
	"fmt"
	"os"
	"strings"
	"time"

	"github.com/huichen/xihu/internal/agent"
	"github.com/huichen/xihu/internal/apiregistry"
	"github.com/huichen/xihu/internal/compaction"
	"github.com/huichen/xihu/internal/extensions"
	"github.com/huichen/xihu/internal/llm"
	"github.com/huichen/xihu/internal/modelregistry"
	"github.com/huichen/xihu/internal/session"
	"github.com/huichen/xihu/internal/settings"
	"github.com/huichen/xihu/pkg/types"
)

// ---------------------------------------------------------------------------
// AgentConfig — richer configuration layered on top of agent.Loop settings
// ---------------------------------------------------------------------------

// AgentConfig holds all configurable agent options.
type AgentConfig struct {
	// CWD is the current working directory for the session (default ".").
	CWD string

	// SystemPrompt is the system prompt sent to the LLM.
	SystemPrompt string

	// MaxTurns is the maximum number of agent loop turns before stopping (default 50).
	MaxTurns int

	// ThinkingLevel controls the thinking/reasoning budget.
	// Valid values: "off", "low", "medium", "high", "xhigh", "max".
	// Empty means "use provider default".
	ThinkingLevel string

	// ScopedModels restricts the agent to only these model IDs.
	// Empty means no restriction.
	ScopedModels []string

	// NoTools controls tool visibility:
	//   "all"     — all tools enabled
	//   "builtin" — built-in tools only (no external extensions)
	//   ""        — use the configured tool list (default)
	NoTools string

	// CompactionReserveTokens is the token budget threshold for auto-compaction.
	// 0 means use default (160000).
	CompactionReserveTokens int

	// CompactionKeepRecentTokens is the token budget for recent messages kept
	// uncompacted during auto-compaction. 0 means use default (half of ReserveTokens).
	CompactionKeepRecentTokens int
}

// Default returns sensible defaults for AgentConfig.
func (c AgentConfig) Default() AgentConfig {
	if c.CWD == "" {
		c.CWD = "."
	}
	if c.MaxTurns <= 0 {
		c.MaxTurns = agent.DefaultMaxTurns
	}
	if c.CompactionReserveTokens <= 0 {
		c.CompactionReserveTokens = 160000
	}
	if c.CompactionKeepRecentTokens <= 0 {
		c.CompactionKeepRecentTokens = c.CompactionReserveTokens / 2
	}
	return c
}

// ---------------------------------------------------------------------------
// EngineOptions — arguments to NewEngine
// ---------------------------------------------------------------------------

// EngineOptions holds all options for creating a new Engine.
type EngineOptions struct {
	// BaseURL is the LLM API base URL (required).
	BaseURL string

	// APIKey is the LLM API key (required).
	APIKey string

	// Model is the model name to use (required).
	Model string

	// CWD is the working directory (default ".").
	CWD string

	// SystemPrompt overrides the default system prompt.
	SystemPrompt string

	// MaxTurns overrides the default max turns (0 = use default, 50).
	MaxTurns int

	// ThinkingLevel sets the thinking/reasoning level.
	ThinkingLevel string

	// Tools is an explicit tool list. If nil and NoTools is false,
	// all built-in tools (CodingTools) are used.
	Tools []types.AgentTool

	// Settings is a pre-loaded settings struct. If nil, settings are loaded
	// automatically via settings.LoadAll().
	Settings *settings.Settings

	// SessionManager is the session persistence manager. If nil, a default
	// manager is created using session.DefaultDir(CWD).
	SessionManager *session.Manager

	// ExtensionPaths is a list of extension paths (dirs, .json, .so files) to load.
	ExtensionPaths []string

	// NoExtensions disables extension loading entirely.
	NoExtensions bool

	// NoTools controls tool visibility:
	//   ""        — tools enabled, extensions merged
	//   "all"     — all tools disabled
	//   "builtin" — built-in tools only (no extension tools)
	NoTools string

	// Verbose enables verbose tool-call logging to stderr.
	Verbose bool
}

// applyDefaults fills in zero values with sensible defaults.
func (o *EngineOptions) applyDefaults() {
	if o.CWD == "" {
		o.CWD = "."
	}
	if o.MaxTurns <= 0 {
		o.MaxTurns = agent.DefaultMaxTurns
	}
}

// ---------------------------------------------------------------------------
// Engine — the unified agent session
// ---------------------------------------------------------------------------

// Engine is the high-level agent session. It bundles provider, configuration,
// session history, settings, tools, and the agent loop into a single struct.
type Engine struct {
	Provider       types.LLMProvider
	Model          string
	ModelInfo      types.Model        // full model metadata from registry
	Config         AgentConfig
	Tools          []types.AgentTool
	Session        *session.Session
	SessionManager *session.Manager
	Settings       *settings.Settings
	Loop           *agent.Loop
	ModelRegistry  *modelregistry.Registry
	// ExtensionRunner manages loaded extensions (nil if no extensions loaded).
	ExtensionRunner *extensions.ExtensionRunner
}

// NewEngine creates a new Engine with the given options. It:
//  1. Resolves the model via ModelRegistry.
//  2. Loads and merges settings (global + project).
//  3. Creates or resumes a session.
//  4. Sets up the agent loop with all configuration.
//
// Returns an Engine ready to use.
func NewEngine(opts EngineOptions) (*Engine, error) {
	opts.applyDefaults()

	// 0. Initialize model registry with embedded catalog
	mreg := modelregistry.New()

	// 0.5. Load/merge settings BEFORE model resolution so defaults apply
	s := opts.Settings
	if s == nil {
		var err error
		s, err = settings.LoadAll()
		if err != nil {
			return nil, fmt.Errorf("load settings: %w", err)
		}
	}

	// Apply settings overrides to opts where not explicitly set
	if opts.SystemPrompt == "" && s.SystemPrompt != "" {
		opts.SystemPrompt = s.SystemPrompt
	}
	if opts.MaxTurns <= 0 && s.MaxTurns > 0 {
		opts.MaxTurns = s.MaxTurns
	}
	if opts.ThinkingLevel == "" && s.DefaultThinkingLevel != "" {
		opts.ThinkingLevel = s.DefaultThinkingLevel
	}
	if opts.Model == "" && s.DefaultModel != "" {
		// If default_provider is set, use canonical "provider/model" format
		if s.DefaultProvider != "" && !strings.Contains(s.DefaultModel, "/") {
			opts.Model = s.DefaultProvider + "/" + s.DefaultModel
		} else {
			opts.Model = s.DefaultModel
		}
	}

	// 1. Resolve model — try explicit provider/model, then fall back to base URL detection
	var modelInfo types.Model
	var resolved bool

	if opts.Model != "" {
		modelInfo, resolved = mreg.Resolve(opts.Model)
	}

	if !resolved && opts.BaseURL != "" {
		// Fall back: try to find a model matching the base URL's provider
		api := apiregistry.LookupAPI(opts.BaseURL)
		provider := providerFromURL(opts.BaseURL)
		// Prefer model from the same provider, then any matching API
		for _, m := range mreg.GetAll() {
			if m.API == string(api) && m.Provider == provider {
				modelInfo = m
				resolved = true
				break
			}
		}
		if !resolved {
			for _, m := range mreg.GetAll() {
				if m.API == string(api) {
					modelInfo = m
					resolved = true
					break
				}
			}
		}
	}

	if !resolved {
		// Last resort: construct a minimal model from base URL
		modelInfo = types.Model{
			ID:       opts.Model,
			Provider: providerFromURL(opts.BaseURL),
			API:      string(apiregistry.LookupAPI(opts.BaseURL)),
			BaseURL:  opts.BaseURL,
		}
		if modelInfo.ID == "" {
			modelInfo.ID = modelregistry.DefaultModel(modelInfo.Provider)
		}
	}

	// Apply provider overrides to base URL if registered
	if ov, ok := mreg.GetProviderOverride(modelInfo.Provider); ok && ov.BaseURL != "" {
		modelInfo.BaseURL = ov.BaseURL
	}

	// 2. Create LLM provider via API registry
	api := apiregistry.API(modelInfo.API)
	factory, err := apiregistry.Get(api)
	if err != nil {
		return nil, fmt.Errorf("resolve provider for API %q (model %s): %w", api, modelInfo.ID, err)
	}
	provider := factory(modelInfo.BaseURL, opts.APIKey, nil)

	// Resolve thinking budget for OpenAI-compatible clients
	thinkingBudget := thinkingLevelToBudget(opts.ThinkingLevel)
	if cl, ok := provider.(*llm.Client); ok {
		cl.ThinkingBudget = thinkingBudget
	} else if s, ok := provider.(apiregistry.ThinkingBudgetSetter); ok {
		s.SetThinkingBudget(thinkingBudget)
	}

	// 3. Create LLM provider via API registry
	cfg := AgentConfig{
		CWD:                        opts.CWD,
		SystemPrompt:               opts.SystemPrompt,
		MaxTurns:                   opts.MaxTurns,
		ThinkingLevel:              opts.ThinkingLevel,
		CompactionReserveTokens:    s.CompactionReserveTokens,
		CompactionKeepRecentTokens: s.CompactionKeepRecentTokens,
		NoTools:                    opts.NoTools,
	}
	cfg = cfg.Default()

	// 5. Resolve tools
	toolList := opts.Tools
	noTools := opts.NoTools == "all"
	if noTools {
		toolList = nil
	} else if len(toolList) == 0 {
		toolList = CodingTools()
	}

	// 6. Set up session manager and session
	sessMgr := opts.SessionManager
	if sessMgr == nil {
		sessMgr = session.NewManager(session.DefaultDir(opts.CWD))
	}

	sess := &session.Session{
		ID:        session.GenerateID(),
		CWD:       opts.CWD,
		Model:     opts.Model,
		BaseURL:   modelInfo.BaseURL,
		CreatedAt: time.Now(),
	}

	// 7. Build the agent loop
	loop := &agent.Loop{
		Provider:      provider,
		Model:         opts.Model,
		SystemPrompt:  cfg.SystemPrompt,
		Tools:         toolList,
		Config: types.AgentConfig{
			SystemPrompt:   cfg.SystemPrompt,
			MaxTurns:       cfg.MaxTurns,
			ThinkingBudget: thinkingLevelToBudget(cfg.ThinkingLevel),
		},
		SteeringQueue: agent.NewPendingMessageQueue(64, "all"),
		FollowUpQueue: agent.NewPendingMessageQueue(64, "all"),
		Verbose:       opts.Verbose,
	}

	// Wire up auto-compaction if configured
	if cfg.CompactionReserveTokens > 0 {
		contextWindow := modelInfo.ContextWindow
		loop.Config.TransformContext = func(messages []types.Message, _ string) []types.Message {
			compacted, result, _ := compaction.Compact(messages, compaction.CompactOptions{
				ReserveTokens:    cfg.CompactionReserveTokens,
				KeepRecentTokens: cfg.CompactionKeepRecentTokens,
				ContextWindow:    contextWindow,
			})
			loop.LastCompactionResult = result
			return compacted
		}
	}

	// 8. Load extensions (with provider registration wiring)
	var extRunner *extensions.ExtensionRunner
	if !opts.NoExtensions {
		extPaths := opts.ExtensionPaths
		if len(extPaths) == 0 {
			extPaths = extensions.DiscoverExtensionPaths(opts.CWD)
		}
		if len(extPaths) > 0 {
			extCtx := extensions.NewExtensionContext(sessMgr, s, extensions.NewEventBus(), nil, opts.CWD, nil)
			// Wire provider registry into extension context
			extCtx.ModelRegistry = mreg
			runner, err := extensions.Run(extPaths, extCtx)
			if err != nil {
				if opts.Verbose {
					fmt.Fprintf(os.Stderr, "extension load warning: %v\n", err)
				}
			}
			if runner != nil {
				extRunner = runner

				// Merge extension tools into agent loop (unless "builtin" mode)
				if opts.NoTools != "builtin" {
					extTools := extensions.GetAllTools()
					toolList = append(toolList, extTools...)
					loop.Tools = toolList
				}

				// Inject extension prompts into system prompt
				extPrompts := extensions.GetAllPrompts()
				if len(extPrompts) > 0 {
					promptLines := make([]string, 0, len(extPrompts))
					for name, tmpl := range extPrompts {
						promptLines = append(promptLines, fmt.Sprintf("## Extension: %s\n%s", name, tmpl))
					}
					injectedPrompts := "\n\n<!-- BEGIN EXTENSION PROMPTS -->\n" + strings.Join(promptLines, "\n\n") + "\n<!-- END EXTENSION PROMPTS -->"
					loop.SystemPrompt += injectedPrompts
					loop.Config.SystemPrompt += injectedPrompts
				}
			}
		}
	}

	return &Engine{
		Provider:        provider,
		Model:           opts.Model,
		ModelInfo:       modelInfo,
		Config:          cfg,
		Tools:           toolList,
		Session:         sess,
		SessionManager:  sessMgr,
		Settings:        s,
		Loop:            loop,
		ModelRegistry:   mreg,
		ExtensionRunner: extRunner,
	}, nil
}

// ---------------------------------------------------------------------------
// Convenience methods
// ---------------------------------------------------------------------------

// Getwd returns the engine's working directory.
func (e *Engine) Getwd() string {
	return e.Config.CWD
}

// Chdir changes the engine's working directory. Validates the directory exists
// and is a directory. Updates both the config CWD and session CWD.
func (e *Engine) Chdir(dir string) error {
	info, err := os.Stat(dir)
	if err != nil {
		return fmt.Errorf("chdir %s: %w", dir, err)
	}
	if !info.IsDir() {
		return fmt.Errorf("chdir %s: not a directory", dir)
	}
	e.Config.CWD = dir
	e.Session.CWD = dir
	return nil
}
