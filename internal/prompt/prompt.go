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
	Name                   string
	Description            string
	Location               string // file path where the skill was found
	DisableModelInvocation bool   // If true, models should not invoke this skill automatically
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

// piDocsPath returns the path to the pi coding-agent package (matching TS pi-mono).
// Returns empty string if not found.
func piDocsPath() string {
	// Try common installation paths (TS pi-mono uses getPackageDir())
	candidates := []string{
		"/opt/homebrew/lib/node_modules/@earendil-works/pi-coding-agent",
		"/usr/local/lib/node_modules/@earendil-works/pi-coding-agent",
	}
	for _, p := range candidates {
		if info, err := os.Stat(p); err == nil && info.IsDir() {
			return p
		}
	}
	return ""
}

// piDocsSection injects pi-specific documentation references into the system prompt.
// Aligned with TS pi-mono: uses file paths so the LLM can read docs via the read tool.
func piDocsSection() string {
	pkgDir := piDocsPath()
	if pkgDir == "" {
		// Fallback to inline URLs when pi package not found locally
		return `Pi documentation (read only when the user asks about pi itself, its SDK, extensions, themes, skills, or TUI):
- Main documentation: https://pi.dev
- Additional docs: https://pi.dev/docs
- When asked about: extensions, themes, skills, prompt templates, TUI components, keybindings, SDK integrations, custom providers, adding models, pi packages
- When working on pi topics, read the docs and follow cross-references before implementing`
	}
	readmePath := filepath.Join(pkgDir, "README.md")
	docsPath := filepath.Join(pkgDir, "docs")
	examplesPath := filepath.Join(pkgDir, "examples")
	return fmt.Sprintf(`Pi documentation (read only when the user asks about pi itself, its SDK, extensions, themes, skills, or TUI):
- Main documentation: %s
- Additional docs: %s
- Examples: %s (extensions, custom tools, SDK)
- When asked about: extensions (docs/extensions.md, examples/extensions/), themes (docs/themes.md), skills (docs/skills.md), prompt templates (docs/prompt-templates.md), TUI components (docs/tui.md), keybindings (docs/keybindings.md), SDK integrations (docs/sdk.md), custom providers (docs/custom-provider.md), adding models (docs/models.md), pi packages (docs/packages.md)
- When working on pi topics, read the docs and examples, and follow .md cross-references before implementing
- Always read pi .md files completely and follow links to related docs (e.g., tui.md for TUI API details)`,
		readmePath, docsPath, examplesPath)
}

// BuildPrompt produces a fully assembled system prompt from the given options.
// Section ordering matches TS pi-mono's buildSystemPrompt().
func BuildPrompt(opts PromptOptions) string {
	var sections []string

	// 1. Base prompt (identity + tools + guidelines + pi docs)
	if opts.CustomPrompt != "" {
		sections = append(sections, opts.CustomPrompt)
		// Custom prompt replaces default, but we still inject context below
	} else {
		// Build identity section (TS pi-mono aligned)
		sections = append(sections, buildIdentitySection(opts))
	}

	// 2. Append prompt (TS: injected after pi docs, before project context)
	if opts.AppendPrompt != "" {
		sections = append(sections, opts.AppendPrompt)
	}

	// 3. Project context (AGENTS.md / CLAUDE.md)
	// TS format: "# Project Context\n\nProject-specific instructions and guidelines:\n\n## {path}\n\n{content}"
	if opts.AGENTSContent != "" {
		trimmed := strings.TrimSpace(opts.AGENTSContent)
		sections = append(sections, "# Project Context\n\nProject-specific instructions and guidelines:\n\n"+trimmed)
	}

	// 4. Skills XML injection (only if read tool is available)
	if len(opts.Skills) > 0 && hasTool(opts.Tools, "read") {
		// Filter out disableModelInvocation skills (TS pi-mono aligned)
		visibleSkills := filterVisibleSkills(opts.Skills)
		if len(visibleSkills) > 0 {
			sections = append(sections, formatSkillsSection(visibleSkills))
		}
	}

	// 5. Date and working directory injection (always last)
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

	return strings.Join(sections, "\n\n")
}

// buildIdentitySection builds the identity + tools + guidelines + pi docs section.
// Matches TS pi-mono's non-customPrompt path.
func buildIdentitySection(opts PromptOptions) string {
	var parts []string

	// Identity (TS pi-mono aligned)
	identity := `You are an expert coding assistant operating inside pi, a coding agent harness. You help users by reading files, executing commands, editing code, and writing new files.`
	parts = append(parts, identity)

	// Tools (TS: only visible tools with snippets; filtered to tools with snippets)
	toolNames := toolNamesFromOpts(opts.Tools)
	visibleTools := opts.Tools // all tools have descriptions in Go
	toolsList := "(none)"
	if len(visibleTools) > 0 {
		var lines []string
		for _, t := range visibleTools {
			snippet := extractFirstSentence(t.Def.Function.Description)
			lines = append(lines, fmt.Sprintf("- %s: %s", t.Def.Function.Name, snippet))
		}
		toolsList = strings.Join(lines, "\n")
	}
	parts = append(parts, "Available tools:")
	parts = append(parts, toolsList)
	parts = append(parts, "In addition to the tools above, you may have access to other custom tools depending on the project.")

	// Dynamic tool-based guidelines (TS pi-mono aligned)
	dynamicGuidelines := buildDynamicToolGuidelines(toolNames)
	var allGuidelines []string
	allGuidelines = append(allGuidelines, dynamicGuidelines...)
	allGuidelines = append(allGuidelines, opts.PromptGuidelines...)
	for _, t := range opts.Tools {
		allGuidelines = append(allGuidelines, t.Guidelines...)
	}
	// Always include default behavioral guidelines
	allGuidelines = append(allGuidelines, defaultBehavioralGuidelines...)
	deduped := deduplicateGuidelines(allGuidelines)
	if len(deduped) > 0 {
		var guideLines []string
		for _, g := range deduped {
			guideLines = append(guideLines, "- "+g)
		}
		parts = append(parts, "Guidelines:")
		parts = append(parts, strings.Join(guideLines, "\n"))
	}

	// Pi docs section (TS pi-mono aligned)
	parts = append(parts, piDocsSection())

	return strings.Join(parts, "\n\n")
}

// buildDynamicToolGuidelines returns conditional guidelines based on which tools are available.
// Aligned with TS pi-mono: only 2 rules (bash-only + prefer-grep-find-ls).
// Removed Go-specific rules: edit preference and test suggestion.
func buildDynamicToolGuidelines(toolNames []string) []string {
	hasBash := contains(toolNames, "bash")
	hasGrep := contains(toolNames, "grep")
	hasFind := contains(toolNames, "find")
	hasLs := contains(toolNames, "ls")

	var guidelines []string

	if hasBash && !hasGrep && !hasFind && !hasLs {
		guidelines = append(guidelines, "Use bash for file operations like ls, rg, find")
	} else if hasBash && (hasGrep || hasFind || hasLs) {
		guidelines = append(guidelines, "Prefer grep/find/ls tools over bash for file exploration (faster, respects .gitignore)")
	}

	return guidelines
}

// formatSkillsSection formats the skills lead-in + XML block.
// Matches TS pi-mono's formatSkillsForPrompt().
func formatSkillsSection(skills []Skill) string {
	var sb strings.Builder
	sb.WriteString("The following skills provide specialized instructions for specific tasks.\n")
	sb.WriteString("Use the read tool to load a skill's file when the task matches its description.\n")
	sb.WriteString("When a skill file references a relative path, resolve it against the skill directory (parent of SKILL.md / dirname of the path) and use that absolute path in tool commands.\n")
	sb.WriteString("\n")
	sb.WriteString("<available_skills>\n")
	for _, s := range skills {
		sb.WriteString("  <skill>\n")
		sb.WriteString(fmt.Sprintf("    <name>%s</name>\n", escapeXML(s.Name)))
		sb.WriteString(fmt.Sprintf("    <description>%s</description>\n", escapeXML(s.Description)))
		sb.WriteString(fmt.Sprintf("    <location>%s</location>\n", escapeXML(s.Location)))
		sb.WriteString("  </skill>\n")
	}
	sb.WriteString("</available_skills>")
	return sb.String()
}

// filterVisibleSkills filters out skills with DisableModelInvocation=true.
func filterVisibleSkills(skills []Skill) []Skill {
	var visible []Skill
	for _, s := range skills {
		if !s.DisableModelInvocation {
			visible = append(visible, s)
		}
	}
	return visible
}

// toolNamesFromOpts extracts tool names from AgentTool list.
func toolNamesFromOpts(tools []types.AgentTool) []string {
	names := make([]string, len(tools))
	for i, t := range tools {
		names[i] = t.Def.Function.Name
	}
	return names
}

// contains checks if a string slice contains a value.
func contains(slice []string, val string) bool {
	for _, v := range slice {
		if v == val {
			return true
		}
	}
	return false
}

// hasTool checks if a tool with the given name is in the tools list.
func hasTool(tools []types.AgentTool, name string) bool {
	return contains(toolNamesFromOpts(tools), name)
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
