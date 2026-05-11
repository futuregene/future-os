package components

import (
	"bytes"
	"os"
	"os/exec"
	"path/filepath"
	"sort"
	"strings"

)

// editorSnapshot captures the editor state for undo.

func extractInsertedText(oldStr, newStr string) string {
	if oldStr == newStr || len(newStr) <= len(oldStr) {
		return ""
	}
	// Find common prefix
	prefixLen := 0
	minLen := len(oldStr)
	if len(newStr) < minLen {
		minLen = len(newStr)
	}
	for i := 0; i < minLen; i++ {
		if oldStr[i] == newStr[i] {
			prefixLen++
		} else {
			break
		}
	}
	// Find common suffix
	oldRemain := oldStr[prefixLen:]
	newRemain := newStr[prefixLen:]
	suffixLen := 0
	for i := 0; i < len(oldRemain) && i < len(newRemain); i++ {
		if oldRemain[len(oldRemain)-1-i] == newRemain[len(newRemain)-1-i] {
			suffixLen++
		} else {
			break
		}
	}
	inserted := newRemain
	if suffixLen > 0 {
		inserted = newRemain[:len(newRemain)-suffixLen]
	}
	return inserted
}

// extractKilledText finds the text removed between oldStr and newStr.
// Uses a simple common-prefix/suffix approach.
func extractKilledText(oldStr, newStr string) string {
	if oldStr == newStr {
		return ""
	}
	// Find common prefix
	minLen := len(oldStr)
	if len(newStr) < minLen {
		minLen = len(newStr)
	}
	prefixLen := 0
	for i := 0; i < minLen; i++ {
		if oldStr[i] == newStr[i] {
			prefixLen++
		} else {
			break
		}
	}
	// Find common suffix
	oldRemain := oldStr[prefixLen:]
	newRemain := newStr[prefixLen:]
	suffixLen := 0
	for i := 0; i < len(oldRemain) && i < len(newRemain); i++ {
		if oldRemain[len(oldRemain)-1-i] == newRemain[len(newRemain)-1-i] {
			suffixLen++
		} else {
			break
		}
	}
	killed := oldRemain
	if suffixLen > 0 {
		killed = oldRemain[:len(oldRemain)-suffixLen]
	}
	return killed
}

// tryFilePathComplete performs file path completion on the last word.
// Uses fd for fuzzy recursive search (TS pi-mono style), falls back to filepath.Glob.
// Multiple matches can be cycled with repeated Tab presses.
func (e *Editor) tryFilePathComplete() {
	val := e.area.Value()
	// Find the last word
	lastSpace := strings.LastIndexAny(val, " \t\n")
	prefix := val
	replaceStart := 0
	if lastSpace >= 0 {
		prefix = val[lastSpace+1:]
		replaceStart = lastSpace + 1
	}
	// Handle @ file attachment syntax (TS pi-mono)
	atPrefix := false
	if strings.HasPrefix(prefix, "@") {
		atPrefix = true
		prefix = prefix[1:]
	}

	// Check if we're cycling through existing matches
	samePrefix := e.fileMatchPrefix == prefix
	if samePrefix && len(e.fileMatches) > 0 {
		e.fileMatchIndex = (e.fileMatchIndex + 1) % len(e.fileMatches)
		match := e.fileMatches[e.fileMatchIndex]
		result := val[:e.fileMatchStart]
		if e.fileMatchAtSign {
			result += "@"
		}
		result += match
		e.area.SetValue(result)
		return
	}

	// Reset match state
	e.fileMatches = nil
	e.fileMatchIndex = -1
	e.fileMatchPrefix = prefix
	e.fileMatchStart = replaceStart
	e.fileMatchAtSign = atPrefix

	// Only complete if it looks like a path, OR if @-prefixed (TS pi-mono: @ triggers file completion)
	if prefix == "" {
		if !atPrefix {
			return
		}
		// @ alone: list files in CWD
		matches := FindFiles(".")
		if len(matches) > 0 {
			e.fileMatches = matches
			e.fileMatchIndex = 0
			match := e.fileMatches[0]
			result := val[:replaceStart] + "@" + match
			e.area.SetValue(result)
		}
		return
	}
	if !atPrefix && !strings.ContainsAny(prefix, "./~") && !strings.HasPrefix(prefix, "/") {
		return
	}

	matches := FindFiles(prefix)
	if len(matches) == 0 {
		return
	}

	// Sort for stable results
	e.fileMatches = matches
	e.fileMatchIndex = 0
	match := e.fileMatches[0]

	result := val[:replaceStart]
	if atPrefix {
		result += "@"
	}
	result += match
	e.area.SetValue(result)
}

// findFiles searches for files matching the given prefix using fd for fuzzy
// recursive search (TS pi-mono style). Falls back to filepath.Glob if fd is
// unavailable. Results are paths relative to CWD (or absolute if prefix is absolute).
func FindFiles(prefix string) []string {
	cwd, err := os.Getwd()
	if err != nil {
		cwd = "."
	}

	// Expand ~
	expanded := prefix
	if strings.HasPrefix(prefix, "~") {
		home, err := os.UserHomeDir()
		if err == nil {
			expanded = filepath.Join(home, prefix[1:])
		}
	}

	isAbs := strings.HasPrefix(expanded, "/")

	// Determine search dir and pattern
	var searchDir, pattern string
	if isAbs {
		searchDir = filepath.Dir(expanded)
		pattern = filepath.Base(expanded)
	} else {
		searchDir = filepath.Join(cwd, filepath.Dir(expanded))
		pattern = filepath.Base(expanded)
	}

	// Try fd first (TS pi-mono: uses fd/fdfind for fuzzy, gitignore-aware search)
	var matches []string
	if matches = tryFd(searchDir, pattern); len(matches) > 0 {
		return formatMatches(matches, searchDir, expanded, prefix, isAbs, cwd)
	}

	// Fallback: filepath.Glob (TS pi-mono: uses readdir/glob as fallback)
	globPattern := expanded + "*"
	globMatches, err := filepath.Glob(globPattern)
	if err != nil || len(globMatches) == 0 {
		return nil
	}

	return formatMatches(globMatches, searchDir, expanded, prefix, isAbs, cwd)
}

// tryFd runs fd/fdfind to find files matching a pattern in a directory.
func tryFd(dir, pattern string) []string {
	bin := "fd"
	if _, err := exec.LookPath("fdfind"); err == nil {
		bin = "fdfind"
	} else if _, err := exec.LookPath("fd"); err != nil {
		return nil
	}

	// Strip leading dot from pattern for fuzzy matching if it's a filename
	// fd does substring matching by default when using -g (glob)
	searchPattern := pattern
	if !strings.HasPrefix(pattern, ".") {
		searchPattern = pattern
	}

	// --glob for glob-like matching, --max-results for limiting,
	// --strip-cwd-prefix for relative paths, --hidden includes dotfiles
	cmd := exec.Command(bin,
		"--glob", searchPattern,
		"--max-results", "20",
		"--strip-cwd-prefix",
		"--color", "never",
		dir,
	)
	var out bytes.Buffer
	cmd.Stdout = &out
	cmd.Stderr = nil

	if err := cmd.Run(); err != nil {
		return nil
	}

	lines := strings.Split(strings.TrimSpace(out.String()), "\n")
	var results []string
	for _, line := range lines {
		line = strings.TrimSpace(line)
		if line != "" {
			results = append(results, line)
		}
	}
	return results
}

// formatMatches converts raw file matches to display-ready strings.
func formatMatches(matches []string, searchDir, expanded, prefix string, isAbs bool, cwd string) []string {
	dedup := make(map[string]bool)
	var result []string
	for _, m := range matches {
		m = strings.TrimSuffix(m, "/")
		// Convert to a path relative to what the user typed
		display := formatMatchPath(m, searchDir, expanded, prefix, isAbs, cwd)
		if display != "" && !dedup[display] {
			dedup[display] = true
			// If directory, add trailing slash
			fullPath := m
			if !isAbs {
				fullPath = filepath.Join(cwd, m)
			}
			if info, err := os.Stat(fullPath); err == nil && info.IsDir() {
				display += "/"
			}
			result = append(result, display)
		}
	}
	sort.Strings(result)
	// Limit to 10 results
	if len(result) > 10 {
		result = result[:10]
	}
	return result
}

// formatMatchPath converts a matched file path to a display string.
func formatMatchPath(match, searchDir, expanded, prefix string, isAbs bool, cwd string) string {
	if isAbs {
		return match
	}
	// If the matched path starts with cwd, make it relative
	if strings.HasPrefix(match, cwd+"/") {
		return match[len(cwd)+1:]
	}
	if strings.HasPrefix(match, cwd) {
		return match[len(cwd):]
	}
	return match
}

// charJump moves the cursor to the next/previous occurrence of target character.
func (e *Editor) charJump(target byte, forward bool) {
	val := e.area.Value()
	if val == "" {
		return
	}
	curLine := e.area.Line()
	curCol := e.area.LineInfo().CharOffset

	// Find byte offset of cursor in val
	lines := strings.Split(val, "\n")
	offset := 0
	for i := 0; i < curLine && i < len(lines); i++ {
		offset += len(lines[i]) + 1 // +1 for newline
	}
	offset += curCol
	if offset > len(val) {
		offset = len(val)
	}

	if forward {
		for i := offset + 1; i < len(val); i++ {
			if val[i] == target {
				e.moveCursorToByte(i)
				return
			}
		}
	} else {
		for i := offset - 1; i >= 0; i-- {
			if val[i] == target {
				e.moveCursorToByte(i)
				return
			}
		}
	}
}

// moveCursorToByte positions the textarea cursor at the given byte offset.
func (e *Editor) moveCursorToByte(pos int) {
	val := e.area.Value()
	targetLine := 0
	targetCol := 0
	for i := 0; i < pos && i < len(val); i++ {
		if val[i] == '\n' {
			targetLine++
			targetCol = 0
		} else {
			targetCol++
		}
	}
	curLine := e.area.Line()
	for curLine < targetLine {
		e.area.CursorDown()
		curLine++
	}
	for curLine > targetLine {
		e.area.CursorUp()
		curLine--
	}
	e.area.SetCursor(targetCol)
}

// pageScroll moves the cursor by one page height (visible editor area).
// direction: -1 for up, 1 for down (TS pi-mono).
func (e *Editor) pageScroll(direction int) {
	height := e.area.Height()
	if height <= 0 {
		height = 5
	}
	targetLine := e.area.Line() + direction*height
	totalLines := max(1, e.area.LineCount())
	if targetLine < 0 {
		targetLine = 0
	}
	if targetLine >= totalLines {
		targetLine = totalLines - 1
	}
	// Use preferred visual column if set, otherwise current column
	targetCol := e.area.LineInfo().CharOffset
	if e.preferredVisualCol != nil {
		targetCol = *e.preferredVisualCol
	}
	// Navigate to target line
	for e.area.Line() < targetLine {
		e.area.CursorDown()
	}
	for e.area.Line() > targetLine {
		e.area.CursorUp()
	}
	e.area.SetCursor(targetCol)
	e.preferredVisualCol = &targetCol
}

// moveWordBackward moves the cursor to the start of the current/previous word.
// Punctuation is treated as a separate category (TS pi-mono: three-category word movement).
// Uses grapheme clusters for correct cursor movement across emoji/combining sequences.
