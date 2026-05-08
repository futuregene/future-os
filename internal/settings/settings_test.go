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

	if !strings.Contains(global, ".cobalt") {
		t.Errorf("global path should contain .cobalt: %s", global)
	}
	if !strings.HasSuffix(global, "settings.json") {
		t.Errorf("global path should end with settings.json: %s", global)
	}

	if !strings.Contains(project, ".cobalt") {
		t.Errorf("project path should contain .cobalt: %s", project)
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
	// LoadAll uses real paths (~/.cobalt/settings.json and .cobalt/settings.json).
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
