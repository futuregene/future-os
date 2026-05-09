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
	"time"
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
func MergeSettings(base, override *Settings) *Settings {
	if base == nil && override == nil {
		return &Settings{}
	}
	if base == nil {
		return override.clone()
	}
	if override == nil {
		return base.clone()
	}

	merged := base.clone()

	if override.DefaultProvider != "" {
		merged.DefaultProvider = override.DefaultProvider
	}
	if override.DefaultModel != "" {
		merged.DefaultModel = override.DefaultModel
	}
	if override.DefaultThinkingLevel != "" {
		merged.DefaultThinkingLevel = override.DefaultThinkingLevel
	}
	if override.Theme != "" {
		merged.Theme = override.Theme
	}
	if override.CompactionEnabled != nil {
		merged.CompactionEnabled = boolPtr(*override.CompactionEnabled)
	}
	if override.CompactionReserveTokens != 0 {
		merged.CompactionReserveTokens = override.CompactionReserveTokens
	}
	if override.CompactionKeepRecentTokens != 0 {
		merged.CompactionKeepRecentTokens = override.CompactionKeepRecentTokens
	}
	if override.ShellPath != "" {
		merged.ShellPath = override.ShellPath
	}
	if override.ShellCommandPrefix != "" {
		merged.ShellCommandPrefix = override.ShellCommandPrefix
	}
	if override.MaxTurns != 0 {
		merged.MaxTurns = override.MaxTurns
	}
	if override.SystemPrompt != "" {
		merged.SystemPrompt = override.SystemPrompt
	}
	if override.Extensions != nil {
		merged.Extensions = copyStringSlice(override.Extensions)
	}
	if override.Skills != nil {
		merged.Skills = copyStringSlice(override.Skills)
	}
	if override.Prompts != nil {
		merged.Prompts = copyStringSlice(override.Prompts)
	}
	if override.EnableSkillCommands != nil {
		merged.EnableSkillCommands = boolPtr(*override.EnableSkillCommands)
	}

	// --- New field merges ---

	if override.ThinkingLevel != "" {
		merged.ThinkingLevel = override.ThinkingLevel
	}

	merged.ThinkingBudgets = mergeThinkingBudgets(merged.ThinkingBudgets, override.ThinkingBudgets)

	if override.HideThinkingBlock != nil {
		merged.HideThinkingBlock = boolPtr(*override.HideThinkingBlock)
	}

	merged.Images = mergeImageSettings(merged.Images, override.Images)

	merged.Terminal = mergeTerminalSettings(merged.Terminal, override.Terminal)

	merged.Retry = mergeRetrySettings(merged.Retry, override.Retry)

	merged.BranchSummary = mergeBranchSummarySettings(merged.BranchSummary, override.BranchSummary)

	if override.QuietStartup != nil {
		merged.QuietStartup = boolPtr(*override.QuietStartup)
	}

	if override.NpmCommand != nil {
		merged.NpmCommand = copyStringSlice(override.NpmCommand)
	}

	if override.CollapseChangelog != nil {
		merged.CollapseChangelog = boolPtr(*override.CollapseChangelog)
	}

	if override.EditorPaddingX != 0 {
		merged.EditorPaddingX = override.EditorPaddingX
	}

	if override.AutocompleteMaxVisible != 0 {
		merged.AutocompleteMaxVisible = override.AutocompleteMaxVisible
	}

	if override.ShowHardwareCursor != nil {
		merged.ShowHardwareCursor = boolPtr(*override.ShowHardwareCursor)
	}

	merged.Markdown = mergeMarkdownSettings(merged.Markdown, override.Markdown)

	merged.Warnings = mergeWarningSettings(merged.Warnings, override.Warnings)

	if override.SessionDir != "" {
		merged.SessionDir = override.SessionDir
	}

	if override.ScopedModels != nil {
		merged.ScopedModels = copyStringSlice(override.ScopedModels)
	}

	if override.DoubleEscapeAction != "" {
		merged.DoubleEscapeAction = override.DoubleEscapeAction
	}

	if override.TreeFilterMode != "" {
		merged.TreeFilterMode = override.TreeFilterMode
	}

	if override.EnabledModels != nil {
		merged.EnabledModels = copyStringSlice(override.EnabledModels)
	}

	if override.Transport != "" {
		merged.Transport = override.Transport
	}

	if override.SteeringMode != "" {
		merged.SteeringMode = override.SteeringMode
	}

	if override.FollowUpMode != "" {
		merged.FollowUpMode = override.FollowUpMode
	}
	if override.EnableInstallTelemetry != nil {
		merged.EnableInstallTelemetry = boolPtr(*override.EnableInstallTelemetry)
	}
	if override.Packages != nil {
		merged.Packages = copyPackageSources(override.Packages)
	}
	if override.Themes != nil {
		merged.Themes = copyStringSlice(override.Themes)
	}
	if override.LastChangelogVersion != "" {
		merged.LastChangelogVersion = override.LastChangelogVersion
	}
	if override.NpmCommand != nil {
		merged.NpmCommand = copyStringSlice(override.NpmCommand)
	}

	return merged
}

// GetDefaultPaths returns the standard paths for global and project-level
// settings files. The global path is ~/.xihu/settings.json.
// The project path is .xihu/settings.json relative to the current working
// directory (or the provided working directory if non-empty).
func GetDefaultPaths() (global, project string) {
	home, err := os.UserHomeDir()
	if err != nil {
		home = os.TempDir()
	}
	global = filepath.Join(home, ".xihu", "settings.json")

	cwd, err := os.Getwd()
	if err != nil {
		cwd = "."
	}
	project = filepath.Join(cwd, ".xihu", "settings.json")

	return global, project
}

// LoadAll loads both global and project settings, merges them (project
// overrides global), and returns the merged result. If neither file
// exists, returns an empty Settings with no error.
func LoadAll() (*Settings, error) {
	globalPath, projectPath := GetDefaultPaths()

	globalSettings, err := LoadSettings(globalPath)
	if err != nil {
		return nil, fmt.Errorf("load global settings: %w", err)
	}

	projectSettings, err := LoadSettings(projectPath)
	if err != nil {
		return nil, fmt.Errorf("load project settings: %w", err)
	}

	return MergeSettings(globalSettings, projectSettings), nil
}

// ---------------------------------------------------------------------------
// Internal helpers: clone
// ---------------------------------------------------------------------------

// clone returns a deep copy of the Settings.
func (s *Settings) clone() *Settings {
	if s == nil {
		return &Settings{}
	}
	c := *s // shallow copy

	// Deep copy pointer fields (existing)
	if s.CompactionEnabled != nil {
		c.CompactionEnabled = boolPtr(*s.CompactionEnabled)
	}
	if s.EnableSkillCommands != nil {
		c.EnableSkillCommands = boolPtr(*s.EnableSkillCommands)
	}
	c.Extensions = copyStringSlice(s.Extensions)
	c.Skills = copyStringSlice(s.Skills)
	c.Prompts = copyStringSlice(s.Prompts)

	// Deep copy new pointer fields
	if s.HideThinkingBlock != nil {
		c.HideThinkingBlock = boolPtr(*s.HideThinkingBlock)
	}
	if s.QuietStartup != nil {
		c.QuietStartup = boolPtr(*s.QuietStartup)
	}
	if s.CollapseChangelog != nil {
		c.CollapseChangelog = boolPtr(*s.CollapseChangelog)
	}
	if s.ShowHardwareCursor != nil {
		c.ShowHardwareCursor = boolPtr(*s.ShowHardwareCursor)
	}

	// Deep copy nested struct pointers
	c.ThinkingBudgets = cloneThinkingBudgets(s.ThinkingBudgets)
	c.Images = cloneImageSettings(s.Images)
	c.Terminal = cloneTerminalSettings(s.Terminal)
	c.Retry = cloneRetrySettings(s.Retry)
	c.BranchSummary = cloneBranchSummarySettings(s.BranchSummary)
	c.Markdown = cloneMarkdownSettings(s.Markdown)
	c.Warnings = cloneWarningSettings(s.Warnings)

	// Deep copy new slices
	c.ScopedModels = copyStringSlice(s.ScopedModels)
	c.EnabledModels = copyStringSlice(s.EnabledModels)
	c.Themes = copyStringSlice(s.Themes)
	c.NpmCommand = copyStringSlice(s.NpmCommand)

	if s.EnableInstallTelemetry != nil {
		c.EnableInstallTelemetry = boolPtr(*s.EnableInstallTelemetry)
	}
	c.Packages = copyPackageSources(s.Packages)

	return &c
}

// ---------------------------------------------------------------------------
// Internal helpers: nested struct clone
// ---------------------------------------------------------------------------

func cloneThinkingBudgets(s *ThinkingBudgetsSettings) *ThinkingBudgetsSettings {
	if s == nil {
		return nil
	}
	c := *s
	return &c
}

func cloneImageSettings(s *ImageSettings) *ImageSettings {
	if s == nil {
		return nil
	}
	c := *s
	if s.AutoResize != nil {
		c.AutoResize = boolPtr(*s.AutoResize)
	}
	if s.BlockImages != nil {
		c.BlockImages = boolPtr(*s.BlockImages)
	}
	return &c
}

func cloneTerminalSettings(s *TerminalSettings) *TerminalSettings {
	if s == nil {
		return nil
	}
	c := *s
	if s.ShowImages != nil {
		c.ShowImages = boolPtr(*s.ShowImages)
	}
	if s.ClearOnShrink != nil {
		c.ClearOnShrink = boolPtr(*s.ClearOnShrink)
	}
	if s.ShowTerminalProgress != nil {
		c.ShowTerminalProgress = boolPtr(*s.ShowTerminalProgress)
	}
	return &c
}

func cloneRetrySettings(s *RetrySettings) *RetrySettings {
	if s == nil {
		return nil
	}
	c := *s
	if s.Enabled != nil {
		c.Enabled = boolPtr(*s.Enabled)
	}
	if s.Provider != nil {
		pc := *s.Provider
		c.Provider = &pc
	}
	return &c
}

func cloneBranchSummarySettings(s *BranchSummarySettings) *BranchSummarySettings {
	if s == nil {
		return nil
	}
	c := *s
	if s.SkipPrompt != nil {
		c.SkipPrompt = boolPtr(*s.SkipPrompt)
	}
	return &c
}

func cloneMarkdownSettings(s *MarkdownSettings) *MarkdownSettings {
	if s == nil {
		return nil
	}
	c := *s
	return &c
}

func cloneWarningSettings(s *WarningSettings) *WarningSettings {
	if s == nil {
		return nil
	}
	c := *s
	if s.AnthropicExtraUsage != nil {
		c.AnthropicExtraUsage = boolPtr(*s.AnthropicExtraUsage)
	}
	return &c
}

// ---------------------------------------------------------------------------
// Internal helpers: nested struct merge
// ---------------------------------------------------------------------------

func mergeThinkingBudgets(base, override *ThinkingBudgetsSettings) *ThinkingBudgetsSettings {
	if override == nil {
		return base
	}
	if base == nil {
		return cloneThinkingBudgets(override)
	}
	merged := cloneThinkingBudgets(base)
	if override.Minimal != 0 {
		merged.Minimal = override.Minimal
	}
	if override.Low != 0 {
		merged.Low = override.Low
	}
	if override.Medium != 0 {
		merged.Medium = override.Medium
	}
	if override.High != 0 {
		merged.High = override.High
	}
	return merged
}

func mergeImageSettings(base, override *ImageSettings) *ImageSettings {
	if override == nil {
		return base
	}
	if base == nil {
		return cloneImageSettings(override)
	}
	merged := cloneImageSettings(base)
	if override.AutoResize != nil {
		merged.AutoResize = boolPtr(*override.AutoResize)
	}
	if override.BlockImages != nil {
		merged.BlockImages = boolPtr(*override.BlockImages)
	}
	return merged
}

func mergeTerminalSettings(base, override *TerminalSettings) *TerminalSettings {
	if override == nil {
		return base
	}
	if base == nil {
		return cloneTerminalSettings(override)
	}
	merged := cloneTerminalSettings(base)
	if override.ShowImages != nil {
		merged.ShowImages = boolPtr(*override.ShowImages)
	}
	if override.ImageWidthCells != 0 {
		merged.ImageWidthCells = override.ImageWidthCells
	}
	if override.ClearOnShrink != nil {
		merged.ClearOnShrink = boolPtr(*override.ClearOnShrink)
	}
	if override.ShowTerminalProgress != nil {
		merged.ShowTerminalProgress = boolPtr(*override.ShowTerminalProgress)
	}
	return merged
}

func mergeProviderRetrySettings(base, override *ProviderRetrySettings) *ProviderRetrySettings {
	if override == nil {
		return base
	}
	if base == nil {
		c := *override
		return &c
	}
	merged := *base
	if override.TimeoutMs != 0 {
		merged.TimeoutMs = override.TimeoutMs
	}
	if override.MaxRetries != 0 {
		merged.MaxRetries = override.MaxRetries
	}
	if override.MaxRetryDelayMs != 0 {
		merged.MaxRetryDelayMs = override.MaxRetryDelayMs
	}
	return &merged
}

func mergeRetrySettings(base, override *RetrySettings) *RetrySettings {
	if override == nil {
		return base
	}
	if base == nil {
		return cloneRetrySettings(override)
	}
	merged := cloneRetrySettings(base)
	if override.Enabled != nil {
		merged.Enabled = boolPtr(*override.Enabled)
	}
	if override.MaxRetries != 0 {
		merged.MaxRetries = override.MaxRetries
	}
	if override.BaseDelayMs != 0 {
		merged.BaseDelayMs = override.BaseDelayMs
	}
	if override.TimeoutMs != 0 {
		merged.TimeoutMs = override.TimeoutMs
	}
	if override.MaxRetryDelayMs != 0 {
		merged.MaxRetryDelayMs = override.MaxRetryDelayMs
	}
	merged.Provider = mergeProviderRetrySettings(merged.Provider, override.Provider)
	return merged
}

func mergeBranchSummarySettings(base, override *BranchSummarySettings) *BranchSummarySettings {
	if override == nil {
		return base
	}
	if base == nil {
		return cloneBranchSummarySettings(override)
	}
	merged := cloneBranchSummarySettings(base)
	if override.ReserveTokens != 0 {
		merged.ReserveTokens = override.ReserveTokens
	}
	if override.SkipPrompt != nil {
		merged.SkipPrompt = boolPtr(*override.SkipPrompt)
	}
	return merged
}

func mergeMarkdownSettings(base, override *MarkdownSettings) *MarkdownSettings {
	if override == nil {
		return base
	}
	if base == nil {
		return cloneMarkdownSettings(override)
	}
	merged := cloneMarkdownSettings(base)
	if override.CodeBlockIndent != "" {
		merged.CodeBlockIndent = override.CodeBlockIndent
	}
	return merged
}

func mergeWarningSettings(base, override *WarningSettings) *WarningSettings {
	if override == nil {
		return base
	}
	if base == nil {
		return cloneWarningSettings(override)
	}
	merged := cloneWarningSettings(base)
	if override.AnthropicExtraUsage != nil {
		merged.AnthropicExtraUsage = boolPtr(*override.AnthropicExtraUsage)
	}
	return merged
}

// ---------------------------------------------------------------------------
// Internal helpers: primitives
// ---------------------------------------------------------------------------

// copyPackageSources returns a deep copy of a PackageSource slice.
func copyPackageSources(src []PackageSource) []PackageSource {
	if src == nil {
		return nil
	}
	dst := make([]PackageSource, len(src))
	copy(dst, src)
	return dst
}

// boolPtr returns a pointer to the given bool.
func boolPtr(b bool) *bool {
	return &b
}

// copyStringSlice returns a copy of a string slice, or nil if the input is nil.
func copyStringSlice(src []string) []string {
	if src == nil {
		return nil
	}
	dst := make([]string, len(src))
	copy(dst, src)
	return dst
}

// ---------------------------------------------------------------------------
// Settings locking
// ---------------------------------------------------------------------------

// lockPath returns the lock file path for a settings file.
func lockPath(path string) string {
	return path + ".lock"
}

// LockSettings acquires an exclusive lock on a settings file.
// The lock is a best-effort mechanism: it creates a .lock file using O_EXCL.
// Returns an error if already locked by another process.
func LockSettings(path string) error {
	lockFile := lockPath(path)
	f, err := os.OpenFile(lockFile, os.O_CREATE|os.O_EXCL|os.O_WRONLY, 0644)
	if err != nil {
		if os.IsExist(err) {
			return fmt.Errorf("settings file is locked: %s", path)
		}
		return fmt.Errorf("lock settings: %w", err)
	}
	// Write lock metadata
	fmt.Fprintf(f, "%d\n%s\n", os.Getpid(), time.Now().Format(time.RFC3339))
	f.Close()
	return nil
}

// UnlockSettings releases the lock on a settings file.
func UnlockSettings(path string) error {
	lockFile := lockPath(path)
	if err := os.Remove(lockFile); err != nil && !os.IsNotExist(err) {
		return fmt.Errorf("unlock settings: %w", err)
	}
	return nil
}

// IsLocked checks if a settings file is currently locked.
func IsLocked(path string) bool {
	_, err := os.Stat(lockPath(path))
	return err == nil
}

// ---------------------------------------------------------------------------
// Settings migration
// ---------------------------------------------------------------------------

// MigrateSettings applies field name and format migrations from older settings formats.
//
// Migration history:
//   v1\u2192v2: queueMode \u2192 steeringMode, websockets transport value \u2192 sse
//   v2\u2192v3: skills object notation ({"file1": true}) \u2192 array notation (["file1"]),
//              retry.maxDelayMs field removed (moved to retry.provider.maxRetryDelayMs)
func MigrateSettings(s *Settings) {
	// v1\u2192v2: queueMode \u2192 steeringMode
	// This is handled by JSON tags (steering_mode), but if an old file
	// used queueMode as a key, it would be ignored. We leave the struct
	// as-is since JSON unmarshalling into the new fields handles it.

	// v2\u2192v3: If skills somehow loaded as map[string]bool instead of []string,
	// we can't detect that from a typed struct. The Skiils field is []string.
	// Any old format with retry.maxDelayMs at top level is naturally ignored
	// since the field no longer exists.

	// Re-validate thinking level
	validLevels := map[string]bool{
		"off": true, "minimal": true, "low": true, "medium": true,
		"high": true, "xhigh": true, "max": true, "": true,
	}
	if !validLevels[s.DefaultThinkingLevel] {
		s.DefaultThinkingLevel = "" // reset invalid value
	}
	if !validLevels[s.ThinkingLevel] {
		s.ThinkingLevel = "" // reset invalid value
	}

	// Re-validate doubleEscapeAction and treeFilterMode
	validEscape := map[string]bool{"fork": true, "tree": true, "none": true, "": true}
	if !validEscape[s.DoubleEscapeAction] {
		s.DoubleEscapeAction = ""
	}
	validTreeFilter := map[string]bool{"all": true, "default": true, "user-only": true, "no-tools": true, "labeled-only": true, "": true}
	if !validTreeFilter[s.TreeFilterMode] {
		s.TreeFilterMode = ""
	}

	// Re-validate steeringMode and followUpMode
	validModes := map[string]bool{"all": true, "one-at-a-time": true, "": true}
	if !validModes[s.SteeringMode] {
		s.SteeringMode = ""
	}
	if !validModes[s.FollowUpMode] {
		s.FollowUpMode = ""
	}
}

// ---------------------------------------------------------------------------
// Reload
// ---------------------------------------------------------------------------

// Reload re-reads settings from the last known path and applies overrides.
// The path parameter is the settings file to reload from.
// This is useful when settings have been modified externally.
func (s *Settings) Reload(path string) error {
	if path == "" {
		return fmt.Errorf("reload: path is empty")
	}
	newSettings, err := LoadSettings(path)
	if err != nil {
		return fmt.Errorf("reload settings from %s: %w", path, err)
	}
	// Copy all fields from loaded settings
	*s = *newSettings
	return nil
}

// ---------------------------------------------------------------------------
// ApplyOverrides
// ---------------------------------------------------------------------------

// ApplyOverrides applies non-zero/non-nil values from overrides onto this Settings.
// This is an in-place merge (unlike MergeSettings which returns a new Settings).
// Useful for applying CLI flags or template overrides on top of existing settings.
func (s *Settings) ApplyOverrides(overrides *Settings) {
	if overrides == nil {
		return
	}
	if overrides.DefaultProvider != "" {
		s.DefaultProvider = overrides.DefaultProvider
	}
	if overrides.DefaultModel != "" {
		s.DefaultModel = overrides.DefaultModel
	}
	if overrides.DefaultThinkingLevel != "" {
		s.DefaultThinkingLevel = overrides.DefaultThinkingLevel
	}
	if overrides.Theme != "" {
		s.Theme = overrides.Theme
	}
	if overrides.CompactionEnabled != nil {
		s.CompactionEnabled = boolPtr(*overrides.CompactionEnabled)
	}
	if overrides.CompactionReserveTokens != 0 {
		s.CompactionReserveTokens = overrides.CompactionReserveTokens
	}
	if overrides.CompactionKeepRecentTokens != 0 {
		s.CompactionKeepRecentTokens = overrides.CompactionKeepRecentTokens
	}
	if overrides.ShellPath != "" {
		s.ShellPath = overrides.ShellPath
	}
	if overrides.ShellCommandPrefix != "" {
		s.ShellCommandPrefix = overrides.ShellCommandPrefix
	}
	if overrides.MaxTurns != 0 {
		s.MaxTurns = overrides.MaxTurns
	}
	if overrides.SystemPrompt != "" {
		s.SystemPrompt = overrides.SystemPrompt
	}
	if overrides.Extensions != nil {
		s.Extensions = copyStringSlice(overrides.Extensions)
	}
	if overrides.Skills != nil {
		s.Skills = copyStringSlice(overrides.Skills)
	}
	if overrides.Prompts != nil {
		s.Prompts = copyStringSlice(overrides.Prompts)
	}
	if overrides.EnableSkillCommands != nil {
		s.EnableSkillCommands = boolPtr(*overrides.EnableSkillCommands)
	}
	if overrides.ThinkingLevel != "" {
		s.ThinkingLevel = overrides.ThinkingLevel
	}
	if overrides.ThinkingBudgets != nil {
		s.ThinkingBudgets = cloneThinkingBudgets(overrides.ThinkingBudgets)
	}
	if overrides.HideThinkingBlock != nil {
		s.HideThinkingBlock = boolPtr(*overrides.HideThinkingBlock)
	}
	if overrides.Images != nil {
		s.Images = cloneImageSettings(overrides.Images)
	}
	if overrides.Terminal != nil {
		s.Terminal = cloneTerminalSettings(overrides.Terminal)
	}
	if overrides.Retry != nil {
		s.Retry = cloneRetrySettings(overrides.Retry)
	}
	if overrides.BranchSummary != nil {
		s.BranchSummary = cloneBranchSummarySettings(overrides.BranchSummary)
	}
	if overrides.QuietStartup != nil {
		s.QuietStartup = boolPtr(*overrides.QuietStartup)
	}
	if overrides.NpmCommand != nil {
		s.NpmCommand = copyStringSlice(overrides.NpmCommand)
	}
	if overrides.CollapseChangelog != nil {
		s.CollapseChangelog = boolPtr(*overrides.CollapseChangelog)
	}
	if overrides.EditorPaddingX != 0 {
		s.EditorPaddingX = overrides.EditorPaddingX
	}
	if overrides.AutocompleteMaxVisible != 0 {
		s.AutocompleteMaxVisible = overrides.AutocompleteMaxVisible
	}
	if overrides.ShowHardwareCursor != nil {
		s.ShowHardwareCursor = boolPtr(*overrides.ShowHardwareCursor)
	}
	if overrides.Markdown != nil {
		s.Markdown = cloneMarkdownSettings(overrides.Markdown)
	}
	if overrides.Warnings != nil {
		s.Warnings = cloneWarningSettings(overrides.Warnings)
	}
	if overrides.SessionDir != "" {
		s.SessionDir = overrides.SessionDir
	}
	if overrides.ScopedModels != nil {
		s.ScopedModels = copyStringSlice(overrides.ScopedModels)
	}
	if overrides.DoubleEscapeAction != "" {
		s.DoubleEscapeAction = overrides.DoubleEscapeAction
	}
	if overrides.TreeFilterMode != "" {
		s.TreeFilterMode = overrides.TreeFilterMode
	}
	if overrides.EnabledModels != nil {
		s.EnabledModels = copyStringSlice(overrides.EnabledModels)
	}
	if overrides.Transport != "" {
		s.Transport = overrides.Transport
	}
	if overrides.SteeringMode != "" {
		s.SteeringMode = overrides.SteeringMode
	}
	if overrides.FollowUpMode != "" {
		s.FollowUpMode = overrides.FollowUpMode
	}
	if overrides.EnableInstallTelemetry != nil {
		s.EnableInstallTelemetry = boolPtr(*overrides.EnableInstallTelemetry)
	}
	if overrides.Packages != nil {
		s.Packages = copyPackageSources(overrides.Packages)
	}
	if overrides.Themes != nil {
		s.Themes = copyStringSlice(overrides.Themes)
	}
	if overrides.LastChangelogVersion != "" {
		s.LastChangelogVersion = overrides.LastChangelogVersion
	}
}
