package settings

import (
	"fmt"
)

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
