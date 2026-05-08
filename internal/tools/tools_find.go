package tools

import (
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"

	"github.com/huichen/cobalt/pkg/types"
)

// FindTool searches for files matching a glob pattern. Uses fd if available
// (falling back to filepath.Walk). Skips .git, node_modules, .DS_Store.
// Returns relative paths, limited to a configurable number of results (default 1000).
func FindTool() types.AgentTool {
	return types.AgentTool{
		Def: types.ToolDef{
			Type: "function",
			Function: types.FunctionDef{
				Name:        "find",
				Description: "Find files matching a glob pattern (e.g. '*.go'). Uses fd if available, falls back to filepath.Walk. Skips .git, node_modules, .DS_Store. Configurable limit (default 1000).",
				Parameters: json.RawMessage(`{
					"type": "object",
					"properties": {
						"pattern": {"type": "string", "description": "Glob pattern to match filenames (e.g. '*.go', 'test_*.py')"},
						"path": {"type": "string", "description": "Directory to start searching from (default: current directory)"},
						"limit": {"type": "integer", "description": "Maximum number of results to return (default: 1000)"}
					},
					"required": ["pattern"]
				}`),
			},
		},
		Guidelines: []string{
			"Use find to locate files by pattern",
		},
		Handler: func(args json.RawMessage) (string, error) {
			var params struct {
				Pattern string `json:"pattern"`
				Path    string `json:"path"`
				Limit   int    `json:"limit"`
			}
			if err := json.Unmarshal(args, &params); err != nil {
				return "", err
			}
			if params.Path == "" {
				params.Path = "."
			}
			if params.Limit <= 0 {
				params.Limit = 1000
			}

			// Try fd first if available
			if fdPath, err := exec.LookPath("fd"); err == nil {
				return findViaFd(fdPath, params)
			}

			return findViaWalk(params)
		},
	}
}

// findViaFd uses the fd utility for fast file searching.
func findViaFd(fdPath string, params struct {
	Pattern string `json:"pattern"`
	Path    string `json:"path"`
	Limit   int    `json:"limit"`
}) (string, error) {
	// Convert glob pattern to fd pattern: fd treats the pattern as a substring
	// match by default. For glob-like patterns like "*.go", we pass --glob.
	// Strip leading ./ or **/ from patterns to match fd's expectations.
	args := []string{
		"--glob", params.Pattern,
		"--max-results", fmt.Sprintf("%d", params.Limit),
		"--strip-cwd-prefix",
		"--search-path", params.Path,
		"--exclude", ".git",
		"--exclude", "node_modules",
	}

	cmd := exec.Command(fdPath, args...)
	out, err := cmd.CombinedOutput()

	exitCode := 0
	if cmd.ProcessState != nil {
		exitCode = cmd.ProcessState.ExitCode()
	}
	// fd exits 1 when no matches
	if exitCode == 1 {
		return fmt.Sprintf("No files matching '%s' found in %s", params.Pattern, params.Path), nil
	}
	if exitCode != 0 {
		// Fall back to walk-based search on fd error
		return findViaWalk(params)
	}
	_ = err

	output := strings.TrimSpace(string(out))
	if output == "" {
		return fmt.Sprintf("No files matching '%s' found in %s", params.Pattern, params.Path), nil
	}

	lines := strings.Split(output, "\n")
	truncated := false
	lineLimit := len(lines)
	if lineLimit > params.Limit {
		lineLimit = params.Limit
		truncated = true
	}

	var sb strings.Builder
	for i := 0; i < lineLimit; i++ {
		sb.WriteString(lines[i])
		sb.WriteByte('\n')
	}

	if truncated {
		sb.WriteString(fmt.Sprintf("... and %d more results\n", len(lines)-params.Limit))
	}

	return sb.String(), nil
}

// findViaWalk uses filepath.Walk as a fallback.
func findViaWalk(params struct {
	Pattern string `json:"pattern"`
	Path    string `json:"path"`
	Limit   int    `json:"limit"`
}) (string, error) {
	var results []string

	err := filepath.Walk(params.Path, func(path string, info os.FileInfo, err error) error {
		if err != nil {
			return nil // skip files we can't access
		}

		base := info.Name()

		// Skip certain directories entirely
		if info.IsDir() {
			switch base {
			case ".git", "node_modules", ".DS_Store":
				return filepath.SkipDir
			}
			return nil
		}

		// Skip .DS_Store files too
		if base == ".DS_Store" {
			return nil
		}

		// Match glob pattern
		matched, matchErr := filepath.Match(params.Pattern, base)
		if matchErr != nil {
			return nil
		}
		if matched {
			// Return relative path
			relPath, relErr := filepath.Rel(params.Path, path)
			if relErr != nil {
				relPath = path
			}
			results = append(results, relPath)
		}

		return nil
	})

	if err != nil {
		return "", fmt.Errorf("walk directory %s: %w", params.Path, err)
	}

	limit := len(results)
	truncated := false
	if limit > params.Limit {
		limit = params.Limit
		truncated = true
	}

	if limit == 0 {
		return fmt.Sprintf("No files matching '%s' found in %s", params.Pattern, params.Path), nil
	}

	var sb strings.Builder
	for i := 0; i < limit; i++ {
		sb.WriteString(results[i])
		sb.WriteByte('\n')
	}

	if truncated {
		sb.WriteString(fmt.Sprintf("... and %d more results\n", len(results)-params.Limit))
	}

	return sb.String(), nil
}
