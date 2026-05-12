package skills

import (
	"fmt"
	"os"
	"path/filepath"
	"regexp"
	"strings"
)

// Skill represents a discovered skill with its metadata and source.
type Skill struct {
	Name                    string // Skill name (kebab-case identifier)
	Description             string // Human-readable description
	Path                    string // Absolute path to the SKILL.md file
	DisableModelInvocation  bool   // If true, models should not invoke this skill automatically
	Source                  string // Source identifier: "user", "project", or a custom path
}

// Predefined skill directories.
// TODO: make configurable via settings.json
const (
	UserSkillsDir     = "~/.xihu/skills/"
	ProjectSkillsDir  = ".xihu/skills/"
	AgentsSkillsDir   = "~/.agents/skills/"
	PiSkillsDir       = "~/.pi/agent/skills/" // transitional: compat with pi skill dirs
)

// nameRegex validates skill names: lowercase letters, digits, single hyphens between segments.
// ^[a-z0-9]+(-[a-z0-9]+)*$ — no consecutive hyphens, no leading/trailing hyphens.
var nameRegex = regexp.MustCompile(`^[a-z0-9]+(-[a-z0-9]+)*$`)

// maxNameLen is the maximum allowed skill name length.
const maxNameLen = 64

// maxDescLen is the maximum allowed skill description length.
const maxDescLen = 1024

// DiscoverSkills walks the given search directories looking for SKILL.md files,
// parses their YAML frontmatter, and returns discovered skills.
// Directories named "node_modules" or ".git" are skipped. Symlinks are followed.
func DiscoverSkills(searchDirs []string, source string) ([]Skill, error) {
	var skills []Skill

	for _, dir := range searchDirs {
		// Expand ~ to user home directory
		dir = expandHome(dir)

		err := filepath.WalkDir(dir, func(path string, d os.DirEntry, err error) error {
			if err != nil {
				// Skip directories/files we cannot access
				return nil
			}

			// Skip node_modules and .git directories
			if d.IsDir() {
				base := filepath.Base(path)
				if base == "node_modules" || base == ".git" {
					return filepath.SkipDir
				}
				return nil
			}

			// Only process SKILL.md files
			if d.Name() != "SKILL.md" {
				return nil
			}

			skill, ok := ParseSkillFile(path, source)
			if ok {
				skills = append(skills, skill)
			}

			return nil
		})

		if err != nil {
			return skills, fmt.Errorf("walking directory %s: %w", dir, err)
		}
	}

	return skills, nil
}

// ParseSkillFile reads a SKILL.md file and extracts frontmatter (YAML between --- markers).
// Returns the Skill and true if parsing succeeded.
func ParseSkillFile(path, source string) (Skill, bool) {
	content, err := os.ReadFile(path)
	if err != nil {
		return Skill{}, false
	}

	text := string(content)

	// Extract frontmatter between --- markers
	name, description, disableModelInvocation := parseFrontmatter(text)

	// Name is required
	if name == "" {
		return Skill{}, false
	}

	return Skill{
		Name:                   name,
		Description:            description,
		Path:                   path,
		DisableModelInvocation: disableModelInvocation,
		Source:                 source,
	}, true
}

// parseFrontmatter extracts YAML frontmatter from markdown content.
// Frontmatter is delimited by --- on its own line at the start of the file.
// Uses simple line-based parsing without external YAML dependencies.
func parseFrontmatter(text string) (name, description string, disableModelInvocation bool) {
	// Check if file starts with ---
	trimmed := strings.TrimLeft(text, "\r\n")
	if !strings.HasPrefix(trimmed, "---") {
		return "", "", false
	}

	// Find closing ---
	rest := trimmed[3:] // skip opening ---
	idx := strings.Index(rest, "\n---")
	if idx == -1 {
		// Try just --- on its own
		idx = strings.Index(rest, "---")
		if idx == -1 {
			return "", "", false
		}
	}

	frontmatter := rest[:idx]

	lines := strings.Split(frontmatter, "\n")
	for _, line := range lines {
		line = strings.TrimSpace(line)
		if line == "" || strings.HasPrefix(line, "#") {
			continue
		}

		// Match "key: value" patterns
		if m := matchKeyValue(line, "name"); m != "" {
			name = m
		} else if m := matchKeyValue(line, "description"); m != "" {
			description = m
		} else if m := matchKeyValue(line, "disable-model-invocation"); m != "" {
			disableModelInvocation = strings.ToLower(m) == "true"
		}
	}

	return name, description, disableModelInvocation
}

// matchKeyValue extracts the value for a given key from a "key: value" line.
// Handles quoted values (single or double quotes). Returns empty string if no match.
func matchKeyValue(line, key string) string {
	prefix := key + ":"
	if !strings.HasPrefix(line, prefix) {
		// Allow for whitespace before the colon
		if !strings.HasPrefix(strings.TrimSpace(line), prefix) {
			return ""
		}
	}

	value := strings.TrimSpace(line[len(prefix):])

	// Strip surrounding quotes
	if len(value) >= 2 {
		if (value[0] == '"' && value[len(value)-1] == '"') ||
			(value[0] == '\'' && value[len(value)-1] == '\'') {
			value = value[1 : len(value)-1]
		}
	}

	return value
}

// ValidateSkill checks that a skill meets all naming and content requirements.
// Returns nil if valid, or an error describing the validation failure.
func ValidateSkill(s Skill) error {
	// Name validation
	if s.Name == "" {
		return fmt.Errorf("skill name is required")
	}

	if len(s.Name) > maxNameLen {
		return fmt.Errorf("skill name too long: %d chars (max %d)", len(s.Name), maxNameLen)
	}

	if !nameRegex.MatchString(s.Name) {
		return fmt.Errorf("invalid skill name %q: must match ^[a-z0-9]+(-[a-z0-9]+)*$", s.Name)
	}

	// Check for consecutive hyphens explicitly (regex should cover this, but be safe)
	if strings.Contains(s.Name, "--") {
		return fmt.Errorf("invalid skill name %q: no consecutive hyphens allowed", s.Name)
	}

	// Description validation
	if s.Description == "" {
		return fmt.Errorf("skill %q: description is required", s.Name)
	}

	if len(s.Description) > maxDescLen {
		return fmt.Errorf("skill %q: description too long: %d chars (max %d)", s.Name, len(s.Description), maxDescLen)
	}

	return nil
}

// FormatSkillsXML formats a list of skills as an XML <available_skills> block
// suitable for injection into a system prompt.
// Uses nested elements format aligned with TS pi-mono's formatSkillsForPrompt().
// Skills with DisableModelInvocation=true are excluded.
func FormatSkillsXML(skills []Skill) string {
	// Filter out disableModelInvocation skills
	visible := make([]Skill, 0, len(skills))
	for _, s := range skills {
		if !s.DisableModelInvocation {
			visible = append(visible, s)
		}
	}
	if len(visible) == 0 {
		return ""
	}

	var sb strings.Builder
	sb.WriteString("<available_skills>\n")
	for _, s := range visible {
		sb.WriteString("  <skill>\n")
		sb.WriteString(fmt.Sprintf("    <name>%s</name>\n", escapeXML(s.Name)))
		sb.WriteString(fmt.Sprintf("    <description>%s</description>\n", escapeXML(s.Description)))
		sb.WriteString(fmt.Sprintf("    <location>%s</location>\n", escapeXML(s.Path)))
		sb.WriteString("  </skill>\n")
	}
	sb.WriteString("</available_skills>")

	return sb.String()
}

// SkillCollision describes a naming conflict between two skills where one was dropped.
type SkillCollision struct {
	Name        string // skill name that collided
	WinnerPath  string // path of the skill that was kept
	LoserPath   string // path of the skill that was skipped
	WinnerSource string // source of the winning skill
	LoserSource  string // source of the losing skill
}

// ResolveCollisions resolves naming conflicts between skills.
// Rules:
//   - Project skills (source="project") take precedence over user skills (source="user")
//   - Within the same source, first-come-first-served (earlier in slice wins)
//   - Other sources are treated as equal priority, first-wins
func ResolveCollisions(skills []Skill) []Skill {
	result, _ := ResolveCollisionsWithDiagnostics(skills)
	return result
}

// ResolveCollisionsWithDiagnostics resolves naming conflicts and returns collisions for reporting.
func ResolveCollisionsWithDiagnostics(skills []Skill) ([]Skill, []SkillCollision) {
	if len(skills) == 0 {
		return nil, nil
	}

	// Group skills by name, tracking the best candidate for each
	type candidate struct {
		skill  Skill
		idx    int
		rank   int // lower = higher priority
	}

	best := make(map[string]candidate)
	var collisions []SkillCollision

	for i, s := range skills {
		rank := sourceRank(s.Source)

		existing, exists := best[s.Name]
		if !exists {
			best[s.Name] = candidate{skill: s, idx: i, rank: rank}
			continue
		}

		// Lower rank wins; tie goes to earlier index
		if rank < existing.rank {
			collisions = append(collisions, SkillCollision{
				Name:         s.Name,
				WinnerPath:   s.Path,
				LoserPath:    existing.skill.Path,
				WinnerSource: s.Source,
				LoserSource:  existing.skill.Source,
			})
			best[s.Name] = candidate{skill: s, idx: i, rank: rank}
		} else {
			collisions = append(collisions, SkillCollision{
				Name:         s.Name,
				WinnerPath:   existing.skill.Path,
				LoserPath:    s.Path,
				WinnerSource: existing.skill.Source,
				LoserSource:  s.Source,
			})
		}
		// If same rank, earlier index (existing) wins — no change needed
	}

	// Build result preserving original order (by first occurrence of each name)
	result := make([]Skill, 0, len(best))
	seen := make(map[string]bool)

	for _, s := range skills {
		if seen[s.Name] {
			continue
		}
		winner := best[s.Name]
		result = append(result, winner.skill)
		seen[s.Name] = true
	}

	return result, collisions
}

// sourceRank returns a numeric rank for a source (lower = higher priority).
func sourceRank(source string) int {
	switch source {
	case "project":
		return 0
	case "user":
		return 1
	default:
		return 2
	}
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

// expandHome expands a leading ~ to the user's home directory.
func expandHome(path string) string {
	if strings.HasPrefix(path, "~/") || path == "~" {
		home, err := os.UserHomeDir()
		if err != nil {
			return path
		}
		if path == "~" {
			return home
		}
		return filepath.Join(home, path[2:])
	}
	return path
}
