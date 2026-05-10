// Package utils provides shared utility functions for xihu.
package utils

import (
	"os"
	"strconv"
	"strings"
)

// ChangelogEntry represents a single version entry from CHANGELOG.md.
type ChangelogEntry struct {
	Major   int
	Minor   int
	Patch   int
	Content string
}

// ParseChangelog parses CHANGELOG.md and returns version entries.
// Scans for ## [x.y.z] headers and collects content until the next header.
func ParseChangelog(path string) ([]ChangelogEntry, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		if os.IsNotExist(err) {
			return nil, nil
		}
		return nil, err
	}

	lines := strings.Split(string(data), "\n")
	var entries []ChangelogEntry
	var currentLines []string
	var currentVersion *changelogVersion

	for _, line := range lines {
		if strings.HasPrefix(line, "## ") {
			// Save previous entry
			if currentVersion != nil && len(currentLines) > 0 {
				entries = append(entries, ChangelogEntry{
					Major:   currentVersion.major,
					Minor:   currentVersion.minor,
					Patch:   currentVersion.patch,
					Content: strings.TrimSpace(strings.Join(currentLines, "\n")),
				})
			}

			// Try to parse version from this line
			if v := parseVersion(line); v != nil {
				currentVersion = v
				currentLines = []string{line}
			} else {
				currentVersion = nil
				currentLines = nil
			}
		} else if currentVersion != nil {
			currentLines = append(currentLines, line)
		}
	}

	// Save last entry
	if currentVersion != nil && len(currentLines) > 0 {
		entries = append(entries, ChangelogEntry{
			Major:   currentVersion.major,
			Minor:   currentVersion.minor,
			Patch:   currentVersion.patch,
			Content: strings.TrimSpace(strings.Join(currentLines, "\n")),
		})
	}

	return entries, nil
}

type changelogVersion struct {
	major, minor, patch int
}

// parseVersion extracts a version from a markdown header line like "## [1.2.3]" or "## 1.2.3".
func parseVersion(line string) *changelogVersion {
	// Match "## [x.y.z]" or "## x.y.z"
	rest := strings.TrimPrefix(line, "## ")
	rest = strings.TrimPrefix(rest, "[")
	idx := strings.Index(rest, "]")
	if idx > 0 {
		rest = rest[:idx]
	}
	// Split on space in case of "## 1.2.3 - Title"
	if spaceIdx := strings.Index(rest, " "); spaceIdx > 0 {
		rest = rest[:spaceIdx]
	}
	parts := strings.Split(rest, ".")
	if len(parts) != 3 {
		return nil
	}
	major, err1 := strconv.Atoi(parts[0])
	minor, err2 := strconv.Atoi(parts[1])
	patch, err3 := strconv.Atoi(parts[2])
	if err1 != nil || err2 != nil || err3 != nil {
		return nil
	}
	return &changelogVersion{major, minor, patch}
}

// compareVersion returns -1 if a < b, 0 if a == b, 1 if a > b.
func compareVersion(a, b ChangelogEntry) int {
	if a.Major != b.Major {
		return a.Major - b.Major
	}
	if a.Minor != b.Minor {
		return a.Minor - b.Minor
	}
	return a.Patch - b.Patch
}

// GetNewEntries returns entries newer than the given lastVersion string.
func GetNewEntries(entries []ChangelogEntry, lastVersion string) []ChangelogEntry {
	if lastVersion == "" {
		// No last version recorded - return all entries (first run)
		return entries
	}
	parts := strings.Split(lastVersion, ".")
	if len(parts) != 3 {
		return entries
	}
	major, err1 := strconv.Atoi(parts[0])
	minor, err2 := strconv.Atoi(parts[1])
	patch, err3 := strconv.Atoi(parts[2])
	if err1 != nil || err2 != nil || err3 != nil {
		return entries
	}
	last := ChangelogEntry{Major: major, Minor: minor, Patch: patch}

	var newEntries []ChangelogEntry
	for _, e := range entries {
		if compareVersion(e, last) > 0 {
			newEntries = append(newEntries, e)
		}
	}
	return newEntries
}

// ChangelogPath returns the path to CHANGELOG.md relative to the executable or cwd.
func ChangelogPath() string {
	// Check alongside the executable first
	if exe, err := os.Executable(); err == nil {
		dir := exe
		if idx := strings.LastIndex(dir, "/"); idx >= 0 {
			dir = dir[:idx]
		}
		candidate := dir + "/CHANGELOG.md"
		if _, err := os.Stat(candidate); err == nil {
			return candidate
		}
	}
	// Fall back to CWD
	if _, err := os.Stat("CHANGELOG.md"); err == nil {
		return "CHANGELOG.md"
	}
	return ""
}
