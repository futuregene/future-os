package skills

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// =============================================================================
// DiscoverSkills tests
// =============================================================================

func TestDiscoverSkills_ValidSkill(t *testing.T) {
	dir := t.TempDir()
	writeSkillMD(t, dir, "my-skill", "Does something useful", false)

	skills, err := DiscoverSkills([]string{dir}, "project")
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if len(skills) != 1 {
		t.Fatalf("expected 1 skill, got %d", len(skills))
	}

	s := skills[0]
	if s.Name != "my-skill" {
		t.Errorf("expected name 'my-skill', got %q", s.Name)
	}
	if s.Description != "Does something useful" {
		t.Errorf("expected description 'Does something useful', got %q", s.Description)
	}
	if s.Source != "project" {
		t.Errorf("expected source 'project', got %q", s.Source)
	}
	if s.Path != filepath.Join(dir, "SKILL.md") {
		t.Errorf("unexpected path: %q", s.Path)
	}
	if s.DisableModelInvocation {
		t.Error("expected disable-model-invocation to be false")
	}
}

func TestDiscoverSkills_MultipleSkills(t *testing.T) {
	dir := t.TempDir()
	writeSkillMD(t, dir, "skill-a", "First skill", false)

	subdir := filepath.Join(dir, "sub")
	os.MkdirAll(subdir, 0o755)
	writeSkillMD(t, subdir, "skill-b", "Second skill", false)

	skills, err := DiscoverSkills([]string{dir}, "project")
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if len(skills) != 2 {
		t.Fatalf("expected 2 skills, got %d", len(skills))
	}
}

func TestDiscoverSkills_DisableModelInvocation(t *testing.T) {
	dir := t.TempDir()
	writeSkillMD(t, dir, "auto-skill", "Auto invoked", true)

	skills, err := DiscoverSkills([]string{dir}, "user")
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if len(skills) != 1 {
		t.Fatalf("expected 1 skill, got %d", len(skills))
	}
	if !skills[0].DisableModelInvocation {
		t.Error("expected DisableModelInvocation to be true")
	}
}

func TestDiscoverSkills_NoFrontmatter(t *testing.T) {
	dir := t.TempDir()
	writeFile(t, filepath.Join(dir, "SKILL.md"), "# Just a readme\n\nNo frontmatter here.\n")

	skills, err := DiscoverSkills([]string{dir}, "project")
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if len(skills) != 0 {
		t.Errorf("expected 0 skills for file without frontmatter, got %d", len(skills))
	}
}

func TestDiscoverSkills_MissingName(t *testing.T) {
	dir := t.TempDir()
	writeFile(t, filepath.Join(dir, "SKILL.md"), "---\ndescription: No name here\n---\n# Body\n")

	skills, err := DiscoverSkills([]string{dir}, "project")
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if len(skills) != 0 {
		t.Errorf("expected 0 skills when name is missing, got %d", len(skills))
	}
}

func TestDiscoverSkills_SkipsNodeModules(t *testing.T) {
	dir := t.TempDir()
	nm := filepath.Join(dir, "node_modules")
	os.MkdirAll(nm, 0o755)
	writeSkillMD(t, nm, "npm-skill", "Should be skipped", false)

	skills, err := DiscoverSkills([]string{dir}, "project")
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if len(skills) != 0 {
		t.Errorf("expected 0 skills (node_modules skipped), got %d", len(skills))
	}
}

func TestDiscoverSkills_SkipsDotGit(t *testing.T) {
	dir := t.TempDir()
	gitDir := filepath.Join(dir, ".git")
	os.MkdirAll(gitDir, 0o755)
	writeSkillMD(t, gitDir, "git-skill", "Should be skipped", false)

	skills, err := DiscoverSkills([]string{dir}, "project")
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if len(skills) != 0 {
		t.Errorf("expected 0 skills (.git skipped), got %d", len(skills))
	}
}

func TestDiscoverSkills_QuotedValues(t *testing.T) {
	dir := t.TempDir()
	content := `---
name: "quoted-skill"
description: 'Has "quotes" inside'
---
Body text.`

	writeFile(t, filepath.Join(dir, "SKILL.md"), content)

	skills, err := DiscoverSkills([]string{dir}, "project")
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if len(skills) != 1 {
		t.Fatalf("expected 1 skill, got %d", len(skills))
	}
	if skills[0].Name != "quoted-skill" {
		t.Errorf("expected name 'quoted-skill', got %q", skills[0].Name)
	}
	if skills[0].Description != `Has "quotes" inside` {
		t.Errorf("expected description with quotes, got %q", skills[0].Description)
	}
}

func TestDiscoverSkills_NonExistentDir(t *testing.T) {
	skills, err := DiscoverSkills([]string{"/nonexistent/path/12345"}, "project")
	if err != nil {
		t.Fatalf("unexpected error for nonexistent dir: %v", err)
	}
	if len(skills) != 0 {
		t.Errorf("expected 0 skills for nonexistent dir, got %d", len(skills))
	}
}

func TestDiscoverSkills_EmptyDirs(t *testing.T) {
	skills, err := DiscoverSkills([]string{}, "project")
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if len(skills) != 0 {
		t.Errorf("expected 0 skills for empty dirs, got %d", len(skills))
	}
}

func TestDiscoverSkills_MultipleSourceDirs(t *testing.T) {
	dir1 := t.TempDir()
	dir2 := t.TempDir()

	writeSkillMD(t, dir1, "skill-1", "From dir1", false)
	writeSkillMD(t, dir2, "skill-2", "From dir2", false)

	skills, err := DiscoverSkills([]string{dir1, dir2}, "custom")
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if len(skills) != 2 {
		t.Fatalf("expected 2 skills, got %d", len(skills))
	}
}

// =============================================================================
// ValidateSkill tests
// =============================================================================

func TestValidateSkill_Valid(t *testing.T) {
	tests := []struct {
		name string
		sk   Skill
	}{
		{"simple", Skill{Name: "skill", Description: "A simple skill"}},
		{"kebab", Skill{Name: "my-skill", Description: "Kebab case"}},
		{"multi-segment", Skill{Name: "a-b-c", Description: "Multi segment"}},
		{"single-char", Skill{Name: "x", Description: "Single char name"}},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if err := ValidateSkill(tt.sk); err != nil {
				t.Errorf("expected valid, got error: %v", err)
			}
		})
	}
}

func TestValidateSkill_InvalidNames(t *testing.T) {
	tests := []struct {
		name string
		sk   Skill
		msg  string
	}{
		{"uppercase", Skill{Name: "MySkill", Description: "desc"}, "uppercase"},
		{"consecutive-hyphens", Skill{Name: "my--skill", Description: "desc"}, "consecutive"},
		{"leading-hyphen", Skill{Name: "-skill", Description: "desc"}, "leading"},
		{"trailing-hyphen", Skill{Name: "skill-", Description: "desc"}, "trailing"},
		{"spaces", Skill{Name: "my skill", Description: "desc"}, "space"},
		{"underscore", Skill{Name: "my_skill", Description: "desc"}, "underscore"},
		{"empty", Skill{Name: "", Description: "desc"}, "empty"},
		{"special-chars", Skill{Name: "skill@1", Description: "desc"}, "special"},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if err := ValidateSkill(tt.sk); err == nil {
				t.Errorf("expected error for %s name, got nil", tt.msg)
			}
		})
	}
}

func TestValidateSkill_NameTooLong(t *testing.T) {
	longName := strings.Repeat("a", 65) // 65 chars, max is 64
	sk := Skill{Name: longName, Description: "desc"}
	if err := ValidateSkill(sk); err == nil {
		t.Error("expected error for name > 64 chars")
	}
}

func TestValidateSkill_NameMaxLength(t *testing.T) {
	// 64 chars should be ok: 32 segments of "a-" pattern, plus final "a"
	longName := strings.Repeat("a", 64)
	sk := Skill{Name: longName, Description: "desc"}
	if err := ValidateSkill(sk); err != nil {
		t.Errorf("expected valid for 64-char name, got: %v", err)
	}
}

func TestValidateSkill_DescriptionRequired(t *testing.T) {
	sk := Skill{Name: "my-skill", Description: ""}
	if err := ValidateSkill(sk); err == nil {
		t.Error("expected error for empty description")
	}
}

func TestValidateSkill_DescriptionTooLong(t *testing.T) {
	longDesc := strings.Repeat("x", 1025) // 1025 chars, max is 1024
	sk := Skill{Name: "my-skill", Description: longDesc}
	if err := ValidateSkill(sk); err == nil {
		t.Error("expected error for description > 1024 chars")
	}
}

func TestValidateSkill_DescriptionMaxLength(t *testing.T) {
	desc := strings.Repeat("x", 1024)
	sk := Skill{Name: "my-skill", Description: desc}
	if err := ValidateSkill(sk); err != nil {
		t.Errorf("expected valid for 1024-char description, got: %v", err)
	}
}

// =============================================================================
// FormatSkillsXML tests
// =============================================================================

func TestFormatSkillsXML_Empty(t *testing.T) {
	result := FormatSkillsXML(nil)
	if result != "" {
		t.Errorf("expected empty string for nil skills, got %q", result)
	}

	result = FormatSkillsXML([]Skill{})
	if result != "" {
		t.Errorf("expected empty string for empty skills, got %q", result)
	}
}

func TestFormatSkillsXML_Single(t *testing.T) {
	skills := []Skill{
		{Name: "refactor", Description: "Refactor Go code"},
	}
	result := FormatSkillsXML(skills)

	if !strings.Contains(result, "<available_skills>") {
		t.Error("missing opening tag")
	}
	if !strings.Contains(result, "</available_skills>") {
		t.Error("missing closing tag")
	}
	if !strings.Contains(result, `<skill name="refactor">Refactor Go code</skill>`) {
		t.Error("missing skill element")
	}
}

func TestFormatSkillsXML_Multiple(t *testing.T) {
	skills := []Skill{
		{Name: "refactor", Description: "Refactor Go code"},
		{Name: "test-gen", Description: "Generate unit tests"},
	}
	result := FormatSkillsXML(skills)

	if !strings.Contains(result, `<skill name="refactor">Refactor Go code</skill>`) {
		t.Error("missing refactor skill")
	}
	if !strings.Contains(result, `<skill name="test-gen">Generate unit tests</skill>`) {
		t.Error("missing test-gen skill")
	}
}

func TestFormatSkillsXML_Escaping(t *testing.T) {
	skills := []Skill{
		{Name: "code-review", Description: "Review <code> & test"},
	}
	result := FormatSkillsXML(skills)

	if !strings.Contains(result, "&amp;") {
		t.Error("missing XML escaping for &")
	}
	if !strings.Contains(result, "&lt;") {
		t.Error("missing XML escaping for <")
	}
	if !strings.Contains(result, "&gt;") {
		t.Error("missing XML escaping for >")
	}
}

// =============================================================================
// ResolveCollisions tests
// =============================================================================

func TestResolveCollisions_Empty(t *testing.T) {
	result := ResolveCollisions(nil)
	if result != nil {
		t.Errorf("expected nil for nil input, got %v", result)
	}

	result = ResolveCollisions([]Skill{})
	if result != nil && len(result) != 0 {
		t.Errorf("expected nil or empty slice for empty input, got %v", result)
	}
}

func TestResolveCollisions_NoConflicts(t *testing.T) {
	skills := []Skill{
		{Name: "a", Description: "Skill A", Source: "project"},
		{Name: "b", Description: "Skill B", Source: "user"},
		{Name: "c", Description: "Skill C", Source: "project"},
	}
	result := ResolveCollisions(skills)
	if len(result) != 3 {
		t.Fatalf("expected 3 skills, got %d", len(result))
	}
}

func TestResolveCollisions_ProjectOverUser(t *testing.T) {
	skills := []Skill{
		{Name: "my-skill", Description: "From user", Source: "user"},
		{Name: "my-skill", Description: "From project", Source: "project"},
	}
	result := ResolveCollisions(skills)
	if len(result) != 1 {
		t.Fatalf("expected 1 skill after collision, got %d", len(result))
	}
	if result[0].Description != "From project" {
		t.Errorf("expected project version to win, got %q", result[0].Description)
	}
}

func TestResolveCollisions_FirstWinsWithinSameSource(t *testing.T) {
	skills := []Skill{
		{Name: "my-skill", Description: "First project", Source: "project"},
		{Name: "my-skill", Description: "Second project", Source: "project"},
	}
	result := ResolveCollisions(skills)
	if len(result) != 1 {
		t.Fatalf("expected 1 skill, got %d", len(result))
	}
	if result[0].Description != "First project" {
		t.Errorf("expected first-wins, got %q", result[0].Description)
	}
}

func TestResolveCollisions_UserVsUserFirstWins(t *testing.T) {
	skills := []Skill{
		{Name: "my-skill", Description: "First user", Source: "user"},
		{Name: "my-skill", Description: "Second user", Source: "user"},
	}
	result := ResolveCollisions(skills)
	if len(result) != 1 {
		t.Fatalf("expected 1 skill, got %d", len(result))
	}
	if result[0].Description != "First user" {
		t.Errorf("expected first user version, got %q", result[0].Description)
	}
}

func TestResolveCollisions_MixedSources(t *testing.T) {
	skills := []Skill{
		{Name: "a", Description: "a-user", Source: "user"},
		{Name: "a", Description: "a-project", Source: "project"},
		{Name: "b", Description: "b-project", Source: "project"},
		{Name: "b", Description: "b-user", Source: "user"},
		{Name: "c", Description: "c-user-1", Source: "user"},
		{Name: "c", Description: "c-user-2", Source: "user"},
	}
	result := ResolveCollisions(skills)

	if len(result) != 3 {
		t.Fatalf("expected 3 skills, got %d", len(result))
	}

	// a: project wins
	if result[0].Name != "a" || result[0].Description != "a-project" {
		t.Errorf("expected a-project, got %q", result[0].Description)
	}
	// b: project wins (appears first)
	if result[1].Name != "b" || result[1].Description != "b-project" {
		t.Errorf("expected b-project, got %q", result[1].Description)
	}
	// c: first user wins
	if result[2].Name != "c" || result[2].Description != "c-user-1" {
		t.Errorf("expected c-user-1, got %q", result[2].Description)
	}
}

func TestResolveCollisions_UserOverPath(t *testing.T) {
	skills := []Skill{
		{Name: "my-skill", Description: "From path", Source: "/custom/path"},
		{Name: "my-skill", Description: "From user", Source: "user"},
	}
	result := ResolveCollisions(skills)
	if len(result) != 1 {
		t.Fatalf("expected 1 skill, got %d", len(result))
	}
	// user (rank 1) beats path (rank 2)
	if result[0].Description != "From user" {
		t.Errorf("expected user version over path source, got %q", result[0].Description)
	}
}

func TestResolveCollisions_PreservesFirstOccurrenceOrder(t *testing.T) {
	// Winner is determined by which appears first AND has best rank
	skills := []Skill{
		{Name: "z", Description: "z-first", Source: "project"},
		{Name: "a", Description: "a-first", Source: "project"},
		{Name: "z", Description: "z-second", Source: "user"},
	}
	result := ResolveCollisions(skills)
	if len(result) != 2 {
		t.Fatalf("expected 2 skills, got %d", len(result))
	}
	// Order should be z, a (first occurrence of each name)
	if result[0].Name != "z" {
		t.Errorf("expected z first, got %s", result[0].Name)
	}
	if result[1].Name != "a" {
		t.Errorf("expected a second, got %s", result[1].Name)
	}
	// z-first (project) should win over z-second (user) because project > user
	if result[0].Description != "z-first" {
		t.Errorf("expected z-first (project) to win, got %q", result[0].Description)
	}
}

// =============================================================================
// parseFrontmatter tests
// =============================================================================

func TestParseFrontmatter_Standard(t *testing.T) {
	text := "---\nname: my-skill\ndescription: Does things\n---\n# Body"
	name, desc, disable := parseFrontmatter(text)
	if name != "my-skill" {
		t.Errorf("expected name 'my-skill', got %q", name)
	}
	if desc != "Does things" {
		t.Errorf("expected description 'Does things', got %q", desc)
	}
	if disable {
		t.Error("expected disable=false")
	}
}

func TestParseFrontmatter_WithDisableModelInvocation(t *testing.T) {
	text := "---\nname: my-skill\ndescription: Does things\ndisable-model-invocation: true\n---\n"
	name, desc, disable := parseFrontmatter(text)
	if name != "my-skill" {
		t.Errorf("expected name 'my-skill', got %q", name)
	}
	if !disable {
		t.Error("expected disable=true")
	}
	_ = desc
}

func TestParseFrontmatter_DisableFalse(t *testing.T) {
	text := "---\nname: my-skill\ndescription: desc\ndisable-model-invocation: false\n---\n"
	_, _, disable := parseFrontmatter(text)
	if disable {
		t.Error("expected disable=false")
	}
}

func TestParseFrontmatter_NoFrontmatter(t *testing.T) {
	text := "# No frontmatter\nJust content."
	name, desc, disable := parseFrontmatter(text)
	if name != "" {
		t.Errorf("expected empty name, got %q", name)
	}
	_ = desc
	_ = disable
}

func TestParseFrontmatter_OnlyOpeningDashes(t *testing.T) {
	text := "---\nname: my-skill\nNo closing dashes"
	name, _, _ := parseFrontmatter(text)
	// Should not find closing --- so should return empty
	if name != "" {
		t.Errorf("expected empty name when no closing ---, got %q", name)
	}
}

func TestParseFrontmatter_CommentedLines(t *testing.T) {
	text := "---\n# This is a comment\nname: my-skill\n# Another comment\ndescription: Has things\n---\n"
	name, desc, _ := parseFrontmatter(text)
	if name != "my-skill" {
		t.Errorf("expected name 'my-skill', got %q", name)
	}
	if desc != "Has things" {
		t.Errorf("expected description 'Has things', got %q", desc)
	}
}

func TestParseFrontmatter_BlankLines(t *testing.T) {
	text := "---\n\nname: my-skill\n\ndescription: Does things\n\n---\n"
	name, desc, _ := parseFrontmatter(text)
	if name != "my-skill" {
		t.Errorf("expected name 'my-skill', got %q", name)
	}
	if desc != "Does things" {
		t.Errorf("expected description 'Does things', got %q", desc)
	}
}

func TestParseFrontmatter_CRLF(t *testing.T) {
	text := "---\r\nname: my-skill\r\ndescription: Does things\r\n---\r\nBody"
	name, desc, _ := parseFrontmatter(text)
	if name != "my-skill" {
		t.Errorf("expected name 'my-skill', got %q", name)
	}
	if desc != "Does things" {
		t.Errorf("expected description 'Does things', got %q", desc)
	}
}

// =============================================================================
// Helpers
// =============================================================================

func writeSkillMD(t *testing.T, dir, name, description string, disableModelInvocation bool) {
	t.Helper()

	var content strings.Builder
	content.WriteString("---\n")
	content.WriteString("name: " + name + "\n")
	content.WriteString("description: " + description + "\n")
	if disableModelInvocation {
		content.WriteString("disable-model-invocation: true\n")
	}
	content.WriteString("---\n")
	content.WriteString("# " + name + "\n\nSkill body text.\n")

	writeFile(t, filepath.Join(dir, "SKILL.md"), content.String())
}

func writeFile(t *testing.T, path, content string) {
	t.Helper()
	if err := os.WriteFile(path, []byte(content), 0o644); err != nil {
		t.Fatalf("failed to write file %s: %v", path, err)
	}
}
