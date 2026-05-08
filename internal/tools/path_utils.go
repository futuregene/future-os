package tools

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

// ResolvePath resolves a relative path against baseDir, preventing directory
// traversal attacks. The returned path is guaranteed to be within baseDir.
//
// If relPath is absolute, it must already be within baseDir.
func ResolvePath(baseDir, relPath string) (string, error) {
	// Clean both paths for consistent comparison
	baseDir = filepath.Clean(baseDir)
	absBase, err := filepath.Abs(baseDir)
	if err != nil {
		return "", fmt.Errorf("resolve base dir: %w", err)
	}

	var resolved string
	if filepath.IsAbs(relPath) {
		resolved = filepath.Clean(relPath)
	} else {
		resolved = filepath.Join(absBase, relPath)
	}
	resolved = filepath.Clean(resolved)

	if !IsWithin(absBase, resolved) {
		return "", fmt.Errorf("path %q escapes base directory %q", relPath, baseDir)
	}

	return resolved, nil
}

// IsWithin reports whether path is equal to or a descendant of baseDir.
// Both arguments are cleaned and made absolute before comparison.
func IsWithin(baseDir, path string) bool {
	baseDir = filepath.Clean(baseDir)
	path = filepath.Clean(path)

	// Ensure both are absolute for reliable prefix matching
	absBase, err := filepath.Abs(baseDir)
	if err != nil {
		return false
	}
	absPath, err := filepath.Abs(path)
	if err != nil {
		return false
	}

	// Exact match
	if absPath == absBase {
		return true
	}

	// Must be a descendant: prefix + separator
	rel, err := filepath.Rel(absBase, absPath)
	if err != nil {
		return false
	}
	// Reject paths that escape via ".."
	return !strings.HasPrefix(rel, "..") && !os.IsPathSeparator(rel[0]) && rel != "."
}
