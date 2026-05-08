package prompt

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

// PromptTemplate represents a reusable prompt template loaded from a markdown file.
type PromptTemplate struct {
	Name        string // Filename without extension
	Description string // First line of the file (stripped of leading #)
	Content     string // Raw content of the .md file
	Source      string // Full path to the .md file
}

// ParseTemplates scans a directory for .md files and returns parsed PromptTemplates.
func ParseTemplates(dir string) ([]PromptTemplate, error) {
	entries, err := os.ReadDir(dir)
	if err != nil {
		return nil, fmt.Errorf("read template dir %s: %w", dir, err)
	}

	var templates []PromptTemplate
	for _, entry := range entries {
		if entry.IsDir() {
			continue
		}
		if !strings.HasSuffix(entry.Name(), ".md") {
			continue
		}

		fullPath := filepath.Join(dir, entry.Name())
		data, err := os.ReadFile(fullPath)
		if err != nil {
			return nil, fmt.Errorf("read template %s: %w", fullPath, err)
		}

		content := string(data)
		name := strings.TrimSuffix(entry.Name(), ".md")

		// Extract description: first non-empty line, strip leading # and whitespace
		description := ""
		lines := strings.Split(content, "\n")
		for _, line := range lines {
			trimmed := strings.TrimSpace(line)
			if trimmed == "" {
				continue
			}
			// Strip leading # markers
			trimmed = strings.TrimLeft(trimmed, "# ")
			trimmed = strings.TrimSpace(trimmed)
			description = trimmed
			break
		}

		templates = append(templates, PromptTemplate{
			Name:        name,
			Description: description,
			Content:     content,
			Source:      fullPath,
		})
	}

	return templates, nil
}

// ExpandTemplate performs variable substitution on a template's Content.
// Supported placeholders:
//   - $1, $2, ... $N — positional arguments
//   - $@ — all arguments joined with spaces
func ExpandTemplate(tmpl PromptTemplate, args ...string) string {
	result := tmpl.Content

	// Replace positional arguments $1, $2, ...
	for i, arg := range args {
		placeholder := fmt.Sprintf("$%d", i+1)
		result = strings.ReplaceAll(result, placeholder, arg)
	}

	// Replace $@ with all args joined
	result = strings.ReplaceAll(result, "$@", strings.Join(args, " "))

	return result
}

// ParseCommandArgs parses a raw input string into individual arguments using
// bash-style tokenization. Supports single quotes, double quotes, and backslash
// escaping.
func ParseCommandArgs(input string) []string {
	var args []string
	var current strings.Builder
	inSingle := false
	inDouble := false
	escaped := false

	for _, r := range input {
		if escaped {
			current.WriteRune(r)
			escaped = false
			continue
		}

		switch {
		case r == '\\' && !inSingle:
			escaped = true
		case r == '\'' && !inDouble:
			inSingle = !inSingle
		case r == '"' && !inSingle:
			inDouble = !inDouble
		case r == ' ' || r == '\t':
			if inSingle || inDouble {
				current.WriteRune(r)
			} else if current.Len() > 0 {
				args = append(args, current.String())
				current.Reset()
			}
		default:
			current.WriteRune(r)
		}
	}

	if current.Len() > 0 {
		args = append(args, current.String())
	}

	return args
}
