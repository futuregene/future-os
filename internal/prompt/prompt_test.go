package prompt

import (
	"encoding/json"
	"strings"
	"testing"

	"github.com/huichen/xihu/pkg/types"
)

func makeReadTool() types.AgentTool {
	return types.AgentTool{
		Def: types.ToolDef{
			Type: "function",
			Function: types.FunctionDef{
				Name:        "read",
				Description: "Read file contents",
			},
		},
	}
}

func makeBashTool() types.AgentTool {
	return types.AgentTool{
		Def: types.ToolDef{
			Type: "function",
			Function: types.FunctionDef{
				Name:        "bash",
				Description: "Execute shell commands",
			},
		},
	}
}

func TestBuildPrompt_Default(t *testing.T) {
	result := BuildPrompt(PromptOptions{})
	if !strings.Contains(result, piDocsSection) {
		t.Errorf("expected default prompt in output, got: %s", result)
	}
}

func TestBuildPrompt_CustomPrompt(t *testing.T) {
	custom := "You are a helpful assistant."
	result := BuildPrompt(PromptOptions{CustomPrompt: custom})
	if !strings.HasPrefix(result, custom) {
		t.Errorf("expected custom prompt as base, got: %s", result)
	}
	if strings.Contains(result, piDocsSection) {
		t.Errorf("default prompt should not appear when custom is set")
	}
}

func TestBuildPrompt_DateAndCWD(t *testing.T) {
	result := BuildPrompt(PromptOptions{
		Date:             "2026-01-15",
		WorkingDirectory: "/home/user/project",
	})
	if !strings.Contains(result, "Current date: 2026-01-15") {
		t.Error("missing date injection")
	}
	if !strings.Contains(result, "Current working directory: /home/user/project") {
		t.Error("missing working directory injection")
	}
}

func TestBuildPrompt_DateOnly(t *testing.T) {
	result := BuildPrompt(PromptOptions{
		Date: "2026-06-01",
	})
	if !strings.Contains(result, "Current date: 2026-06-01") {
		t.Error("missing date injection")
	}
	if strings.Contains(result, "Current working directory") {
		t.Error("working directory should not appear when empty")
	}
}

func TestBuildPrompt_CWDOnly(t *testing.T) {
	result := BuildPrompt(PromptOptions{
		WorkingDirectory: "/tmp/test",
	})
	if !strings.Contains(result, "Current working directory: /tmp/test") {
		t.Error("missing working directory injection")
	}
	if strings.Contains(result, "Current date:") {
		t.Error("date should not appear when empty")
	}
}

func TestBuildPrompt_ToolSnippets(t *testing.T) {
	tools := []types.AgentTool{makeReadTool(), makeBashTool()}
	result := BuildPrompt(PromptOptions{Tools: tools})
	if !strings.Contains(result, "- read: Read file contents") {
		t.Error("missing read tool snippet")
	}
	if !strings.Contains(result, "- bash: Execute shell commands") {
		t.Error("missing bash tool snippet")
	}
	if !strings.Contains(result, "Available tools:") {
		t.Error("missing tools header")
	}
}

func TestBuildPrompt_NoTools(t *testing.T) {
	result := BuildPrompt(PromptOptions{})
	// New behavior: "(none)" is shown when no tools
	if !strings.Contains(result, "(none)") {
		t.Error("tools section should show (none) when no tools provided")
	}
}

func TestBuildPrompt_AGENTSContent(t *testing.T) {
	content := "# Project Rules\n- Keep it simple"
	result := BuildPrompt(PromptOptions{AGENTSContent: content})
	if !strings.Contains(result, "## Project Context") {
		t.Error("missing Project Context heading")
	}
	if !strings.Contains(result, "# Project Rules") {
		t.Error("missing AGENTS content")
	}
}

func TestBuildPrompt_EmptyAGENTSContent(t *testing.T) {
	result := BuildPrompt(PromptOptions{AGENTSContent: ""})
	if strings.Contains(result, "## Project Context") {
		t.Error("Project Context should not appear when empty")
	}
}

func TestBuildPrompt_Skills(t *testing.T) {
	// Skills are only injected when read tool is available
	tools := []types.AgentTool{makeReadTool()}
	skills := []Skill{
		{Name: "refactor", Description: "Refactor Go code", Location: "/skills/refactor.md"},
		{Name: "test-gen", Description: "Generate unit tests", Location: "/skills/test-gen.md"},
	}
	result := BuildPrompt(PromptOptions{Skills: skills, Tools: tools})
	if !strings.Contains(result, "<available_skills>") {
		t.Error("missing opening available_skills tag")
	}
	if !strings.Contains(result, "</available_skills>") {
		t.Error("missing closing available_skills tag")
	}
	if !strings.Contains(result, `name="refactor"`) {
		t.Error("missing refactor skill")
	}
	if !strings.Contains(result, `name="test-gen"`) {
		t.Error("missing test-gen skill")
	}
}

func TestBuildPrompt_SkillsWithoutReadTool(t *testing.T) {
	// Skills should NOT appear if read tool is not available
	skills := []Skill{
		{Name: "refactor", Description: "Refactor Go code"},
	}
	result := BuildPrompt(PromptOptions{Skills: skills}) // no tools
	if strings.Contains(result, "<available_skills>") {
		t.Error("skills should not appear without read tool")
	}
}

func TestBuildPrompt_SkillsXMLEscaping(t *testing.T) {
	tools := []types.AgentTool{makeReadTool()}
	skills := []Skill{
		{Name: "code & review", Description: "Review <code> & test"},
	}
	result := BuildPrompt(PromptOptions{Skills: skills, Tools: tools})
	if !strings.Contains(result, "&amp;") {
		t.Error("missing XML escaping in name/description")
	}
	if !strings.Contains(result, "&lt;") {
		t.Error("missing XML escaping for <")
	}
}

func TestBuildPrompt_GuidelinesDeduplication(t *testing.T) {
	guidelines := []string{
		"Be concise",
		"Be helpful",
		"Be concise",
		"  Be concise  ",
		"",
	}
	result := BuildPrompt(PromptOptions{PromptGuidelines: guidelines})
	// Default guidelines always include "Be concise in your responses"
	// "General Guidelines:" header should be present (default guidelines exist)
	if !strings.Contains(result, "General Guidelines:") {
		t.Error("missing General Guidelines header")
	}
	// Check that deduplication works even with default guidelines
	count := strings.Count(result, "- Be concise\n")
	if count != 1 {
		t.Errorf("expected 1 occurrence of '- Be concise' in guidelines, got %d", count)
	}
	if !strings.Contains(result, "- Be helpful") {
		t.Error("missing non-duplicate guideline")
	}
}

func TestBuildPrompt_DefaultGuidelines(t *testing.T) {
	// Default behavioral guidelines are always present
	result := BuildPrompt(PromptOptions{})
	if !strings.Contains(result, "General Guidelines:") {
		t.Error("default behavioral guidelines should always appear")
	}
	if !strings.Contains(result, "Be concise in your responses") {
		t.Error("missing default 'Be concise' guideline")
	}
	if !strings.Contains(result, "Show file paths clearly when working with files") {
		t.Error("missing default 'Show file paths' guideline")
	}
}

func TestBuildPrompt_AppendPrompt(t *testing.T) {
	result := BuildPrompt(PromptOptions{
		AppendPrompt: "Remember to always format your output.",
	})
	if !strings.HasSuffix(strings.TrimSpace(result), "Remember to always format your output.") {
		t.Errorf("append prompt should be at the end, got: %s", result)
	}
}

func TestBuildPrompt_FullIntegration(t *testing.T) {
	tools := []types.AgentTool{makeBashTool(), makeReadTool()}
	skills := []Skill{
		{Name: "go-dev", Description: "Go development helper"},
	}
	guidelines := []string{
		"Use idiomatic Go",
		"Handle errors properly",
	}

	result := BuildPrompt(PromptOptions{
		CustomPrompt:     "You are an expert Go developer.",
		WorkingDirectory: "/home/dev/project",
		Date:             "2026-05-08",
		Tools:            tools,
		Skills:           skills,
		AGENTSContent:    "# AGENTS.md\nRule: write tests first",
		AppendPrompt:     "Always run tests before submitting.",
		PromptGuidelines: guidelines,
	})

	checks := []string{
		"You are an expert Go developer.",
		"Current date: 2026-05-08",
		"Current working directory: /home/dev/project",
		"Available tools:",
		"- bash: Execute shell commands",
		"- read: Read file contents",
		"## Project Context",
		"# AGENTS.md",
		"<available_skills>",
		`name="go-dev"`,
		"</available_skills>",
		"General Guidelines:",
		"- Use idiomatic Go",
		"- Handle errors properly",
		"Always run tests before submitting.",
	}

	for _, check := range checks {
		if !strings.Contains(result, check) {
			t.Errorf("full integration: missing '%s' in output", check)
		}
	}

	// Default prompt should not appear since custom is set
	if strings.Contains(result, piDocsSection) {
		t.Error("default prompt should not appear when custom prompt is set")
	}
}

func TestBuildPrompt_ToolWithRawJSONParams(t *testing.T) {
	tools := []types.AgentTool{
		{
			Def: types.ToolDef{
				Type: "function",
				Function: types.FunctionDef{
					Name:        "complex_tool",
					Description: "Does complex things",
					Parameters:  json.RawMessage(`{"type":"object"}`),
				},
			},
			Handler: nil,
		},
	}
	result := BuildPrompt(PromptOptions{Tools: tools})
	if !strings.Contains(result, "- complex_tool: Does complex things") {
		t.Error("missing tool with raw JSON parameters")
	}
}

func TestBuildPrompt_DynamicGuidelines(t *testing.T) {
	// Bash-only: should suggest using bash for file ops
	bashOnly := []types.AgentTool{makeBashTool()}
	result := BuildPrompt(PromptOptions{Tools: bashOnly})
	if !strings.Contains(result, "Use bash for file system operations") {
		t.Error("missing dynamic guideline for bash-only mode")
	}

	// Bash + grep: should suggest preferring grep over bash
	bashAndGrep := []types.AgentTool{makeBashTool(), {
		Def: types.ToolDef{
			Type: "function",
			Function: types.FunctionDef{
				Name:        "grep",
				Description: "Search files",
			},
		},
	}}
	result2 := BuildPrompt(PromptOptions{Tools: bashAndGrep})
	if !strings.Contains(result2, "Prefer specialized tools") {
		t.Error("missing dynamic guideline for bash+grep mode")
	}
}

func TestDeduplicateGuidelines(t *testing.T) {
	tests := []struct {
		name   string
		input  []string
		expect []string
	}{
		{
			name:   "empty",
			input:  nil,
			expect: nil,
		},
		{
			name:   "no duplicates",
			input:  []string{"a", "b", "c"},
			expect: []string{"a", "b", "c"},
		},
		{
			name:   "with duplicates",
			input:  []string{"a", "b", "a", "c", "b"},
			expect: []string{"a", "b", "c"},
		},
		{
			name:   "whitespace only",
			input:  []string{"", "  ", "\t"},
			expect: nil,
		},
		{
			name:   "mixed whitespace",
			input:  []string{"a", "  a  ", "b"},
			expect: []string{"a", "b"},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := deduplicateGuidelines(tt.input)
			if len(got) != len(tt.expect) {
				t.Errorf("expected %d items, got %d: %v", len(tt.expect), len(got), got)
				return
			}
			for i := range got {
				if got[i] != tt.expect[i] {
					t.Errorf("item %d: expected %q, got %q", i, tt.expect[i], got[i])
				}
			}
		})
	}
}

func TestEscapeXML(t *testing.T) {
	tests := []struct {
		input  string
		expect string
	}{
		{"hello", "hello"},
		{"a & b", "a &amp; b"},
		{"<tag>", "&lt;tag&gt;"},
		{`"quoted"`, "&quot;quoted&quot;"},
		{"it's", "it&apos;s"},
	}

	for _, tt := range tests {
		got := escapeXML(tt.input)
		if got != tt.expect {
			t.Errorf("escapeXML(%q) = %q, want %q", tt.input, got, tt.expect)
		}
	}
}
