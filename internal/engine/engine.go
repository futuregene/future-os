// Package engine provides the unified Engine — a high-level session manager
// that wires together provider detection, settings, sessions, tool config,
// and the agent loop into a single ready-to-use struct.
package engine

import (
	"fmt"
	"os"
	"strings"
	"time"

	"github.com/huichen/cobalt/internal/agent"
	"github.com/huichen/cobalt/internal/compaction"
	"github.com/huichen/cobalt/internal/llm"
	"github.com/huichen/cobalt/internal/session"
	"github.com/huichen/cobalt/internal/settings"
	"github.com/huichen/cobalt/internal/tools"
	"github.com/huichen/cobalt/pkg/types"
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

	// NoTools disables all tools if true.
	NoTools bool
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
	Config         AgentConfig
	Tools          []types.AgentTool
	Session        *session.Session
	SessionManager *session.Manager
	Settings       *settings.Settings
	Loop           *agent.Loop
}

// NewEngine creates a new Engine with the given options. It:
//  1. Auto-detects the provider (Anthropic vs OpenAI) from BaseURL.
//  2. Loads and merges settings (global + project).
//  3. Creates or resumes a session.
//  4. Sets up the agent loop with all configuration.
//
// Returns an Engine ready to use.
func NewEngine(opts EngineOptions) (*Engine, error) {
	opts.applyDefaults()

	// 1. Auto-detect provider from BaseURL
	provider := detectProvider(opts.BaseURL, opts.APIKey)

	// 2. Load/merge settings
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
		opts.Model = s.DefaultModel
	}

	// 3. Build AgentConfig
	cfg := AgentConfig{
		CWD:                       opts.CWD,
		SystemPrompt:              opts.SystemPrompt,
		MaxTurns:                  opts.MaxTurns,
		ThinkingLevel:             opts.ThinkingLevel,
		CompactionReserveTokens:   s.CompactionReserveTokens,
		CompactionKeepRecentTokens: s.CompactionKeepRecentTokens,
	}
	if opts.NoTools {
		cfg.NoTools = "all"
	}
	cfg = cfg.Default()

	// 4. Resolve tools
	toolList := opts.Tools
	if opts.NoTools {
		toolList = nil
	} else if len(toolList) == 0 {
		toolList = CodingTools()
	}

	// 5. Set up session manager and session
	sessMgr := opts.SessionManager
	if sessMgr == nil {
		sessMgr = session.NewManager(session.DefaultDir(opts.CWD))
	}

	sess := &session.Session{
		ID:        session.GenerateID(),
		CWD:       opts.CWD,
		Model:     opts.Model,
		BaseURL:   opts.BaseURL,
		CreatedAt: time.Now(),
	}

	// 6. Build agent loop config
	loopConfig := types.AgentConfig{
		SystemPrompt:   cfg.SystemPrompt,
		MaxTurns:       cfg.MaxTurns,
		ThinkingBudget: thinkingLevelToBudget(cfg.ThinkingLevel),
	}

	// Wire up auto-compaction if configured
	if cfg.CompactionReserveTokens > 0 {
		loopConfig.TransformContext = func(messages []types.Message, _ string) []types.Message {
			compacted, _, _ := compaction.Compact(messages, compaction.CompactOptions{
				ReserveTokens:    cfg.CompactionReserveTokens,
				KeepRecentTokens: cfg.CompactionKeepRecentTokens,
			})
			return compacted
		}
	}

	// 7. Build the agent loop
	loop := &agent.Loop{
		Provider:      provider,
		Model:         opts.Model,
		SystemPrompt:  cfg.SystemPrompt,
		Tools:         toolList,
		Config:        loopConfig,
		SteeringQueue: make(chan string, 64),
	}

	return &Engine{
		Provider:       provider,
		Model:          opts.Model,
		Config:         cfg,
		Tools:          toolList,
		Session:        sess,
		SessionManager: sessMgr,
		Settings:       s,
		Loop:           loop,
	}, nil
}

// ---------------------------------------------------------------------------
// Provider auto-detection
// ---------------------------------------------------------------------------

// detectProvider returns the appropriate LLMProvider for the given base URL.
// Anthropic if the URL contains "anthropic.com", otherwise OpenAI-compatible.
func detectProvider(baseURL, apiKey string) types.LLMProvider {
	if strings.Contains(baseURL, "anthropic.com") {
		return llm.NewAnthropicClient(baseURL, apiKey)
	}
	return llm.NewClient(baseURL, apiKey)
}

// ---------------------------------------------------------------------------
// Thinking level → budget mapping
// ---------------------------------------------------------------------------

// thinkingLevelToBudget converts a named thinking level to a token budget.
// Returns 0 for "off" or empty string (meaning "not set / use default").
func thinkingLevelToBudget(level string) int {
	switch level {
	case "off":
		return 0
	case "low":
		return 4000
	case "medium":
		return 8000
	case "high":
		return 16000
	case "xhigh":
		return 24000
	case "max":
		return 32000
	default:
		return 0
	}
}

// ---------------------------------------------------------------------------
// Tool subsets
// ---------------------------------------------------------------------------

// CodingTools returns the full set of built-in coding tools:
// bash, read, write, edit, grep, ls, find.
func CodingTools() []types.AgentTool {
	return []types.AgentTool{
		tools.BashTool(),
		tools.ReadTool(),
		tools.WriteTool(),
		tools.EditTool(),
		tools.GrepTool(),
		tools.LsTool(),
		tools.FindTool(),
	}
}

// ReadOnlyTools returns a read-only subset of tools:
// read, grep, ls, find (no bash, write, or edit).
func ReadOnlyTools() []types.AgentTool {
	return []types.AgentTool{
		tools.ReadTool(),
		tools.GrepTool(),
		tools.LsTool(),
		tools.FindTool(),
	}
}

// ---------------------------------------------------------------------------
// Convenience methods
// ---------------------------------------------------------------------------

// Getwd returns the engine's working directory.
func (e *Engine) Getwd() string {
	return e.Config.CWD
}

// hasAuthEnv checks if any known API key env var is set.
func hasAuthEnv() bool {
	return os.Getenv("LLM_API_KEY") != "" || os.Getenv("ANTHROPIC_API_KEY") != "" || os.Getenv("OPENAI_API_KEY") != ""
}

// defaultModelForProvider returns a sensible default model for the provider.
func defaultModelForProvider(baseURL string) string {
	if strings.Contains(baseURL, "anthropic.com") {
		return "claude-sonnet-4-20250514"
	}
	return "gpt-4o"
}

// clampThinkingLevel ensures the thinking level is compatible with the model.
func clampThinkingLevel(model, level string) string {
	if level == "" || level == "off" {
		return level
	}
	// o-series models support thinking; other models may not
	lowerModel := strings.ToLower(model)
	if strings.HasPrefix(lowerModel, "o1") || strings.HasPrefix(lowerModel, "o3") || strings.HasPrefix(lowerModel, "o4") {
		return level
	}
	// Claude models support thinking
	if strings.Contains(lowerModel, "claude") {
		return level
	}
	// Other models: disable thinking by default
	if level == "low" || level == "medium" {
		return level
	}
	return "low" // clamp to minimal for non-thinking models
}

// RestoreSession creates an Engine from an existing session.
func RestoreEngine(opts EngineOptions, sessionID string) (*Engine, error) {
	eng, err := NewEngine(opts)
	if err != nil {
		return nil, err
	}
	sess, err := eng.SessionManager.Load(sessionID, opts.CWD)
	if err != nil {
		return nil, fmt.Errorf("restore session %s: %w", sessionID, err)
	}
	eng.Session = sess
	if sess.Model != "" && opts.Model == "" {
		eng.Model = sess.Model
	}
	return eng, nil
}

// ModelFallbackMessage returns a message if the configured model differs from the saved one.
func ModelFallbackMessage(savedModel, currentModel string) string {
	if savedModel != "" && currentModel != "" && savedModel != currentModel {
		return fmt.Sprintf("Note: session was created with model %s, but using %s instead.", savedModel, currentModel)
	}
	return ""
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
