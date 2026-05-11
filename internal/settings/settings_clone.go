package settings

import (
)

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

