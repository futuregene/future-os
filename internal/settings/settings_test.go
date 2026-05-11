package settings

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// ---------------------------------------------------------------------------
// LoadSettings tests
// ---------------------------------------------------------------------------

func TestLoadSettings_FileExists(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "settings.json")

	content := `{
  "default_provider": "openai",
  "default_model": "gpt-4o",
  "compaction_enabled": true,
  "compaction_reserve_tokens": 100000,
  "max_turns": 50,
  "theme": "dark"
}`
	if err := os.WriteFile(path, []byte(content), 0644); err != nil {
		t.Fatal(err)
	}

	s, err := LoadSettings(path)
	if err != nil {
		t.Fatalf("LoadSettings() error: %v", err)
	}
	if s.DefaultProvider != "openai" {
		t.Errorf("DefaultProvider = %q, want %q", s.DefaultProvider, "openai")
	}
	if s.DefaultModel != "gpt-4o" {
		t.Errorf("DefaultModel = %q, want %q", s.DefaultModel, "gpt-4o")
	}
	if s.CompactionEnabled == nil || *s.CompactionEnabled != true {
		t.Errorf("CompactionEnabled = %v, want true", s.CompactionEnabled)
	}
	if s.CompactionReserveTokens != 100000 {
		t.Errorf("CompactionReserveTokens = %d, want 100000", s.CompactionReserveTokens)
	}
	if s.MaxTurns != 50 {
		t.Errorf("MaxTurns = %d, want 50", s.MaxTurns)
	}
	if s.Theme != "dark" {
		t.Errorf("Theme = %q, want %q", s.Theme, "dark")
	}
}

func TestLoadSettings_FileNotExist(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "nonexistent.json")

	s, err := LoadSettings(path)
	if err != nil {
		t.Fatalf("LoadSettings() error on missing file: %v", err)
	}
	if s == nil {
		t.Fatal("LoadSettings() returned nil for missing file")
	}
	// Should be empty (zero values)
	if s.DefaultProvider != "" {
		t.Errorf("expected empty DefaultProvider, got %q", s.DefaultProvider)
	}
}

func TestLoadSettings_InvalidJSON(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "bad.json")
	if err := os.WriteFile(path, []byte("not json{{{"), 0644); err != nil {
		t.Fatal(err)
	}

	_, err := LoadSettings(path)
	if err == nil {
		t.Fatal("expected error for invalid JSON")
	}
}

func TestLoadSettings_AllFields(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "full.json")

	tru := true
	fals := false
	content := Settings{
		DefaultProvider:          "anthropic",
		DefaultModel:             "claude-sonnet-4-20250514",
		DefaultThinkingLevel:     "high",
		Theme:                    "light",
		CompactionEnabled:        &tru,
		CompactionReserveTokens:  200000,
		CompactionKeepRecentTokens: 50000,
		ShellPath:                "/bin/zsh",
		ShellCommandPrefix:       "source ~/.zshrc && ",
		MaxTurns:                 100,
		SystemPrompt:             "You are a helpful assistant.",
		Extensions:               []string{"/path/to/ext1", "/path/to/ext2"},
		Skills:                   []string{"/path/to/skill1"},
		Prompts:                  []string{"/path/to/prompt1", "/path/to/prompt2"},
		EnableSkillCommands:      &fals,
	}

	data, err := json.MarshalIndent(content, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, data, 0644); err != nil {
		t.Fatal(err)
	}

	s, err := LoadSettings(path)
	if err != nil {
		t.Fatalf("LoadSettings() error: %v", err)
	}

	if s.DefaultProvider != "anthropic" {
		t.Errorf("DefaultProvider = %q", s.DefaultProvider)
	}
	if s.DefaultModel != "claude-sonnet-4-20250514" {
		t.Errorf("DefaultModel = %q", s.DefaultModel)
	}
	if s.DefaultThinkingLevel != "high" {
		t.Errorf("DefaultThinkingLevel = %q", s.DefaultThinkingLevel)
	}
	if s.Theme != "light" {
		t.Errorf("Theme = %q", s.Theme)
	}
	if s.CompactionEnabled == nil || *s.CompactionEnabled != true {
		t.Errorf("CompactionEnabled = %v", s.CompactionEnabled)
	}
	if s.CompactionReserveTokens != 200000 {
		t.Errorf("CompactionReserveTokens = %d", s.CompactionReserveTokens)
	}
	if s.CompactionKeepRecentTokens != 50000 {
		t.Errorf("CompactionKeepRecentTokens = %d", s.CompactionKeepRecentTokens)
	}
	if s.ShellPath != "/bin/zsh" {
		t.Errorf("ShellPath = %q", s.ShellPath)
	}
	if s.ShellCommandPrefix != "source ~/.zshrc && " {
		t.Errorf("ShellCommandPrefix = %q", s.ShellCommandPrefix)
	}
	if s.MaxTurns != 100 {
		t.Errorf("MaxTurns = %d", s.MaxTurns)
	}
	if s.SystemPrompt != "You are a helpful assistant." {
		t.Errorf("SystemPrompt = %q", s.SystemPrompt)
	}
	if len(s.Extensions) != 2 || s.Extensions[0] != "/path/to/ext1" {
		t.Errorf("Extensions = %v", s.Extensions)
	}
	if len(s.Skills) != 1 || s.Skills[0] != "/path/to/skill1" {
		t.Errorf("Skills = %v", s.Skills)
	}
	if len(s.Prompts) != 2 || s.Prompts[1] != "/path/to/prompt2" {
		t.Errorf("Prompts = %v", s.Prompts)
	}
	if s.EnableSkillCommands == nil || *s.EnableSkillCommands != false {
		t.Errorf("EnableSkillCommands = %v, want false", s.EnableSkillCommands)
	}
}

// ---------------------------------------------------------------------------
// SaveSettings tests
// ---------------------------------------------------------------------------

func TestSaveSettings_RoundTrip(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "saved.json")

	tru := true
	original := &Settings{
		DefaultProvider:          "openai",
		DefaultModel:             "gpt-4o",
		CompactionEnabled:        &tru,
		CompactionReserveTokens:  150000,
		MaxTurns:                 42,
		SystemPrompt:             "Custom prompt",
		Extensions:               []string{"ext1", "ext2"},
	}

	if err := SaveSettings(path, original); err != nil {
		t.Fatalf("SaveSettings() error: %v", err)
	}

	loaded, err := LoadSettings(path)
	if err != nil {
		t.Fatalf("LoadSettings() after save error: %v", err)
	}

	if loaded.DefaultProvider != original.DefaultProvider {
		t.Errorf("DefaultProvider = %q, want %q", loaded.DefaultProvider, original.DefaultProvider)
	}
	if loaded.DefaultModel != original.DefaultModel {
		t.Errorf("DefaultModel = %q, want %q", loaded.DefaultModel, original.DefaultModel)
	}
	if loaded.CompactionEnabled == nil || *loaded.CompactionEnabled != true {
		t.Errorf("CompactionEnabled = %v", loaded.CompactionEnabled)
	}
	if loaded.CompactionReserveTokens != 150000 {
		t.Errorf("CompactionReserveTokens = %d", loaded.CompactionReserveTokens)
	}
	if loaded.MaxTurns != 42 {
		t.Errorf("MaxTurns = %d", loaded.MaxTurns)
	}
	if loaded.SystemPrompt != "Custom prompt" {
		t.Errorf("SystemPrompt = %q", loaded.SystemPrompt)
	}
	if len(loaded.Extensions) != 2 {
		t.Errorf("Extensions length = %d", len(loaded.Extensions))
	}
}

func TestSaveSettings_NilInput(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "nil.json")

	if err := SaveSettings(path, nil); err != nil {
		t.Fatalf("SaveSettings(nil) error: %v", err)
	}

	// Should produce a valid empty JSON object
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(data), "{") {
		t.Errorf("expected JSON object, got: %s", string(data))
	}
}

func TestSaveSettings_CreatesParentDirs(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "deep", "nested", "config.json")

	s := &Settings{Theme: "dark"}
	if err := SaveSettings(path, s); err != nil {
		t.Fatalf("SaveSettings() error: %v", err)
	}

	if _, err := os.Stat(path); os.IsNotExist(err) {
		t.Fatal("file was not created")
	}
}

// ---------------------------------------------------------------------------
// MergeSettings tests
// ---------------------------------------------------------------------------

func TestMergeSettings_BothNil(t *testing.T) {
	result := MergeSettings(nil, nil)
	if result == nil {
		t.Fatal("MergeSettings(nil, nil) returned nil")
	}
	if result.DefaultProvider != "" {
		t.Errorf("expected empty settings, got DefaultProvider=%q", result.DefaultProvider)
	}
}

func TestMergeSettings_BaseOnly(t *testing.T) {
	base := &Settings{DefaultModel: "gpt-4o", MaxTurns: 10}
	result := MergeSettings(base, nil)
	if result.DefaultModel != "gpt-4o" {
		t.Errorf("DefaultModel = %q", result.DefaultModel)
	}
	if result.MaxTurns != 10 {
		t.Errorf("MaxTurns = %d", result.MaxTurns)
	}
}

func TestMergeSettings_OverrideOnly(t *testing.T) {
	override := &Settings{Theme: "dark", MaxTurns: 20}
	result := MergeSettings(nil, override)
	if result.Theme != "dark" {
		t.Errorf("Theme = %q", result.Theme)
	}
	if result.MaxTurns != 20 {
		t.Errorf("MaxTurns = %d", result.MaxTurns)
	}
}

func TestMergeSettings_OverrideWins(t *testing.T) {
	base := &Settings{
		DefaultProvider: "openai",
		DefaultModel:    "gpt-4o",
		MaxTurns:        10,
		Theme:           "light",
	}
	override := &Settings{
		DefaultModel: "gpt-4-turbo",
		MaxTurns:     20,
	}

	result := MergeSettings(base, override)
	if result.DefaultProvider != "openai" {
		t.Errorf("DefaultProvider = %q, want 'openai' (from base)", result.DefaultProvider)
	}
	if result.DefaultModel != "gpt-4-turbo" {
		t.Errorf("DefaultModel = %q, want 'gpt-4-turbo' (from override)", result.DefaultModel)
	}
	if result.MaxTurns != 20 {
		t.Errorf("MaxTurns = %d, want 20 (from override)", result.MaxTurns)
	}
	if result.Theme != "light" {
		t.Errorf("Theme = %q, want 'light' (from base)", result.Theme)
	}
}

func TestMergeSettings_BoolOverride(t *testing.T) {
	tru := true
	fals := false

	// Base sets compaction to true, override sets to false
	base := &Settings{CompactionEnabled: &tru}
	override := &Settings{CompactionEnabled: &fals}

	result := MergeSettings(base, override)
	if result.CompactionEnabled == nil || *result.CompactionEnabled != false {
		t.Errorf("CompactionEnabled = %v, want false (override should win)", result.CompactionEnabled)
	}

	// Base false, override true
	base = &Settings{CompactionEnabled: &fals}
	override = &Settings{CompactionEnabled: &tru}

	result = MergeSettings(base, override)
	if result.CompactionEnabled == nil || *result.CompactionEnabled != true {
		t.Errorf("CompactionEnabled = %v, want true", result.CompactionEnabled)
	}
}

func TestMergeSettings_BoolOnlyInBase(t *testing.T) {
	tru := true
	base := &Settings{CompactionEnabled: &tru, EnableSkillCommands: &tru}
	override := &Settings{DefaultModel: "claude"} // no bool fields

	result := MergeSettings(base, override)
	if result.CompactionEnabled == nil || *result.CompactionEnabled != true {
		t.Errorf("CompactionEnabled = %v, want true (from base)", result.CompactionEnabled)
	}
	if result.EnableSkillCommands == nil || *result.EnableSkillCommands != true {
		t.Errorf("EnableSkillCommands = %v, want true (from base)", result.EnableSkillCommands)
	}
	if result.DefaultModel != "claude" {
		t.Errorf("DefaultModel = %q, want 'claude'", result.DefaultModel)
	}
}

func TestMergeSettings_ZeroIntNotOverride(t *testing.T) {
	base := &Settings{MaxTurns: 50, CompactionReserveTokens: 100000}
	override := &Settings{MaxTurns: 0} // zero should not override

	result := MergeSettings(base, override)
	if result.MaxTurns != 50 {
		t.Errorf("MaxTurns = %d, want 50 (zero should not override)", result.MaxTurns)
	}
	if result.CompactionReserveTokens != 100000 {
		t.Errorf("CompactionReserveTokens = %d, want 100000", result.CompactionReserveTokens)
	}
}

func TestMergeSettings_SlicesOverride(t *testing.T) {
	base := &Settings{
		Extensions: []string{"a", "b", "c"},
		Skills:     []string{"skill-a"},
	}
	override := &Settings{
		Extensions: []string{"x", "y"},
		// Skills not set — should keep base
	}

	result := MergeSettings(base, override)
	if len(result.Extensions) != 2 || result.Extensions[0] != "x" {
		t.Errorf("Extensions = %v, want [x y] (override replaces)", result.Extensions)
	}
	if len(result.Skills) != 1 || result.Skills[0] != "skill-a" {
		t.Errorf("Skills = %v, want [skill-a] (from base)", result.Skills)
	}
}

func TestMergeSettings_DoesNotMutateInputs(t *testing.T) {
	base := &Settings{DefaultModel: "gpt-4o"}
	override := &Settings{MaxTurns: 30}

	result := MergeSettings(base, override)

	// Modify result
	result.DefaultModel = "modified"

	if base.DefaultModel != "gpt-4o" {
		t.Errorf("base was mutated: DefaultModel = %q", base.DefaultModel)
	}
}

func TestMergeSettings_EmptyStringNotOverride(t *testing.T) {
	base := &Settings{ShellPath: "/bin/bash"}
	override := &Settings{ShellPath: ""}

	result := MergeSettings(base, override)
	if result.ShellPath != "/bin/bash" {
		t.Errorf("ShellPath = %q, want '/bin/bash' (empty string should not override)", result.ShellPath)
	}
}

// ---------------------------------------------------------------------------
// GetDefaultPaths tests
// ---------------------------------------------------------------------------

func TestGetDefaultPaths_ReturnsPaths(t *testing.T) {
	global, project := GetDefaultPaths()

	if !strings.Contains(global, ".xihu") {
		t.Errorf("global path should contain .xihu: %s", global)
	}
	if !strings.HasSuffix(global, "settings.json") {
		t.Errorf("global path should end with settings.json: %s", global)
	}

	if !strings.Contains(project, ".xihu") {
		t.Errorf("project path should contain .xihu: %s", project)
	}
	if !strings.HasSuffix(project, "settings.json") {
		t.Errorf("project path should end with settings.json: %s", project)
	}

	// Global and project should be different paths
	if global == project {
		t.Error("global and project paths should differ")
	}
}

// ---------------------------------------------------------------------------
// LoadAll tests
// ---------------------------------------------------------------------------

func TestLoadAll_NoFiles(t *testing.T) {
	// LoadAll uses real paths (~/.xihu/settings.json and .xihu/settings.json).
	// These likely don't exist in test, so we expect an empty Settings.
	s, err := LoadAll()
	if err != nil {
		t.Fatalf("LoadAll() error: %v", err)
	}
	if s == nil {
		t.Fatal("LoadAll() returned nil")
	}
	// Should be an empty but valid settings struct
	if s.DefaultProvider != "" {
		t.Logf("Note: global settings file exists with DefaultProvider=%q", s.DefaultProvider)
	}
}

// ---------------------------------------------------------------------------
// clone / internal helpers tests
// ---------------------------------------------------------------------------

func TestClone_DeepCopy(t *testing.T) {
	tru := true
	original := &Settings{
		DefaultModel:      "gpt-4o",
		CompactionEnabled: &tru,
		Extensions:        []string{"a", "b"},
		Skills:            []string{"s1"},
	}

	clone := original.clone()

	// Modify clone
	clone.DefaultModel = "changed"
	*clone.CompactionEnabled = false
	clone.Extensions[0] = "modified"

	if original.DefaultModel != "gpt-4o" {
		t.Errorf("original.DefaultModel mutated: %q", original.DefaultModel)
	}
	if *original.CompactionEnabled != true {
		t.Errorf("original.CompactionEnabled mutated: %v", *original.CompactionEnabled)
	}
	if original.Extensions[0] != "a" {
		t.Errorf("original.Extensions mutated: %v", original.Extensions)
	}
}

func TestClone_NilSlices(t *testing.T) {
	original := &Settings{} // nil slices
	clone := original.clone()
	if clone.Extensions != nil {
		t.Errorf("Extensions should be nil, got %v", clone.Extensions)
	}
	if clone.Skills != nil {
		t.Errorf("Skills should be nil, got %v", clone.Skills)
	}
	if clone.Prompts != nil {
		t.Errorf("Prompts should be nil, got %v", clone.Prompts)
	}
}

// ---------------------------------------------------------------------------
// JSON omitempty behavior tests
// ---------------------------------------------------------------------------

func TestSettings_JSONOmitEmpty(t *testing.T) {
	// Empty settings should serialize to "{}" (with newline from SaveSettings)
	s := &Settings{}
	dir := t.TempDir()
	path := filepath.Join(dir, "empty.json")

	if err := SaveSettings(path, s); err != nil {
		t.Fatal(err)
	}

	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}

	trimmed := strings.TrimSpace(string(data))
	if trimmed != "{}" {
		t.Errorf("empty Settings should serialize to {}, got: %s", trimmed)
	}
}

func TestSettings_JSONBoolFalse(t *testing.T) {
	// Explicit false should be serialized (not omitted)
	fals := false
	s := &Settings{CompactionEnabled: &fals}
	dir := t.TempDir()
	path := filepath.Join(dir, "bool_false.json")

	if err := SaveSettings(path, s); err != nil {
		t.Fatal(err)
	}

	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}

	if !strings.Contains(string(data), "compaction_enabled") {
		t.Errorf("explicit false should be serialized, got: %s", string(data))
	}
	if !strings.Contains(string(data), "false") {
		t.Errorf("expected 'false' in output, got: %s", string(data))
	}
}

// ---------------------------------------------------------------------------
// Nested struct round-trip tests
// ---------------------------------------------------------------------------

func TestSettings_NestedStructs_RoundTrip(t *testing.T) {
	// Verify all nested config structs survive a save/load round-trip.
	dir := t.TempDir()
	path := filepath.Join(dir, "nested.json")

	tru := true
	fals := false
	original := &Settings{
		ThinkingLevel: "high",
		ThinkingBudgets: &ThinkingBudgetsSettings{
			Minimal: 1000,
			Low:     4000,
			Medium:  16000,
			High:    64000,
		},
		HideThinkingBlock: &tru,
		Images: &ImageSettings{
			AutoResize:  &tru,
			BlockImages: &fals,
		},
		Terminal: &TerminalSettings{
			ShowImages:      &tru,
			ImageWidthCells: 80,
			ClearOnShrink:   &fals,
		},
		Retry: &RetrySettings{
			Enabled:         &tru,
			MaxRetries:      3,
			BaseDelayMs:     1000,
			TimeoutMs:       30000,
			MaxRetryDelayMs: 60000,
		},
		BranchSummary: &BranchSummarySettings{
			ReserveTokens: 2000,
			SkipPrompt:    &tru,
		},
		QuietStartup: &fals,
		NpmCommand:      []string{"pnpm"},
		DoubleEscapeAction: "clear",
		TreeFilterMode:      "files",
		EnabledModels:       []string{"gpt-4o", "claude-sonnet-4-20250514"},
		Transport:           "sse",
		SteeringMode:        "one-at-a-time",
		FollowUpMode:        "all",
		ScopedModels:        []string{"openai/*", "anthropic/*"},
		Markdown: &MarkdownSettings{
			CodeBlockIndent: "4",
		},
		Warnings: &WarningSettings{
			AnthropicExtraUsage: &tru,
		},
		SessionDir: "/tmp/pi-sessions",
	}

	if err := SaveSettings(path, original); err != nil {
		t.Fatalf("SaveSettings() error: %v", err)
	}

	loaded, err := LoadSettings(path)
	if err != nil {
		t.Fatalf("LoadSettings() error: %v", err)
	}

	// Verify nested structs round-tripped
	if loaded.ThinkingLevel != "high" {
		t.Errorf("ThinkingLevel = %q, want high", loaded.ThinkingLevel)
	}
	if loaded.ThinkingBudgets == nil {
		t.Fatal("ThinkingBudgets is nil")
	}
	if loaded.ThinkingBudgets.Minimal != 1000 {
		t.Errorf("ThinkingBudgets.Minimal = %d", loaded.ThinkingBudgets.Minimal)
	}
	if loaded.ThinkingBudgets.Low != 4000 {
		t.Errorf("ThinkingBudgets.Low = %d", loaded.ThinkingBudgets.Low)
	}
	if loaded.ThinkingBudgets.Medium != 16000 {
		t.Errorf("ThinkingBudgets.Medium = %d", loaded.ThinkingBudgets.Medium)
	}
	if loaded.ThinkingBudgets.High != 64000 {
		t.Errorf("ThinkingBudgets.High = %d", loaded.ThinkingBudgets.High)
	}
	if loaded.HideThinkingBlock == nil || *loaded.HideThinkingBlock != true {
		t.Errorf("HideThinkingBlock = %v, want true", loaded.HideThinkingBlock)
	}

	if loaded.Images == nil {
		t.Fatal("Images is nil")
	}
	if loaded.Images.AutoResize == nil || *loaded.Images.AutoResize != true {
		t.Errorf("Images.AutoResize = %v, want true", loaded.Images.AutoResize)
	}
	if loaded.Images.BlockImages == nil || *loaded.Images.BlockImages != false {
		t.Errorf("Images.BlockImages = %v, want false", loaded.Images.BlockImages)
	}

	if loaded.Terminal == nil {
		t.Fatal("Terminal is nil")
	}
	if loaded.Terminal.ShowImages == nil || *loaded.Terminal.ShowImages != true {
		t.Errorf("Terminal.ShowImages = %v, want true", loaded.Terminal.ShowImages)
	}
	if loaded.Terminal.ImageWidthCells != 80 {
		t.Errorf("Terminal.ImageWidthCells = %d", loaded.Terminal.ImageWidthCells)
	}
	if loaded.Terminal.ClearOnShrink == nil || *loaded.Terminal.ClearOnShrink != false {
		t.Errorf("Terminal.ClearOnShrink = %v, want false", loaded.Terminal.ClearOnShrink)
	}

	if loaded.Retry == nil {
		t.Fatal("Retry is nil")
	}
	if loaded.Retry.Enabled == nil || *loaded.Retry.Enabled != true {
		t.Errorf("Retry.Enabled = %v, want true", loaded.Retry.Enabled)
	}
	if loaded.Retry.MaxRetries != 3 {
		t.Errorf("Retry.MaxRetries = %d", loaded.Retry.MaxRetries)
	}
	if loaded.Retry.BaseDelayMs != 1000 {
		t.Errorf("Retry.BaseDelayMs = %d", loaded.Retry.BaseDelayMs)
	}
	if loaded.Retry.TimeoutMs != 30000 {
		t.Errorf("Retry.TimeoutMs = %d", loaded.Retry.TimeoutMs)
	}
	if loaded.Retry.MaxRetryDelayMs != 60000 {
		t.Errorf("Retry.MaxRetryDelayMs = %d", loaded.Retry.MaxRetryDelayMs)
	}

	if loaded.BranchSummary == nil {
		t.Fatal("BranchSummary is nil")
	}
	if loaded.BranchSummary.ReserveTokens != 2000 {
		t.Errorf("BranchSummary.ReserveTokens = %d", loaded.BranchSummary.ReserveTokens)
	}
	if loaded.BranchSummary.SkipPrompt == nil || *loaded.BranchSummary.SkipPrompt != true {
		t.Errorf("BranchSummary.SkipPrompt = %v, want true", loaded.BranchSummary.SkipPrompt)
	}

	if loaded.QuietStartup == nil || *loaded.QuietStartup != false {
		t.Errorf("QuietStartup = %v, want false", loaded.QuietStartup)
	}
	if len(loaded.NpmCommand) != 1 || loaded.NpmCommand[0] != "pnpm" {
		t.Errorf("NpmCommand = %q", loaded.NpmCommand)
	}
	if loaded.EditorPaddingX != 0 {
		t.Errorf("EditorPaddingX = %d, want 0 (not set in fixture)", loaded.EditorPaddingX)
	}
	if loaded.DoubleEscapeAction != "clear" {
		t.Errorf("DoubleEscapeAction = %q", loaded.DoubleEscapeAction)
	}
	if loaded.TreeFilterMode != "files" {
		t.Errorf("TreeFilterMode = %q", loaded.TreeFilterMode)
	}
	if len(loaded.EnabledModels) != 2 || loaded.EnabledModels[0] != "gpt-4o" {
		t.Errorf("EnabledModels = %v", loaded.EnabledModels)
	}
	if loaded.Transport != "sse" {
		t.Errorf("Transport = %q", loaded.Transport)
	}
	if loaded.SteeringMode != "one-at-a-time" {
		t.Errorf("SteeringMode = %q", loaded.SteeringMode)
	}
	if loaded.FollowUpMode != "all" {
		t.Errorf("FollowUpMode = %q", loaded.FollowUpMode)
	}
	if len(loaded.ScopedModels) != 2 || loaded.ScopedModels[1] != "anthropic/*" {
		t.Errorf("ScopedModels = %v", loaded.ScopedModels)
	}
	if loaded.Markdown == nil || loaded.Markdown.CodeBlockIndent != "4" {
		t.Errorf("Markdown.CodeBlockIndent = %v", loaded.Markdown)
	}
	if loaded.Warnings == nil || loaded.Warnings.AnthropicExtraUsage == nil || *loaded.Warnings.AnthropicExtraUsage != true {
		t.Errorf("Warnings.AnthropicExtraUsage = %v", loaded.Warnings)
	}
	if loaded.SessionDir != "/tmp/pi-sessions" {
		t.Errorf("SessionDir = %q", loaded.SessionDir)
	}
}

// ---------------------------------------------------------------------------
// Nested struct merge tests
// ---------------------------------------------------------------------------

func TestMergeSettings_NestedStructs_Merge(t *testing.T) {
	tru := true
	fals := false

	base := &Settings{
		Images: &ImageSettings{
			AutoResize:  &tru,
			BlockImages: &fals,
		},
		Retry: &RetrySettings{
			MaxRetries:  3,
			BaseDelayMs: 500,
		},
		Terminal: &TerminalSettings{
			ImageWidthCells: 80,
		},
	}

	override := &Settings{
		Images: &ImageSettings{
			BlockImages: &tru, // override BlockImages, AutoResize should keep base
		},
		Retry: &RetrySettings{
			MaxRetries: 5, // override MaxRetries, BaseDelayMs should keep base
		},
	}

	result := MergeSettings(base, override)

	// Images: AutoResize from base, BlockImages from override
	if result.Images == nil {
		t.Fatal("Images is nil")
	}
	if result.Images.AutoResize == nil || *result.Images.AutoResize != true {
		t.Errorf("Images.AutoResize = %v, want true (from base)", result.Images.AutoResize)
	}
	if result.Images.BlockImages == nil || *result.Images.BlockImages != true {
		t.Errorf("Images.BlockImages = %v, want true (from override)", result.Images.BlockImages)
	}

	// Retry: MaxRetries from override, BaseDelayMs from base
	if result.Retry == nil {
		t.Fatal("Retry is nil")
	}
	if result.Retry.MaxRetries != 5 {
		t.Errorf("Retry.MaxRetries = %d, want 5 (from override)", result.Retry.MaxRetries)
	}
	if result.Retry.BaseDelayMs != 500 {
		t.Errorf("Retry.BaseDelayMs = %d, want 500 (from base)", result.Retry.BaseDelayMs)
	}

	// Terminal: ImageWidthCells from base
	if result.Terminal == nil {
		t.Fatal("Terminal is nil")
	}
	if result.Terminal.ImageWidthCells != 80 {
		t.Errorf("Terminal.ImageWidthCells = %d, want 80 (from base)", result.Terminal.ImageWidthCells)
	}
}

func TestMergeSettings_NestedStructs_NilOverride(t *testing.T) {
	// When override nested struct is nil, base should survive intact.
	tru := true
	base := &Settings{
		Images: &ImageSettings{AutoResize: &tru},
		Retry:  &RetrySettings{MaxRetries: 3},
	}
	override := &Settings{
		Theme: "dark",
	}

	result := MergeSettings(base, override)

	if result.Images == nil || result.Images.AutoResize == nil || *result.Images.AutoResize != true {
		t.Errorf("Images.AutoResize should survive when override Images is nil")
	}
	if result.Retry == nil || result.Retry.MaxRetries != 3 {
		t.Errorf("Retry.MaxRetries should survive when override Retry is nil")
	}
	if result.Theme != "dark" {
		t.Errorf("Theme = %q, want dark", result.Theme)
	}
}

func TestMergeSettings_NestedStructs_NilBase(t *testing.T) {
	// When base nested struct is nil, override should be used.
	fals := false
	override := &Settings{
		Images: &ImageSettings{BlockImages: &fals},
	}
	base := &Settings{}

	result := MergeSettings(base, override)

	if result.Images == nil {
		t.Fatal("Images is nil, expected override to be used")
	}
	if result.Images.BlockImages == nil || *result.Images.BlockImages != false {
		t.Errorf("Images.BlockImages = %v, want false", result.Images.BlockImages)
	}
}

func TestMergeSettings_NewSlices(t *testing.T) {
	base := &Settings{
		EnabledModels: []string{"a", "b"},
		ScopedModels:  []string{"x/*"},
	}
	override := &Settings{
		EnabledModels: []string{"c"},
	}

	result := MergeSettings(base, override)

	if len(result.EnabledModels) != 1 || result.EnabledModels[0] != "c" {
		t.Errorf("EnabledModels = %v, want [c] (override replaces)", result.EnabledModels)
	}
	if len(result.ScopedModels) != 1 || result.ScopedModels[0] != "x/*" {
		t.Errorf("ScopedModels = %v, want [x/*] (from base)", result.ScopedModels)
	}
}

func TestMergeSettings_NewStringFields(t *testing.T) {
	base := &Settings{
		DoubleEscapeAction: "quit",
		Transport:          "sse",
		SteeringMode:       "all",
	}
	override := &Settings{
		DoubleEscapeAction: "clear",
		FollowUpMode:       "one-at-a-time",
	}

	result := MergeSettings(base, override)

	if result.DoubleEscapeAction != "clear" {
		t.Errorf("DoubleEscapeAction = %q, want clear", result.DoubleEscapeAction)
	}
	if result.Transport != "sse" {
		t.Errorf("Transport = %q, want sse (from base)", result.Transport)
	}
	if result.SteeringMode != "all" {
		t.Errorf("SteeringMode = %q, want all (from base)", result.SteeringMode)
	}
	if result.FollowUpMode != "one-at-a-time" {
		t.Errorf("FollowUpMode = %q, want one-at-a-time", result.FollowUpMode)
	}
}

// ---------------------------------------------------------------------------
// Clone tests for nested structs and new fields
// ---------------------------------------------------------------------------

func TestClone_NestedStructs(t *testing.T) {
	tru := true
	original := &Settings{
		Images: &ImageSettings{
			AutoResize:  &tru,
			BlockImages: &tru,
		},
		Retry: &RetrySettings{
			MaxRetries:  5,
			BaseDelayMs: 1000,
		},
		Terminal: &TerminalSettings{
			ImageWidthCells: 120,
		},
		BranchSummary: &BranchSummarySettings{
			ReserveTokens: 500,
		},
		ThinkingBudgets: &ThinkingBudgetsSettings{
			Low: 2000,
		},
		Markdown: &MarkdownSettings{
			CodeBlockIndent: "2",
		},
		Warnings: &WarningSettings{
			AnthropicExtraUsage: &tru,
		},
		EnabledModels: []string{"gpt-4o"},
		ScopedModels:  []string{"openai/*"},
	}

	clone := original.clone()

	// Mutate clone
	*clone.Images.AutoResize = false
	clone.Retry.MaxRetries = 99
	clone.Terminal.ImageWidthCells = 0
	clone.ThinkingBudgets.Low = 0
	clone.Markdown.CodeBlockIndent = "8"
	*clone.Warnings.AnthropicExtraUsage = false
	clone.EnabledModels[0] = "modified"
	clone.ScopedModels[0] = "modified/*"

	// Verify original untouched
	if original.Images.AutoResize == nil || *original.Images.AutoResize != true {
		t.Errorf("Images.AutoResize mutated: %v", original.Images.AutoResize)
	}
	if original.Retry.MaxRetries != 5 {
		t.Errorf("Retry.MaxRetries mutated: %d", original.Retry.MaxRetries)
	}
	if original.Terminal.ImageWidthCells != 120 {
		t.Errorf("Terminal.ImageWidthCells mutated: %d", original.Terminal.ImageWidthCells)
	}
	if original.ThinkingBudgets.Low != 2000 {
		t.Errorf("ThinkingBudgets.Low mutated: %d", original.ThinkingBudgets.Low)
	}
	if original.Markdown.CodeBlockIndent != "2" {
		t.Errorf("Markdown.CodeBlockIndent mutated: %s", original.Markdown.CodeBlockIndent)
	}
	if original.Warnings.AnthropicExtraUsage == nil || *original.Warnings.AnthropicExtraUsage != true {
		t.Errorf("Warnings.AnthropicExtraUsage mutated: %v", original.Warnings.AnthropicExtraUsage)
	}
	if original.EnabledModels[0] != "gpt-4o" {
		t.Errorf("EnabledModels mutated: %v", original.EnabledModels)
	}
	if original.ScopedModels[0] != "openai/*" {
		t.Errorf("ScopedModels mutated: %v", original.ScopedModels)
	}
}

func TestClone_NilNestedStructs(t *testing.T) {
	original := &Settings{} // all nested structs nil
	clone := original.clone()

	if clone.Images != nil {
		t.Error("Images should be nil")
	}
	if clone.Terminal != nil {
		t.Error("Terminal should be nil")
	}
	if clone.Retry != nil {
		t.Error("Retry should be nil")
	}
	if clone.BranchSummary != nil {
		t.Error("BranchSummary should be nil")
	}
	if clone.ThinkingBudgets != nil {
		t.Error("ThinkingBudgets should be nil")
	}
	if clone.Markdown != nil {
		t.Error("Markdown should be nil")
	}
	if clone.Warnings != nil {
		t.Error("Warnings should be nil")
	}
	if clone.EnabledModels != nil {
		t.Error("EnabledModels should be nil")
	}
	if clone.ScopedModels != nil {
		t.Error("ScopedModels should be nil")
	}
}

// ---------------------------------------------------------------------------
// Edge cases
// ---------------------------------------------------------------------------

func TestMergeSettings_AllFieldsComprehensive(t *testing.T) {
	tru := true
	fals := false

	base := &Settings{
		DefaultProvider:           "openai",
		DefaultModel:              "gpt-4o",
		DefaultThinkingLevel:      "medium",
		Theme:                     "dark",
		CompactionEnabled:         &tru,
		CompactionReserveTokens:   100000,
		CompactionKeepRecentTokens: 20000,
		ShellPath:                 "/bin/bash",
		ShellCommandPrefix:        "",
		MaxTurns:                  50,
		SystemPrompt:              "base prompt",
		Extensions:                []string{"base-ext"},
		Skills:                    []string{"base-skill"},
		Prompts:                   []string{"base-prompt"},
		EnableSkillCommands:       &fals,
	}

	override := &Settings{
		DefaultModel:              "claude-sonnet-4-20250514",
		DefaultThinkingLevel:      "high",
		CompactionReserveTokens:   200000,
		ShellCommandPrefix:        "source ~/.bashrc && ",
		MaxTurns:                  100,
		SystemPrompt:              "override prompt",
		Extensions:                []string{"ov-ext1", "ov-ext2"},
		EnableSkillCommands:       &tru,
		// Theme not set — should keep base
		// CompactionEnabled not set — should keep base
		// ShellPath not set — should keep base
		// Skills not set — should keep base
		// Prompts not set — should keep base
	}

	result := MergeSettings(base, override)

	// From base (not overridden)
	if result.DefaultProvider != "openai" {
		t.Errorf("DefaultProvider = %q, want openai", result.DefaultProvider)
	}
	if result.Theme != "dark" {
		t.Errorf("Theme = %q, want dark", result.Theme)
	}
	if result.CompactionEnabled == nil || *result.CompactionEnabled != true {
		t.Errorf("CompactionEnabled = %v, want true", result.CompactionEnabled)
	}
	if result.ShellPath != "/bin/bash" {
		t.Errorf("ShellPath = %q, want /bin/bash", result.ShellPath)
	}
	if len(result.Skills) != 1 || result.Skills[0] != "base-skill" {
		t.Errorf("Skills = %v, want [base-skill]", result.Skills)
	}
	if len(result.Prompts) != 1 || result.Prompts[0] != "base-prompt" {
		t.Errorf("Prompts = %v, want [base-prompt]", result.Prompts)
	}

	// From override
	if result.DefaultModel != "claude-sonnet-4-20250514" {
		t.Errorf("DefaultModel = %q", result.DefaultModel)
	}
	if result.DefaultThinkingLevel != "high" {
		t.Errorf("DefaultThinkingLevel = %q", result.DefaultThinkingLevel)
	}
	if result.CompactionReserveTokens != 200000 {
		t.Errorf("CompactionReserveTokens = %d", result.CompactionReserveTokens)
	}
	if result.ShellCommandPrefix != "source ~/.bashrc && " {
		t.Errorf("ShellCommandPrefix = %q", result.ShellCommandPrefix)
	}
	if result.MaxTurns != 100 {
		t.Errorf("MaxTurns = %d", result.MaxTurns)
	}
	if result.SystemPrompt != "override prompt" {
		t.Errorf("SystemPrompt = %q", result.SystemPrompt)
	}
	if len(result.Extensions) != 2 || result.Extensions[0] != "ov-ext1" {
		t.Errorf("Extensions = %v, want [ov-ext1 ov-ext2]", result.Extensions)
	}
	if result.EnableSkillCommands == nil || *result.EnableSkillCommands != true {
		t.Errorf("EnableSkillCommands = %v, want true", result.EnableSkillCommands)
	}
}

// ---------------------------------------------------------------------------
// LockSettings / UnlockSettings / IsLocked tests
// ---------------------------------------------------------------------------

func TestLockSettings_AcquireAndRelease(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "settings.lock_test.json")

	// Not locked initially
	if IsLocked(path) {
		t.Fatal("expected not locked initially")
	}

	// Acquire lock
	if err := LockSettings(path); err != nil {
		t.Fatalf("LockSettings() error: %v", err)
	}
	if !IsLocked(path) {
		t.Fatal("expected locked after LockSettings")
	}

	// Double lock should fail
	if err := LockSettings(path); err == nil {
		t.Fatal("expected error on double lock")
	}

	// Unlock
	if err := UnlockSettings(path); err != nil {
		t.Fatalf("UnlockSettings() error: %v", err)
	}
	if IsLocked(path) {
		t.Fatal("expected not locked after UnlockSettings")
	}
}

func TestLockSettings_UnlockNonExistent(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "nonexistent.json")

	// Unlock on non-existent should not error
	if err := UnlockSettings(path); err != nil {
		t.Fatalf("UnlockSettings() on non-existent should not error: %v", err)
	}
}

func TestIsLocked_NonExistent(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "nonexistent.json")
	if IsLocked(path) {
		t.Fatal("IsLocked should return false for non-existent lock")
	}
}

// ---------------------------------------------------------------------------
// MigrateSettings tests
// ---------------------------------------------------------------------------

func TestMigrateSettings_ValidLevels(t *testing.T) {
	// All valid thinking levels should pass through unchanged
	validLevels := []string{"off", "minimal", "low", "medium", "high", "xhigh", "max", ""}
	for _, level := range validLevels {
		s := &Settings{
			DefaultThinkingLevel: level,
			ThinkingLevel:        level,
		}
		MigrateSettings(s)
		if s.DefaultThinkingLevel != level {
			t.Errorf("DefaultThinkingLevel changed from %q to %q", level, s.DefaultThinkingLevel)
		}
		if s.ThinkingLevel != level {
			t.Errorf("ThinkingLevel changed from %q to %q", level, s.ThinkingLevel)
		}
	}
}

func TestMigrateSettings_InvalidThinkingLevel(t *testing.T) {
	s := &Settings{
		DefaultThinkingLevel: "invalid_level",
		ThinkingLevel:        "also_invalid",
	}
	MigrateSettings(s)
	if s.DefaultThinkingLevel != "" {
		t.Errorf("DefaultThinkingLevel should be reset, got %q", s.DefaultThinkingLevel)
	}
	if s.ThinkingLevel != "" {
		t.Errorf("ThinkingLevel should be reset, got %q", s.ThinkingLevel)
	}
}

func TestMigrateSettings_ValidDoubleEscapeAction(t *testing.T) {
	validActions := []string{"fork", "tree", "none", ""}
	for _, action := range validActions {
		s := &Settings{DoubleEscapeAction: action}
		MigrateSettings(s)
		if s.DoubleEscapeAction != action {
			t.Errorf("DoubleEscapeAction changed from %q to %q", action, s.DoubleEscapeAction)
		}
	}
}

func TestMigrateSettings_InvalidDoubleEscapeAction(t *testing.T) {
	s := &Settings{DoubleEscapeAction: "invalid"}
	MigrateSettings(s)
	if s.DoubleEscapeAction != "" {
		t.Errorf("DoubleEscapeAction should be reset, got %q", s.DoubleEscapeAction)
	}
}

func TestMigrateSettings_ValidTreeFilterMode(t *testing.T) {
	validModes := []string{"all", "default", "user-only", "no-tools", "labeled-only", ""}
	for _, mode := range validModes {
		s := &Settings{TreeFilterMode: mode}
		MigrateSettings(s)
		if s.TreeFilterMode != mode {
			t.Errorf("TreeFilterMode changed from %q to %q", mode, s.TreeFilterMode)
		}
	}
}

func TestMigrateSettings_InvalidTreeFilterMode(t *testing.T) {
	s := &Settings{TreeFilterMode: "bogus"}
	MigrateSettings(s)
	if s.TreeFilterMode != "" {
		t.Errorf("TreeFilterMode should be reset, got %q", s.TreeFilterMode)
	}
}

func TestMigrateSettings_ValidSteeringAndFollowUp(t *testing.T) {
	validModes := []string{"all", "one-at-a-time", ""}
	for _, mode := range validModes {
		s := &Settings{SteeringMode: mode, FollowUpMode: mode}
		MigrateSettings(s)
		if s.SteeringMode != mode {
			t.Errorf("SteeringMode changed from %q to %q", mode, s.SteeringMode)
		}
		if s.FollowUpMode != mode {
			t.Errorf("FollowUpMode changed from %q to %q", mode, s.FollowUpMode)
		}
	}
}

func TestMigrateSettings_InvalidSteeringAndFollowUp(t *testing.T) {
	s := &Settings{SteeringMode: "broken", FollowUpMode: "also_broken"}
	MigrateSettings(s)
	if s.SteeringMode != "" {
		t.Errorf("SteeringMode should be reset, got %q", s.SteeringMode)
	}
	if s.FollowUpMode != "" {
		t.Errorf("FollowUpMode should be reset, got %q", s.FollowUpMode)
	}
}

// ---------------------------------------------------------------------------
// Reload tests
// ---------------------------------------------------------------------------

func TestReload_Success(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "reload.json")

	tru := true
	original := &Settings{
		DefaultModel:      "gpt-4o",
		MaxTurns:          50,
		CompactionEnabled: &tru,
	}
	if err := SaveSettings(path, original); err != nil {
		t.Fatal(err)
	}

	// Start with a different settings struct
	s := &Settings{DefaultModel: "old-model", MaxTurns: 10}
	if err := s.Reload(path); err != nil {
		t.Fatalf("Reload() error: %v", err)
	}

	if s.DefaultModel != "gpt-4o" {
		t.Errorf("DefaultModel = %q, want gpt-4o", s.DefaultModel)
	}
	if s.MaxTurns != 50 {
		t.Errorf("MaxTurns = %d, want 50", s.MaxTurns)
	}
	if s.CompactionEnabled == nil || *s.CompactionEnabled != true {
		t.Errorf("CompactionEnabled = %v, want true", s.CompactionEnabled)
	}
}

func TestReload_EmptyPath(t *testing.T) {
	s := &Settings{}
	if err := s.Reload(""); err == nil {
		t.Fatal("expected error for empty path")
	}
}

func TestReload_NonExistentFile(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "nonexistent.json")
	s := &Settings{DefaultModel: "keep-me"}
	if err := s.Reload(path); err != nil {
		t.Fatalf("Reload() on non-existent file should not error: %v", err)
	}
	// After loading non-existent, defaults to empty settings
	if s.DefaultModel != "" {
		t.Errorf("DefaultModel = %q, expected empty after reloading non-existent", s.DefaultModel)
	}
}

// ---------------------------------------------------------------------------
// ApplyOverrides tests
// ---------------------------------------------------------------------------

func TestApplyOverrides_NilOverrides(t *testing.T) {
	base := &Settings{DefaultModel: "gpt-4o", MaxTurns: 50}
	base.ApplyOverrides(nil)
	if base.DefaultModel != "gpt-4o" {
		t.Errorf("DefaultModel changed: %q", base.DefaultModel)
	}
	if base.MaxTurns != 50 {
		t.Errorf("MaxTurns changed: %d", base.MaxTurns)
	}
}

func TestApplyOverrides_StringFields(t *testing.T) {
	base := &Settings{
		DefaultProvider:  "openai",
		DefaultModel:     "gpt-4o",
		Theme:            "dark",
		ShellPath:        "/bin/bash",
		SystemPrompt:     "base prompt",
		SessionDir:       "/tmp/base",
		DoubleEscapeAction: "fork",
		Transport:        "sse",
		SteeringMode:     "all",
	}
	overrides := &Settings{
		DefaultModel:      "claude-sonnet",
		Theme:             "light",
		ShellCommandPrefix: "source ~/.zshrc && ",
		SessionDir:        "/tmp/override",
		FollowUpMode:      "one-at-a-time",
	}
	base.ApplyOverrides(overrides)

	if base.DefaultProvider != "openai" {
		t.Errorf("DefaultProvider should be unchanged: %q", base.DefaultProvider)
	}
	if base.DefaultModel != "claude-sonnet" {
		t.Errorf("DefaultModel = %q", base.DefaultModel)
	}
	if base.Theme != "light" {
		t.Errorf("Theme = %q", base.Theme)
	}
	if base.ShellPath != "/bin/bash" {
		t.Errorf("ShellPath = %q", base.ShellPath)
	}
	if base.ShellCommandPrefix != "source ~/.zshrc && " {
		t.Errorf("ShellCommandPrefix = %q", base.ShellCommandPrefix)
	}
	if base.SessionDir != "/tmp/override" {
		t.Errorf("SessionDir = %q", base.SessionDir)
	}
	if base.DoubleEscapeAction != "fork" {
		t.Errorf("DoubleEscapeAction = %q", base.DoubleEscapeAction)
	}
	if base.Transport != "sse" {
		t.Errorf("Transport = %q", base.Transport)
	}
	if base.SteeringMode != "all" {
		t.Errorf("SteeringMode = %q", base.SteeringMode)
	}
	if base.FollowUpMode != "one-at-a-time" {
		t.Errorf("FollowUpMode = %q", base.FollowUpMode)
	}
}

func TestApplyOverrides_IntFields(t *testing.T) {
	base := &Settings{
		MaxTurns:                50,
		CompactionReserveTokens: 100000,
		EditorPaddingX:          2,
	}
	overrides := &Settings{
		MaxTurns:                100,
		CompactionKeepRecentTokens: 50000,
		AutocompleteMaxVisible:   20,
	}
	base.ApplyOverrides(overrides)

	if base.MaxTurns != 100 {
		t.Errorf("MaxTurns = %d", base.MaxTurns)
	}
	if base.CompactionReserveTokens != 100000 {
		t.Errorf("CompactionReserveTokens = %d", base.CompactionReserveTokens)
	}
	if base.CompactionKeepRecentTokens != 50000 {
		t.Errorf("CompactionKeepRecentTokens = %d", base.CompactionKeepRecentTokens)
	}
	if base.EditorPaddingX != 2 {
		t.Errorf("EditorPaddingX = %d", base.EditorPaddingX)
	}
	if base.AutocompleteMaxVisible != 20 {
		t.Errorf("AutocompleteMaxVisible = %d", base.AutocompleteMaxVisible)
	}
}

func TestApplyOverrides_BoolFields(t *testing.T) {
	tru := true
	fals := false
	base := &Settings{
		QuietStartup:        &fals,
		CollapseChangelog:   &fals,
		ShowHardwareCursor:  &fals,
	}
	overrides := &Settings{
		QuietStartup:        &tru,
		EnableInstallTelemetry: &tru,
	}
	base.ApplyOverrides(overrides)

	if base.QuietStartup == nil || *base.QuietStartup != true {
		t.Errorf("QuietStartup = %v", base.QuietStartup)
	}
	if base.CollapseChangelog == nil || *base.CollapseChangelog != false {
		t.Errorf("CollapseChangelog = %v", base.CollapseChangelog)
	}
	if base.EnableInstallTelemetry == nil || *base.EnableInstallTelemetry != true {
		t.Errorf("EnableInstallTelemetry = %v", base.EnableInstallTelemetry)
	}
}

func TestApplyOverrides_SliceFields(t *testing.T) {
	base := &Settings{
		Extensions:    []string{"a", "b"},
		Skills:        []string{"s1"},
		EnabledModels: []string{"gpt-4o"},
		NpmCommand:    []string{"npm"},
		Packages:      []PackageSource{{Name: "pkg1", Version: "v1"}},
	}
	overrides := &Settings{
		Extensions:    []string{"x", "y"},
		ScopedModels:  []string{"openai/*"},
		Themes:        []string{"theme1"},
	}
	base.ApplyOverrides(overrides)

	if len(base.Extensions) != 2 || base.Extensions[0] != "x" {
		t.Errorf("Extensions = %v", base.Extensions)
	}
	if len(base.Skills) != 1 || base.Skills[0] != "s1" {
		t.Errorf("Skills = %v", base.Skills)
	}
	if len(base.ScopedModels) != 1 || base.ScopedModels[0] != "openai/*" {
		t.Errorf("ScopedModels = %v", base.ScopedModels)
	}
	if len(base.Themes) != 1 || base.Themes[0] != "theme1" {
		t.Errorf("Themes = %v", base.Themes)
	}
	if len(base.Packages) != 1 || base.Packages[0].Name != "pkg1" {
		t.Errorf("Packages = %v", base.Packages)
	}
	if len(base.EnabledModels) != 1 || base.EnabledModels[0] != "gpt-4o" {
		t.Errorf("EnabledModels = %v", base.EnabledModels)
	}
}

func TestApplyOverrides_NestedStructs(t *testing.T) {
	tru := true
	base := &Settings{
		Images:    &ImageSettings{AutoResize: &tru},
		Terminal:  &TerminalSettings{ImageWidthCells: 80},
		Retry:     &RetrySettings{MaxRetries: 3},
		Markdown:  &MarkdownSettings{CodeBlockIndent: "2"},
		Warnings:  &WarningSettings{AnthropicExtraUsage: &tru},
		BranchSummary: &BranchSummarySettings{ReserveTokens: 1000},
		ThinkingBudgets: &ThinkingBudgetsSettings{Low: 1000},
	}
	overrides := &Settings{
		Images:    &ImageSettings{BlockImages: &tru},
		Terminal:  &TerminalSettings{ShowImages: &tru},
		Retry:     &RetrySettings{BaseDelayMs: 2000},
		Markdown:  &MarkdownSettings{CodeBlockIndent: "4"},
		Warnings:  &WarningSettings{},
	}
	base.ApplyOverrides(overrides)

	// Images: overrides fully replaces (ApplyOverrides uses clone, not merge for nested structs)
	if base.Images == nil {
		t.Fatal("Images is nil")
	}
	if base.Images.AutoResize != nil {
		t.Error("Images.AutoResize should be nil (overrides replace, not merge)")
	}
	if base.Images.BlockImages == nil || *base.Images.BlockImages != true {
		t.Errorf("Images.BlockImages = %v", base.Images.BlockImages)
	}

	// Terminal: overrides replace
	if base.Terminal.ImageWidthCells != 0 {
		t.Errorf("Terminal.ImageWidthCells = %d (overrides replace, not merge)", base.Terminal.ImageWidthCells)
	}
	if base.Terminal.ShowImages == nil || *base.Terminal.ShowImages != true {
		t.Errorf("Terminal.ShowImages = %v", base.Terminal.ShowImages)
	}

	// Retry: overrides replace
	if base.Retry.MaxRetries != 0 {
		t.Errorf("Retry.MaxRetries = %d", base.Retry.MaxRetries)
	}
	if base.Retry.BaseDelayMs != 2000 {
		t.Errorf("Retry.BaseDelayMs = %d", base.Retry.BaseDelayMs)
	}

	// Markdown: overrides replace
	if base.Markdown.CodeBlockIndent != "4" {
		t.Errorf("Markdown.CodeBlockIndent = %q", base.Markdown.CodeBlockIndent)
	}

	// Warnings: overrides replace with empty struct
	if base.Warnings.AnthropicExtraUsage != nil {
		t.Error("Warnings.AnthropicExtraUsage should be nil (overrides replace)")
	}

	// BranchSummary: should survive (not overridden)
	if base.BranchSummary == nil {
		t.Fatal("BranchSummary is nil")
	}
	if base.BranchSummary.ReserveTokens != 1000 {
		t.Errorf("BranchSummary.ReserveTokens = %d", base.BranchSummary.ReserveTokens)
	}

	// ThinkingBudgets: should survive (not overridden)
	if base.ThinkingBudgets == nil {
		t.Fatal("ThinkingBudgets is nil")
	}
	if base.ThinkingBudgets.Low != 1000 {
		t.Errorf("ThinkingBudgets.Low = %d", base.ThinkingBudgets.Low)
	}
}

// ---------------------------------------------------------------------------
// Internal helper tests: boolPtr, copyStringSlice, copyPackageSources
// ---------------------------------------------------------------------------

func TestBoolPtr(t *testing.T) {
	p := boolPtr(true)
	if p == nil || *p != true {
		t.Errorf("boolPtr(true) = %v", p)
	}

	p = boolPtr(false)
	if p == nil || *p != false {
		t.Errorf("boolPtr(false) = %v", p)
	}
}

func TestCopyStringSlice_Nil(t *testing.T) {
	if copyStringSlice(nil) != nil {
		t.Error("copyStringSlice(nil) should return nil")
	}
}

func TestCopyStringSlice_NonNil(t *testing.T) {
	src := []string{"a", "b", "c"}
	dst := copyStringSlice(src)
	if len(dst) != 3 {
		t.Fatalf("len = %d, want 3", len(dst))
	}
	src[0] = "modified"
	if dst[0] != "a" {
		t.Error("copyStringSlice should make a copy, not alias")
	}
}

func TestCopyStringSlice_Empty(t *testing.T) {
	src := []string{}
	dst := copyStringSlice(src)
	if dst == nil {
		t.Error("copyStringSlice([]) should return empty slice, not nil")
	}
	if len(dst) != 0 {
		t.Errorf("len = %d, want 0", len(dst))
	}
}

func TestCopyPackageSources_Nil(t *testing.T) {
	if copyPackageSources(nil) != nil {
		t.Error("copyPackageSources(nil) should return nil")
	}
}

func TestCopyPackageSources_NonNil(t *testing.T) {
	src := []PackageSource{
		{Name: "pkg1", Version: "v1", Source: "npm"},
		{Name: "pkg2", Version: "v2", Source: "git"},
	}
	dst := copyPackageSources(src)
	if len(dst) != 2 {
		t.Fatalf("len = %d, want 2", len(dst))
	}
	if dst[0].Name != "pkg1" || dst[0].Version != "v1" || dst[0].Source != "npm" {
		t.Errorf("dst[0] = %+v", dst[0])
	}
	src[0].Name = "modified"
	if dst[0].Name != "pkg1" {
		t.Error("copyPackageSources should make a copy, not alias")
	}
}

// ---------------------------------------------------------------------------
// Clone helper tests for nested structs
// ---------------------------------------------------------------------------

func TestCloneThinkingBudgets_Nil(t *testing.T) {
	if cloneThinkingBudgets(nil) != nil {
		t.Error("cloneThinkingBudgets(nil) should return nil")
	}
}

func TestCloneThinkingBudgets_NonNil(t *testing.T) {
	src := &ThinkingBudgetsSettings{Minimal: 100, Low: 500, Medium: 2000, High: 8000}
	dst := cloneThinkingBudgets(src)
	if dst == &(*src) {
		t.Error("cloneThinkingBudgets should return a different pointer")
	}
	dst.Minimal = 999
	if src.Minimal != 100 {
		t.Errorf("src.Minimal mutated: %d", src.Minimal)
	}
}

func TestCloneImageSettings_Nil(t *testing.T) {
	if cloneImageSettings(nil) != nil {
		t.Error("cloneImageSettings(nil) should return nil")
	}
}

func TestCloneImageSettings_NonNil(t *testing.T) {
	tru := true
	fals := false
	src := &ImageSettings{AutoResize: &tru, BlockImages: &fals}
	dst := cloneImageSettings(src)
	if dst == src {
		t.Error("should be different pointer")
	}
	if dst.AutoResize == nil || *dst.AutoResize != true {
		t.Errorf("dst.AutoResize = %v", dst.AutoResize)
	}
	if dst.BlockImages == nil || *dst.BlockImages != false {
		t.Errorf("dst.BlockImages = %v", dst.BlockImages)
	}
	*dst.AutoResize = false
	if *src.AutoResize != true {
		t.Error("src.AutoResize mutated")
	}
}

func TestCloneImageSettings_NilBools(t *testing.T) {
	src := &ImageSettings{}
	dst := cloneImageSettings(src)
	if dst.AutoResize != nil {
		t.Error("dst.AutoResize should be nil")
	}
	if dst.BlockImages != nil {
		t.Error("dst.BlockImages should be nil")
	}
}

func TestCloneTerminalSettings_Nil(t *testing.T) {
	if cloneTerminalSettings(nil) != nil {
		t.Error("cloneTerminalSettings(nil) should return nil")
	}
}

func TestCloneTerminalSettings_NonNil(t *testing.T) {
	tru := true
	src := &TerminalSettings{
		ShowImages:           &tru,
		ImageWidthCells:      100,
		ClearOnShrink:        &tru,
		ShowTerminalProgress: &tru,
	}
	dst := cloneTerminalSettings(src)
	if dst == src {
		t.Error("should be different pointer")
	}
	dst.ImageWidthCells = 999
	*dst.ShowImages = false
	if src.ImageWidthCells != 100 {
		t.Errorf("src.ImageWidthCells mutated: %d", src.ImageWidthCells)
	}
	if *src.ShowImages != true {
		t.Error("src.ShowImages mutated")
	}
}

func TestCloneRetrySettings_Nil(t *testing.T) {
	if cloneRetrySettings(nil) != nil {
		t.Error("cloneRetrySettings(nil) should return nil")
	}
}

func TestCloneRetrySettings_NonNil(t *testing.T) {
	tru := true
	src := &RetrySettings{
		Enabled:    &tru,
		MaxRetries: 5,
		BaseDelayMs: 1000,
		Provider: &ProviderRetrySettings{
			TimeoutMs:       30000,
			MaxRetries:      3,
			MaxRetryDelayMs: 60000,
		},
	}
	dst := cloneRetrySettings(src)
	if dst == src {
		t.Error("should be different pointer")
	}
	if dst.Provider == src.Provider {
		t.Error("Provider should be different pointer")
	}
	dst.MaxRetries = 99
	dst.Provider.MaxRetries = 99
	if src.MaxRetries != 5 {
		t.Errorf("src.MaxRetries mutated: %d", src.MaxRetries)
	}
	if src.Provider.MaxRetries != 3 {
		t.Errorf("src.Provider.MaxRetries mutated: %d", src.Provider.MaxRetries)
	}
}

func TestCloneRetrySettings_NilProvider(t *testing.T) {
	src := &RetrySettings{MaxRetries: 3, Provider: nil}
	dst := cloneRetrySettings(src)
	if dst.Provider != nil {
		t.Error("dst.Provider should be nil")
	}
}

func TestCloneBranchSummarySettings_Nil(t *testing.T) {
	if cloneBranchSummarySettings(nil) != nil {
		t.Error("cloneBranchSummarySettings(nil) should return nil")
	}
}

func TestCloneBranchSummarySettings_NonNil(t *testing.T) {
	tru := true
	src := &BranchSummarySettings{ReserveTokens: 500, SkipPrompt: &tru}
	dst := cloneBranchSummarySettings(src)
	dst.ReserveTokens = 999
	*dst.SkipPrompt = false
	if src.ReserveTokens != 500 {
		t.Errorf("src.ReserveTokens mutated: %d", src.ReserveTokens)
	}
	if *src.SkipPrompt != true {
		t.Error("src.SkipPrompt mutated")
	}
}

func TestCloneMarkdownSettings_Nil(t *testing.T) {
	if cloneMarkdownSettings(nil) != nil {
		t.Error("cloneMarkdownSettings(nil) should return nil")
	}
}

func TestCloneMarkdownSettings_NonNil(t *testing.T) {
	src := &MarkdownSettings{CodeBlockIndent: "4"}
	dst := cloneMarkdownSettings(src)
	dst.CodeBlockIndent = "8"
	if src.CodeBlockIndent != "4" {
		t.Errorf("src.CodeBlockIndent mutated: %s", src.CodeBlockIndent)
	}
}

func TestCloneWarningSettings_Nil(t *testing.T) {
	if cloneWarningSettings(nil) != nil {
		t.Error("cloneWarningSettings(nil) should return nil")
	}
}

func TestCloneWarningSettings_NonNil(t *testing.T) {
	tru := true
	src := &WarningSettings{AnthropicExtraUsage: &tru}
	dst := cloneWarningSettings(src)
	*dst.AnthropicExtraUsage = false
	if *src.AnthropicExtraUsage != true {
		t.Error("src.AnthropicExtraUsage mutated")
	}
}

func TestCloneWarningSettings_NilBool(t *testing.T) {
	src := &WarningSettings{}
	dst := cloneWarningSettings(src)
	if dst.AnthropicExtraUsage != nil {
		t.Error("dst.AnthropicExtraUsage should be nil")
	}
}

// ---------------------------------------------------------------------------
// Merge helper tests for nested structs
// ---------------------------------------------------------------------------

func TestMergeThinkingBudgets_BothNil(t *testing.T) {
	if mergeThinkingBudgets(nil, nil) != nil {
		t.Error("mergeThinkingBudgets(nil, nil) should return nil")
	}
}

func TestMergeThinkingBudgets_BaseOnly(t *testing.T) {
	base := &ThinkingBudgetsSettings{Minimal: 100, Low: 500}
	if mergeThinkingBudgets(base, nil) != base {
		t.Error("mergeThinkingBudgets(base, nil) should return base unchanged")
	}
}

func TestMergeThinkingBudgets_OverrideOnly(t *testing.T) {
	override := &ThinkingBudgetsSettings{Low: 200, Medium: 1000}
	result := mergeThinkingBudgets(nil, override)
	if result.Low != 200 {
		t.Errorf("Low = %d", result.Low)
	}
	if result.Medium != 1000 {
		t.Errorf("Medium = %d", result.Medium)
	}
}

func TestMergeThinkingBudgets_Merge(t *testing.T) {
	base := &ThinkingBudgetsSettings{Minimal: 100, Low: 500, High: 8000}
	override := &ThinkingBudgetsSettings{Low: 1000, Medium: 4000}
	result := mergeThinkingBudgets(base, override)
	if result.Minimal != 100 {
		t.Errorf("Minimal = %d, want 100 (from base)", result.Minimal)
	}
	if result.Low != 1000 {
		t.Errorf("Low = %d, want 1000 (from override)", result.Low)
	}
	if result.Medium != 4000 {
		t.Errorf("Medium = %d, want 4000 (from override)", result.Medium)
	}
	if result.High != 8000 {
		t.Errorf("High = %d, want 8000 (from base)", result.High)
	}
}

func TestMergeImageSettings_BothNil(t *testing.T) {
	if mergeImageSettings(nil, nil) != nil {
		t.Error("mergeImageSettings(nil, nil) should return nil")
	}
}

func TestMergeImageSettings_BaseOnly(t *testing.T) {
	tru := true
	base := &ImageSettings{AutoResize: &tru}
	if mergeImageSettings(base, nil) != base {
		t.Error("mergeImageSettings(base, nil) should return base unchanged")
	}
}

func TestMergeImageSettings_OverrideOnly(t *testing.T) {
	fals := false
	override := &ImageSettings{BlockImages: &fals}
	result := mergeImageSettings(nil, override)
	if result.BlockImages == nil || *result.BlockImages != false {
		t.Errorf("BlockImages = %v", result.BlockImages)
	}
}

func TestMergeImageSettings_Merge(t *testing.T) {
	tru := true
	fals := false
	base := &ImageSettings{AutoResize: &tru, BlockImages: &fals}
	override := &ImageSettings{BlockImages: &tru}
	result := mergeImageSettings(base, override)
	if result.AutoResize == nil || *result.AutoResize != true {
		t.Errorf("AutoResize = %v", result.AutoResize)
	}
	if result.BlockImages == nil || *result.BlockImages != true {
		t.Errorf("BlockImages = %v, want true (from override)", result.BlockImages)
	}
}

func TestMergeTerminalSettings_BothNil(t *testing.T) {
	if mergeTerminalSettings(nil, nil) != nil {
		t.Error("mergeTerminalSettings(nil, nil) should return nil")
	}
}

func TestMergeTerminalSettings_BaseOnly(t *testing.T) {
	tru := true
	base := &TerminalSettings{ShowImages: &tru}
	if mergeTerminalSettings(base, nil) != base {
		t.Error("mergeTerminalSettings(base, nil) should return base unchanged")
	}
}

func TestMergeTerminalSettings_OverrideOnly(t *testing.T) {
	override := &TerminalSettings{ImageWidthCells: 120}
	result := mergeTerminalSettings(nil, override)
	if result.ImageWidthCells != 120 {
		t.Errorf("ImageWidthCells = %d", result.ImageWidthCells)
	}
}

func TestMergeTerminalSettings_Merge(t *testing.T) {
	tru := true
	fals := false
	base := &TerminalSettings{ShowImages: &tru, ImageWidthCells: 80, ClearOnShrink: &fals}
	override := &TerminalSettings{ImageWidthCells: 120, ShowTerminalProgress: &tru}
	result := mergeTerminalSettings(base, override)
	if result.ShowImages == nil || *result.ShowImages != true {
		t.Errorf("ShowImages = %v", result.ShowImages)
	}
	if result.ImageWidthCells != 120 {
		t.Errorf("ImageWidthCells = %d", result.ImageWidthCells)
	}
	if result.ClearOnShrink == nil || *result.ClearOnShrink != false {
		t.Errorf("ClearOnShrink = %v", result.ClearOnShrink)
	}
	if result.ShowTerminalProgress == nil || *result.ShowTerminalProgress != true {
		t.Errorf("ShowTerminalProgress = %v", result.ShowTerminalProgress)
	}
}

func TestMergeProviderRetrySettings_BothNil(t *testing.T) {
	if mergeProviderRetrySettings(nil, nil) != nil {
		t.Error("mergeProviderRetrySettings(nil, nil) should return nil")
	}
}

func TestMergeProviderRetrySettings_BaseOnly(t *testing.T) {
	base := &ProviderRetrySettings{TimeoutMs: 30000}
	if mergeProviderRetrySettings(base, nil) != base {
		t.Error("mergeProviderRetrySettings(base, nil) should return base unchanged")
	}
}

func TestMergeProviderRetrySettings_OverrideOnly(t *testing.T) {
	override := &ProviderRetrySettings{MaxRetries: 5, TimeoutMs: 15000}
	result := mergeProviderRetrySettings(nil, override)
	if result.MaxRetries != 5 {
		t.Errorf("MaxRetries = %d", result.MaxRetries)
	}
	if result.TimeoutMs != 15000 {
		t.Errorf("TimeoutMs = %d", result.TimeoutMs)
	}
	if result == override {
		t.Error("result should be a copy, not the same pointer")
	}
}

func TestMergeProviderRetrySettings_Merge(t *testing.T) {
	base := &ProviderRetrySettings{TimeoutMs: 30000, MaxRetries: 3, MaxRetryDelayMs: 60000}
	override := &ProviderRetrySettings{MaxRetries: 5, TimeoutMs: 0}
	result := mergeProviderRetrySettings(base, override)
	if result.TimeoutMs != 30000 {
		t.Errorf("TimeoutMs = %d, want 30000 (from base)", result.TimeoutMs)
	}
	if result.MaxRetries != 5 {
		t.Errorf("MaxRetries = %d, want 5 (from override)", result.MaxRetries)
	}
	if result.MaxRetryDelayMs != 60000 {
		t.Errorf("MaxRetryDelayMs = %d, want 60000 (from base)", result.MaxRetryDelayMs)
	}
}

func TestMergeRetrySettings_BothNil(t *testing.T) {
	if mergeRetrySettings(nil, nil) != nil {
		t.Error("mergeRetrySettings(nil, nil) should return nil")
	}
}

func TestMergeRetrySettings_BaseOnly(t *testing.T) {
	tru := true
	base := &RetrySettings{Enabled: &tru, MaxRetries: 3}
	if mergeRetrySettings(base, nil) != base {
		t.Error("mergeRetrySettings(base, nil) should return base unchanged")
	}
}

func TestMergeRetrySettings_OverrideOnly(t *testing.T) {
	override := &RetrySettings{MaxRetries: 10, BaseDelayMs: 2000, Provider: &ProviderRetrySettings{TimeoutMs: 15000}}
	result := mergeRetrySettings(nil, override)
	if result.MaxRetries != 10 {
		t.Errorf("MaxRetries = %d", result.MaxRetries)
	}
	if result.BaseDelayMs != 2000 {
		t.Errorf("BaseDelayMs = %d", result.BaseDelayMs)
	}
	if result.Provider == nil || result.Provider.TimeoutMs != 15000 {
		t.Errorf("Provider = %v", result.Provider)
	}
}

func TestMergeRetrySettings_Merge(t *testing.T) {
	tru := true
	base := &RetrySettings{
		Enabled:    &tru,
		MaxRetries: 3,
		BaseDelayMs: 1000,
		Provider:   &ProviderRetrySettings{TimeoutMs: 30000, MaxRetries: 2},
	}
	override := &RetrySettings{
		MaxRetries: 5,
		Provider:   &ProviderRetrySettings{MaxRetries: 4, MaxRetryDelayMs: 120000},
	}
	result := mergeRetrySettings(base, override)
	if result.Enabled == nil || *result.Enabled != true {
		t.Errorf("Enabled = %v", result.Enabled)
	}
	if result.MaxRetries != 5 {
		t.Errorf("MaxRetries = %d", result.MaxRetries)
	}
	if result.BaseDelayMs != 1000 {
		t.Errorf("BaseDelayMs = %d", result.BaseDelayMs)
	}
	if result.Provider == nil {
		t.Fatal("Provider is nil")
	}
	if result.Provider.TimeoutMs != 30000 {
		t.Errorf("Provider.TimeoutMs = %d, want 30000 (from base)", result.Provider.TimeoutMs)
	}
	if result.Provider.MaxRetries != 4 {
		t.Errorf("Provider.MaxRetries = %d, want 4 (from override)", result.Provider.MaxRetries)
	}
	if result.Provider.MaxRetryDelayMs != 120000 {
		t.Errorf("Provider.MaxRetryDelayMs = %d, want 120000 (from override)", result.Provider.MaxRetryDelayMs)
	}
}

func TestMergeRetrySettings_MergeWithNilProvider(t *testing.T) {
	base := &RetrySettings{
		MaxRetries: 3,
		Provider:   nil,
	}
	override := &RetrySettings{
		Provider: &ProviderRetrySettings{MaxRetries: 5},
	}
	result := mergeRetrySettings(base, override)
	if result.Provider == nil || result.Provider.MaxRetries != 5 {
		t.Errorf("Provider = %v", result.Provider)
	}
	if result.MaxRetries != 3 {
		t.Errorf("MaxRetries = %d", result.MaxRetries)
	}
}

func TestMergeBranchSummarySettings_BothNil(t *testing.T) {
	if mergeBranchSummarySettings(nil, nil) != nil {
		t.Error("mergeBranchSummarySettings(nil, nil) should return nil")
	}
}

func TestMergeBranchSummarySettings_BaseOnly(t *testing.T) {
	tru := true
	base := &BranchSummarySettings{SkipPrompt: &tru}
	if mergeBranchSummarySettings(base, nil) != base {
		t.Error("mergeBranchSummarySettings(base, nil) should return base unchanged")
	}
}

func TestMergeBranchSummarySettings_OverrideOnly(t *testing.T) {
	override := &BranchSummarySettings{ReserveTokens: 2000}
	result := mergeBranchSummarySettings(nil, override)
	if result.ReserveTokens != 2000 {
		t.Errorf("ReserveTokens = %d", result.ReserveTokens)
	}
}

func TestMergeBranchSummarySettings_Merge(t *testing.T) {
	tru := true
	fals := false
	base := &BranchSummarySettings{ReserveTokens: 1000, SkipPrompt: &tru}
	override := &BranchSummarySettings{ReserveTokens: 3000, SkipPrompt: &fals}
	result := mergeBranchSummarySettings(base, override)
	if result.ReserveTokens != 3000 {
		t.Errorf("ReserveTokens = %d", result.ReserveTokens)
	}
	if result.SkipPrompt == nil || *result.SkipPrompt != false {
		t.Errorf("SkipPrompt = %v", result.SkipPrompt)
	}
}

func TestMergeMarkdownSettings_BothNil(t *testing.T) {
	if mergeMarkdownSettings(nil, nil) != nil {
		t.Error("mergeMarkdownSettings(nil, nil) should return nil")
	}
}

func TestMergeMarkdownSettings_BaseOnly(t *testing.T) {
	base := &MarkdownSettings{CodeBlockIndent: "2"}
	if mergeMarkdownSettings(base, nil) != base {
		t.Error("mergeMarkdownSettings(base, nil) should return base unchanged")
	}
}

func TestMergeMarkdownSettings_OverrideOnly(t *testing.T) {
	override := &MarkdownSettings{CodeBlockIndent: "4"}
	result := mergeMarkdownSettings(nil, override)
	if result.CodeBlockIndent != "4" {
		t.Errorf("CodeBlockIndent = %q", result.CodeBlockIndent)
	}
}

func TestMergeMarkdownSettings_Merge(t *testing.T) {
	base := &MarkdownSettings{CodeBlockIndent: "2"}
	override := &MarkdownSettings{CodeBlockIndent: "8"}
	result := mergeMarkdownSettings(base, override)
	if result.CodeBlockIndent != "8" {
		t.Errorf("CodeBlockIndent = %q, want 8", result.CodeBlockIndent)
	}
}

func TestMergeWarningSettings_BothNil(t *testing.T) {
	if mergeWarningSettings(nil, nil) != nil {
		t.Error("mergeWarningSettings(nil, nil) should return nil")
	}
}

func TestMergeWarningSettings_BaseOnly(t *testing.T) {
	tru := true
	base := &WarningSettings{AnthropicExtraUsage: &tru}
	if mergeWarningSettings(base, nil) != base {
		t.Error("mergeWarningSettings(base, nil) should return base unchanged")
	}
}

func TestMergeWarningSettings_OverrideOnly(t *testing.T) {
	fals := false
	override := &WarningSettings{AnthropicExtraUsage: &fals}
	result := mergeWarningSettings(nil, override)
	if result.AnthropicExtraUsage == nil || *result.AnthropicExtraUsage != false {
		t.Errorf("AnthropicExtraUsage = %v", result.AnthropicExtraUsage)
	}
}

func TestMergeWarningSettings_Merge(t *testing.T) {
	tru := true
	fals := false
	base := &WarningSettings{AnthropicExtraUsage: &tru}
	override := &WarningSettings{AnthropicExtraUsage: &fals}
	result := mergeWarningSettings(base, override)
	if result.AnthropicExtraUsage == nil || *result.AnthropicExtraUsage != false {
		t.Errorf("AnthropicExtraUsage = %v, want false", result.AnthropicExtraUsage)
	}
}

// ---------------------------------------------------------------------------
// MergeSettings additional edge cases
// ---------------------------------------------------------------------------

func TestMergeSettings_EmptySettings(t *testing.T) {
	base := &Settings{}
	override := &Settings{}
	result := MergeSettings(base, override)
	if result.DefaultProvider != "" {
		t.Errorf("expected empty settings, got DefaultProvider=%q", result.DefaultProvider)
	}
	if result.CompactionEnabled != nil {
		t.Error("CompactionEnabled should be nil")
	}
}

func TestMergeSettings_Clone_DoesNotAlias(t *testing.T) {
	// Verify that MergeSettings always clones, even when one input is nil
	base := &Settings{DefaultModel: "gpt-4o", Extensions: []string{"a"}}
	// nil override -> should return clone of base
	result := MergeSettings(base, nil)
	result.DefaultModel = "modified"
	if base.DefaultModel != "gpt-4o" {
		t.Errorf("base.DefaultModel mutated: %q", base.DefaultModel)
	}

	// nil base -> should return clone of override
	override := &Settings{DefaultModel: "claude", Extensions: []string{"b"}}
	result = MergeSettings(nil, override)
	result.Extensions[0] = "modified"
	if override.Extensions[0] != "b" {
		t.Errorf("override.Extensions mutated: %v", override.Extensions)
	}
}
