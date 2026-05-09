package prompt

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"

	"github.com/huichen/xihu/pkg/types"
)

// Skill represents an available skill with name and description.
type Skill struct {
	Name        string
	Description string
	Location    string // file path where the skill was found
}

// PromptOptions configures the dynamic system prompt builder.
type PromptOptions struct {
	CustomPrompt     string            // If set, used as the base prompt instead of default
	WorkingDirectory string            // Current working directory path
	Date             string            // Date in format "2026-01-15"
	Tools            []types.AgentTool // Available tools for snippet generation
	Skills           []Skill           // Available skills for XML injection
	AGENTSContent    string            // Content from AGENTS.md / CLAUDE.md to inject
	AppendPrompt     string            // Additional prompt appended at the end
	PromptGuidelines []string          // Guidelines to append with deduplication
}

// defaultBehavioralGuidelines are always injected as base behavioral guidelines.
var defaultBehavioralGuidelines = []string{
	"Be concise in your responses",
	"Show file paths clearly when working with files",
}

// piDocsSection injects pi-specific documentation references into the system prompt.
const piDocsSection = `You are pi, a coding agent created by Earendil Works.
You have tools to read, write, edit, and execute code.
Be concise and direct.

For advanced usage and configuration, refer to the pi documentation:
- Extensions: pi.dev/docs/extensions
- Themes: pi.dev/docs/themes  
- Skills: pi.dev/docs/skills
- TUI keybindings: pi.dev/docs/tui
- SDK/API: pi.dev/docs/sdk
- Providers & models: pi.dev/docs/providers
- Packages: pi.dev/docs/packages

When in doubt about pi-specific features, consult the docs rather than guessing.`

// BuildPrompt produces a fully assembled system prompt from the given options.
func BuildPrompt(opts PromptOptions) string {
	var sections []string

	// 1. Base prompt
	if opts.CustomPrompt != "" {
		sections = append(sections, opts.CustomPrompt)
		// Custom prompt replaces default, but we still inject context below
	} else {
		sections = append(sections, piDocsSection)
	}

	// 2. Tool snippets: one-line descriptions (first sentence only)
	if len(opts.Tools) > 0 {
		var toolLines []string
		toolLines = append(toolLines, "Available tools:")
		for _, t := range opts.Tools {
			snippet := extractFirstSentence(t.Def.Function.Description)
			toolLines = append(toolLines, fmt.Sprintf("- %s: %s", t.Def.Function.Name, snippet))
		}
		// Hint about possible custom tools
		toolLines = append(toolLines, "In addition to the tools above, you may have access to other custom tools depending on the project.")
		sections = append(sections, strings.Join(toolLines, "\n"))
	} else {
		sections = append(sections, "Available tools:\n(none)")
	}

	// 3. Dynamic tool-based guidelines (conditional on which tools are present)
	dynamicGuidelines := buildDynamicToolGuidelines(opts.Tools)
	if len(dynamicGuidelines) > 0 {
		lines := []string{"Guidelines:"}
		for _, g := range dynamicGuidelines {
			lines = append(lines, "- "+g)
		}
		sections = append(sections, strings.Join(lines, "\n"))
	}

	// 4. AGENTS.md / CLAUDE.md content injection
	if opts.AGENTSContent != "" {
		trimmed := strings.TrimSpace(opts.AGENTSContent)
		sections = append(sections, "## Project Context\n\n"+trimmed)
	}

	// 5. Skills XML injection (only if read tool is available)
	if len(opts.Skills) > 0 && hasTool(opts.Tools, "read") {
		var skillLines []string
		skillLines = append(skillLines, "You have access to the following skills. Use the read tool to view their full SKILL.md files for detailed instructions.")
		skillLines = append(skillLines, "<available_skills>")
		for _, s := range opts.Skills {
			loc := ""
			if s.Location != "" {
				loc = fmt.Sprintf(" location=\"%s\"", escapeXML(s.Location))
			}
			skillLines = append(skillLines, fmt.Sprintf("  <skill name=\"%s\"%s>%s</skill>",
				escapeXML(s.Name), loc, escapeXML(s.Description)))
		}
		skillLines = append(skillLines, "</available_skills>")
		sections = append(sections, strings.Join(skillLines, "\n"))
	}

	// 6. Combined behavioral guidelines (default + opts + tool guidelines)
	var allGuidelines []string
	allGuidelines = append(allGuidelines, defaultBehavioralGuidelines...)
	allGuidelines = append(allGuidelines, opts.PromptGuidelines...)
	for _, t := range opts.Tools {
		allGuidelines = append(allGuidelines, t.Guidelines...)
	}
	if len(allGuidelines) > 0 {
		deduped := deduplicateGuidelines(allGuidelines)
		if len(deduped) > 0 {
			var guideLines []string
			guideLines = append(guideLines, "General Guidelines:")
			for _, g := range deduped {
				guideLines = append(guideLines, "- "+g)
			}
			sections = append(sections, strings.Join(guideLines, "\n"))
		}
	}

	// 7. Date and working directory injection
	if opts.Date != "" || opts.WorkingDirectory != "" {
		var infoLines []string
		if opts.Date != "" {
			infoLines = append(infoLines, fmt.Sprintf("Current date: %s", opts.Date))
		}
		if opts.WorkingDirectory != "" {
			infoLines = append(infoLines, fmt.Sprintf("Current working directory: %s", opts.WorkingDirectory))
		}
		sections = append(sections, strings.Join(infoLines, "\n"))
	}

	// 8. Append prompt (always last)
	if opts.AppendPrompt != "" {
		sections = append(sections, opts.AppendPrompt)
	}

	return strings.Join(sections, "\n\n")
}

// buildDynamicToolGuidelines returns conditional guidelines based on which tools are available.
func buildDynamicToolGuidelines(tools []types.AgentTool) []string {
	hasBash := hasTool(tools, "bash")
	hasGrep := hasTool(tools, "grep")
	hasFind := hasTool(tools, "find")
	hasLs := hasTool(tools, "ls")
	hasEdit := hasTool(tools, "edit")
	hasRead := hasTool(tools, "read")

	var guidelines []string

	if hasBash && !hasGrep && !hasFind && !hasLs {
		guidelines = append(guidelines, "Use bash for file system operations (grep/find/ls are not available)")
	}
	if hasBash && (hasGrep || hasFind || hasLs) {
		guidelines = append(guidelines, "Prefer specialized tools (grep/find/ls) over bash for file system operations when possible")
	}
	if hasEdit && hasRead {
		guidelines = append(guidelines, "Prefer the edit tool for targeted changes — it's more efficient than read+write cycles")
	}
	if hasBash {
		guidelines = append(guidelines, "When using bash, consider running tests after making changes to verify correctness")
	}

	return guidelines
}

// hasTool checks if a tool with the given name is in the tools list.
func hasTool(tools []types.AgentTool, name string) bool {
	for _, t := range tools {
		if t.Def.Function.Name == name {
			return true
		}
	}
	return false
}

// deduplicateGuidelines removes duplicate guidelines while preserving order.
func deduplicateGuidelines(guidelines []string) []string {
	seen := make(map[string]bool)
	result := make([]string, 0, len(guidelines))
	for _, g := range guidelines {
		trimmed := strings.TrimSpace(g)
		if trimmed == "" {
			continue
		}
		if !seen[trimmed] {
			seen[trimmed] = true
			result = append(result, trimmed)
		}
	}
	return result
}

// extractFirstSentence returns the first sentence of a description (up to and
// including the first period). If there is no period, returns the entire string.
func extractFirstSentence(s string) string {
	if idx := strings.Index(s, "."); idx >= 0 {
		return s[:idx+1]
	}
	return s
}

// escapeXML performs basic XML entity escaping for name and description values.
func escapeXML(s string) string {
	s = strings.ReplaceAll(s, "&", "&amp;")
	s = strings.ReplaceAll(s, "\"", "&quot;")
	s = strings.ReplaceAll(s, "'", "&apos;")
	s = strings.ReplaceAll(s, "<", "&lt;")
	s = strings.ReplaceAll(s, ">", "&gt;")
	return s
}

// ContextFile represents a discovered context file (AGENTS.md, CLAUDE.md, etc.).
type ContextFile struct {
	Path    string // absolute path
	Content string // file content
}

// contextFileNames lists files scanned for implicit context rules.
var contextFileNames = []string{"AGENTS.md", "AGENTS.MD", "CLAUDE.md", "CLAUDE.MD"}

// DiscoverContextFiles scans the agent directory and project root for context files.
// Returns discovered files in order: global config first, then project root and ancestors.
func DiscoverContextFiles(agentDir, cwd string) []ContextFile {
	var files []ContextFile
	seen := make(map[string]bool)

	tryDir := func(dir string) {
		for _, name := range contextFileNames {
			fp := filepath.Join(dir, name)
			if seen[fp] {
				continue
			}
			seen[fp] = true
			data, err := os.ReadFile(fp)
			if err == nil {
				files = append(files, ContextFile{Path: fp, Content: string(data)})
			}
		}
	}

	// 1. Global agent directory (~/.xihu/)
	if agentDir != "" {
		tryDir(agentDir)
	}

	// 2. Traverse from cwd up to root
	if cwd != "" {
		abs, err := filepath.Abs(cwd)
		if err == nil {
			for {
				tryDir(abs)
				parent := filepath.Dir(abs)
				if parent == abs {
					break
				}
				abs = parent
			}
		}
	}

	return files
}
