package settings

import (
	"fmt"
	"os"
	"path/filepath"
)

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
