// Package settings provides configuration loading, saving, and merging
// for xihu agent settings. Settings can be stored in a global user-level
// file (~/.pi/agent/settings.json) and per-project (.pi/settings.json),
// with project settings overriding global ones.
package settings

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
)

// ---------------------------------------------------------------------------
// Nested settings types
// ---------------------------------------------------------------------------

// ThinkingBudgetsSettings holds custom token budgets for each thinking level.
type ThinkingBudgetsSettings struct {
	Minimal int `json:"minimal,omitempty"`
	Low     int `json:"low,omitempty"`
	Medium  int `json:"medium,omitempty"`
	High    int `json:"high,omitempty"`
}

// ImageSettings holds image-related settings.
type ImageSettings struct {
	AutoResize  *bool `json:"auto_resize,omitempty"`
	BlockImages *bool `json:"block_images,omitempty"`
}

// TerminalSettings holds terminal display settings.
type TerminalSettings struct {
	ShowImages           *bool `json:"show_images,omitempty"`
	ImageWidthCells      int   `json:"image_width_cells,omitempty"`
	ClearOnShrink        *bool `json:"clear_on_shrink,omitempty"`
	ShowTerminalProgress *bool `json:"show_terminal_progress,omitempty"`
}

// ProviderRetrySettings holds provider-specific retry settings.
type ProviderRetrySettings struct {
	TimeoutMs       int `json:"timeout_ms,omitempty"`
	MaxRetries      int `json:"max_retries,omitempty"`
	MaxRetryDelayMs int `json:"max_retry_delay_ms,omitempty"`
}

// RetrySettings holds retry behavior settings.
type RetrySettings struct {
	Enabled         *bool                 `json:"enabled,omitempty"`
	MaxRetries      int                   `json:"max_retries,omitempty"`
	BaseDelayMs     int                   `json:"base_delay_ms,omitempty"`
	TimeoutMs       int                   `json:"timeout_ms,omitempty"`
	MaxRetryDelayMs int                   `json:"max_retry_delay_ms,omitempty"`
	Provider        *ProviderRetrySettings `json:"provider,omitempty"`
}

// BranchSummarySettings holds branch summary settings.
type BranchSummarySettings struct {
	ReserveTokens int   `json:"reserve_tokens,omitempty"`
	SkipPrompt    *bool `json:"skip_prompt,omitempty"`
}

// MarkdownSettings holds markdown rendering settings.
type MarkdownSettings struct {
	CodeBlockIndent string `json:"code_block_indent,omitempty"`
}

// PackageSource represents a package source configuration.
type PackageSource struct {
	Name    string `json:"name"`
	Version string `json:"version,omitempty"`
	Source  string `json:"source,omitempty"` // npm, git, local
	Path    string `json:"path,omitempty"`
}

// WarningSettings holds warning-related settings.
type WarningSettings struct {
	AnthropicExtraUsage *bool `json:"anthropic_extra_usage,omitempty"`
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

// Settings holds all configurable options for the xihu agent.
// All fields are optional — zero/nil values mean "use default".
// Pointer types are used for bool fields so that "false" explicitly
// set in a JSON file can override a "true" from a parent config.
type Settings struct {
	// DefaultProvider is the default LLM provider (e.g. "openai", "anthropic").
	DefaultProvider string `json:"default_provider,omitempty"`

	// DefaultModel is the default model name (e.g. "gpt-4o", "claude-sonnet-4-20250514").
	DefaultModel string `json:"default_model,omitempty"`

	// DefaultThinkingLevel sets the thinking/reasoning level for models that support it.
	DefaultThinkingLevel string `json:"default_thinking_level,omitempty"`

	// Theme sets the UI theme (e.g. "dark", "light").
	Theme string `json:"theme,omitempty"`

	// CompactionEnabled controls whether automatic context compaction is active.
	// nil means "not configured" (inherit from parent config).
	CompactionEnabled *bool `json:"compaction_enabled,omitempty"`

	// CompactionReserveTokens is the token budget threshold. Compaction only
	// triggers when total tokens exceed this value. 0 means use default.
	CompactionReserveTokens int `json:"compaction_reserve_tokens,omitempty"`

	// CompactionKeepRecentTokens is the token budget for recent messages
	// kept uncompacted. 0 means use default.
	CompactionKeepRecentTokens int `json:"compaction_keep_recent_tokens,omitempty"`

	// ShellPath is the path to the shell binary (e.g. "/bin/bash").
	ShellPath string `json:"shell_path,omitempty"`

	// ShellCommandPrefix is a prefix prepended to every shell command
	// (e.g. "source ~/.bashrc && ").
	ShellCommandPrefix string `json:"shell_command_prefix,omitempty"`

	// MaxTurns is the maximum number of agent loop turns before stopping.
	// 0 means use default.
	MaxTurns int `json:"max_turns,omitempty"`

	// SystemPrompt is a custom system prompt that overrides the built-in default.
	SystemPrompt string `json:"system_prompt,omitempty"`

	// Extensions is a list of file paths to extension modules.
	Extensions []string `json:"extensions,omitempty"`

	// Skills is a list of file paths to skill definitions.
	Skills []string `json:"skills,omitempty"`

	// Prompts is a list of file paths to custom prompt files.
	Prompts []string `json:"prompts,omitempty"`

	// EnableSkillCommands controls whether slash-command skill invocation is enabled.
	// nil means "not configured" (inherit from parent config).
	EnableSkillCommands *bool `json:"enable_skill_commands,omitempty"`

	// --- New fields below ---

	// ThinkingLevel sets the thinking/reasoning level (minimal/low/medium/high).
	ThinkingLevel string `json:"thinking_level,omitempty"`

	// ThinkingBudgets holds custom token budgets for thinking levels.
	ThinkingBudgets *ThinkingBudgetsSettings `json:"thinking_budgets,omitempty"`

	// HideThinkingBlock controls whether thinking blocks are hidden in output.
	HideThinkingBlock *bool `json:"hide_thinking_block,omitempty"`

	// Images holds image-related settings.
	Images *ImageSettings `json:"images,omitempty"`

	// Terminal holds terminal display settings.
	Terminal *TerminalSettings `json:"terminal,omitempty"`

	// Retry holds retry behavior settings.
	Retry *RetrySettings `json:"retry,omitempty"`

	// BranchSummary holds branch summary settings.
	BranchSummary *BranchSummarySettings `json:"branch_summary,omitempty"`

	// QuietStartup suppresses startup messages when true.
	QuietStartup *bool `json:"quiet_startup,omitempty"`

	// NpmCommand is the npm command to use for package operations (argv-style).
	NpmCommand []string `json:"npm_command,omitempty"`

	// CollapseChangelog shows condensed changelog after update.
	CollapseChangelog *bool `json:"collapse_changelog,omitempty"`

	// EditorPaddingX is the horizontal padding for the input editor.
	EditorPaddingX int `json:"editor_padding_x,omitempty"`

	// AutocompleteMaxVisible is the max visible items in autocomplete dropdown.
	AutocompleteMaxVisible int `json:"autocomplete_max_visible,omitempty"`

	// ShowHardwareCursor controls terminal cursor visibility for IME support.
	ShowHardwareCursor *bool `json:"show_hardware_cursor,omitempty"`

	// Markdown holds markdown rendering settings.
	Markdown *MarkdownSettings `json:"markdown,omitempty"`

	// Warnings holds warning-related settings.
	Warnings *WarningSettings `json:"warnings,omitempty"`

	// SessionDir is a custom session storage directory.
	SessionDir string `json:"session_dir,omitempty"`

	// ScopedModels is a list of scoped model patterns.
	ScopedModels []string `json:"scoped_models,omitempty"`

	// DoubleEscapeAction is the action for double-escape with empty editor.
	DoubleEscapeAction string `json:"double_escape_action,omitempty"`

	// TreeFilterMode is the default filter when opening /tree.
	TreeFilterMode string `json:"tree_filter_mode,omitempty"`

	// EnabledModels is a list of enabled model patterns for cycling.
	EnabledModels []string `json:"enabled_models,omitempty"`

	// Transport is the transport mechanism (sse/websocket).
	Transport string `json:"transport,omitempty"`

	// SteeringMode controls steering behavior (all/one-at-a-time).
	SteeringMode string `json:"steering_mode,omitempty"`

	// FollowUpMode controls follow-up behavior (all/one-at-a-time).
	FollowUpMode string `json:"follow_up_mode,omitempty"`

	// EnableInstallTelemetry opt-in to installation telemetry.
	EnableInstallTelemetry *bool `json:"enable_install_telemetry,omitempty"`

	// Packages is a list of package sources.
	Packages []PackageSource `json:"packages,omitempty"`

	// Themes is a list of file paths to custom themes.
	Themes []string `json:"themes,omitempty"`

	// LastChangelogVersion tracks the last viewed changelog version.
	LastChangelogVersion string `json:"last_changelog_version,omitempty"`
}

// LoadSettings reads and parses a Settings JSON file from the given path.
// Returns an empty Settings (with no error) if the file does not exist.
func LoadSettings(path string) (*Settings, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		if os.IsNotExist(err) {
			return &Settings{}, nil
		}
		return nil, fmt.Errorf("read settings file %s: %w", path, err)
	}

	var s Settings
	if err := json.Unmarshal(data, &s); err != nil {
		return nil, fmt.Errorf("parse settings file %s: %w", path, err)
	}
	return &s, nil
}

// SaveSettings writes a Settings struct to a JSON file at the given path.
// Parent directories are created if they don't exist.
func SaveSettings(path string, s *Settings) error {
	if s == nil {
		s = &Settings{}
	}

	dir := filepath.Dir(path)
	if err := os.MkdirAll(dir, 0755); err != nil {
		return fmt.Errorf("create settings directory %s: %w", dir, err)
	}

	data, err := json.MarshalIndent(s, "", "  ")
	if err != nil {
		return fmt.Errorf("marshal settings: %w", err)
	}

	// Append newline for POSIX-friendly files
	data = append(data, '\n')

	if err := os.WriteFile(path, data, 0644); err != nil {
		return fmt.Errorf("write settings file %s: %w", path, err)
	}
	return nil
}

// MergeSettings performs a deep merge of two Settings structs.
// Returns a new Settings where any non-zero (or non-nil) field in
// override takes precedence over the corresponding field in base.
// Slices from override replace base slices entirely (no append).
// Nested struct pointers are deep-merged field by field.
//
// If base is nil, a copy of override is returned.
// If override is nil, a copy of base is returned.
// If both are nil, an empty Settings is returned.
