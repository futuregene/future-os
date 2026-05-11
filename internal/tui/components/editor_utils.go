package components

import (
	"fmt"
	"strings"
	"unicode"
	"unicode/utf8"

	"github.com/rivo/uniseg"
)

// editorSnapshot captures the editor state for undo.

func (e *Editor) moveWordBackward() {
	val := e.area.Value()
	if val == "" {
		return
	}
	curLine := e.area.Line()
	curCol := e.area.LineInfo().CharOffset

	offset := bytePos(val, curLine, curCol)

	// Skip trailing whitespace
	for offset > 0 {
		cluster, clusterPos, _ := lastGraphemeCluster(val[:offset])
		r, _ := utf8.DecodeRuneInString(cluster)
		if !isWhitespaceRune(r) {
			break
		}
		offset = clusterPos
	}
	if offset == 0 {
		e.moveCursorToByte(0)
		return
	}
	// Check if we're at punctuation — if so, skip the entire punctuation run
	cluster, _, _ := lastGraphemeCluster(val[:offset])
	r, _ := utf8.DecodeRuneInString(cluster)
	if isPunct(r) {
		for offset > 0 {
			cluster, clusterPos, _ := lastGraphemeCluster(val[:offset])
			r, _ := utf8.DecodeRuneInString(cluster)
			if !isPunct(r) {
				break
			}
			offset = clusterPos
		}
	} else {
		// Skip word characters (non-whitespace, non-punctuation)
		for offset > 0 {
			cluster, clusterPos, _ := lastGraphemeCluster(val[:offset])
			r, _ := utf8.DecodeRuneInString(cluster)
			if isWhitespaceRune(r) || isPunct(r) {
				break
			}
			offset = clusterPos
		}
	}
	e.moveCursorToByte(offset)
}

// moveWordForward moves the cursor to the start of the next word.
// Punctuation is treated as a separate category (TS pi-mono: three-category word movement).
// Uses grapheme clusters for correct cursor movement across emoji/combining sequences.
func (e *Editor) moveWordForward() {
	val := e.area.Value()
	if val == "" {
		return
	}
	curLine := e.area.Line()
	curCol := e.area.LineInfo().CharOffset

	offset := bytePos(val, curLine, curCol)

	// Skip leading whitespace
	for offset < len(val) {
		cluster, size := firstGraphemeCluster(val[offset:])
		if size == 0 {
			break
		}
		r, _ := utf8.DecodeRuneInString(cluster)
		if !isWhitespaceRune(r) {
			break
		}
		offset += size
	}
	if offset >= len(val) {
		e.moveCursorToByte(len(val))
		return
	}
	// Check if next char is punctuation — if so, skip the entire run
	cluster, size := firstGraphemeCluster(val[offset:])
	if size == 0 {
		return
	}
	r, _ := utf8.DecodeRuneInString(cluster)
	if isPunct(r) {
		for offset < len(val) {
			cluster, size := firstGraphemeCluster(val[offset:])
			if size == 0 {
				break
			}
			r, _ := utf8.DecodeRuneInString(cluster)
			if !isPunct(r) {
				break
			}
			offset += size
		}
	} else {
		// Skip word characters (non-whitespace, non-punctuation)
		for offset < len(val) {
			cluster, size := firstGraphemeCluster(val[offset:])
			if size == 0 {
				break
			}
			r, _ := utf8.DecodeRuneInString(cluster)
			if isWhitespaceRune(r) || isPunct(r) {
				break
			}
			offset += size
		}
	}
	e.moveCursorToByte(offset)
}

// graphemeBackspace deletes the grapheme cluster immediately before the cursor (TS pi-mono).
// Unlike the textarea default, this correctly handles multi-codepoint emoji sequences.
func (e *Editor) graphemeBackspace() {
	val := e.area.Value()
	if val == "" {
		return
	}
	curLine := e.area.Line()
	curCol := e.area.LineInfo().CharOffset
	offset := bytePos(val, curLine, curCol)
	if offset == 0 {
		return
	}
	cluster, clusterPos, _ := lastGraphemeCluster(val[:offset])
	if cluster == "" {
		return
	}
	e.area.SetValue(val[:clusterPos] + val[offset:])
	e.moveCursorToByte(clusterPos)
}

// deleteCharForward deletes the character at the cursor position (Ctrl+D).
// At end of line, merges with the next line (TS pi-mono: deleteCharForward).
func (e *Editor) deleteCharForward() {
	val := e.area.Value()
	if val == "" {
		return
	}
	offset := bytePos(val, e.area.Line(), e.area.LineInfo().CharOffset)
	if offset >= len(val) {
		return
	}
	_, size := firstGraphemeCluster(val[offset:])
	if size == 0 {
		return
	}
	if val[offset] == '\n' {
		e.area.SetValue(val[:offset] + val[offset+1:])
		e.moveCursorToByte(offset)
	} else {
		e.area.SetValue(val[:offset] + val[offset+size:])
		e.moveCursorToByte(offset)
	}
}

// pushUndo saves a snapshot of the current editor state before a modification.
// typ is "word" (coalesced with previous word insertions), "space" (word boundary),
// or "delete" (always new snapshot).
func (e *Editor) pushUndo(typ string) {
	snap := editorSnapshot{
		value:  e.area.Value(),
		offset: bytePos(e.area.Value(), e.area.Line(), e.area.LineInfo().CharOffset),
	}
	// Coalesce consecutive word-character insertions (fish-style undo).
	// Keep the first snapshot at the start of the word chain.
	if typ == "word" && len(e.undoStack) > 0 && e.lastUndoType == "word" {
		return
	}
	e.undoStack = append(e.undoStack, snap)
	e.lastUndoType = typ
	// Cap at 100 snapshots
	if len(e.undoStack) > 100 {
		e.undoStack = e.undoStack[1:]
	}
}

// undo pops the most recent snapshot and restores the editor state.
func (e *Editor) undo() {
	if len(e.undoStack) == 0 {
		return
	}
	snap := e.undoStack[len(e.undoStack)-1]
	e.undoStack = e.undoStack[:len(e.undoStack)-1]
	e.lastUndoType = "" // reset coalescing
	e.area.SetValue(snap.value)
	if snap.offset <= len(snap.value) {
		e.moveCursorToByte(snap.offset)
	}
}

// bytePos converts a (line, column) position to a byte offset into the string.
func bytePos(val string, line, col int) int {
	curLine := 0
	curCol := 0
	for i := 0; i < len(val); i++ {
		if curLine == line && curCol >= col {
			return i
		}
		if val[i] == '\n' {
			curLine++
			curCol = 0
		} else {
			curCol++
		}
	}
	return len(val)
}

// isWhitespace returns true if b is an ASCII whitespace character.
func isWhitespace(b byte) bool {
	return b == ' ' || b == '\t' || b == '\n' || b == '\r'
}

// isWhitespaceRune returns true if r is a Unicode whitespace character.
func isWhitespaceRune(r rune) bool {
	return unicode.IsSpace(r)
}

// firstGraphemeCluster returns the first grapheme cluster and its byte length in s.
func firstGraphemeCluster(s string) (string, int) {
	gr := uniseg.NewGraphemes(s)
	if gr.Next() {
		c := gr.Str()
		return c, len(c)
	}
	return "", 0
}

// lastGraphemeCluster returns the last grapheme cluster in s, its byte position, and byte length.
func lastGraphemeCluster(s string) (string, int, int) {
	var last string
	var lastFrom, lastTo int
	gr := uniseg.NewGraphemes(s)
	for gr.Next() {
		last = gr.Str()
		lastFrom, lastTo = gr.Positions()
	}
	return last, lastFrom, lastTo - lastFrom
}

// isOnFirstVisualLine returns true when the cursor is at the first character position (TS pi-mono).
func (e *Editor) isOnFirstVisualLine() bool {
	return e.area.Line() == 0 && e.area.LineInfo().CharOffset == 0
}

// isOnLastVisualLine returns true when the cursor is at the last character position (TS pi-mono).
func (e *Editor) isOnLastVisualLine() bool {
	val := e.area.Value()
	lines := strings.Split(val, "\n")
	curLine := e.area.Line()
	if curLine >= len(lines)-1 {
		lastLine := lines[len(lines)-1]
		return e.area.LineInfo().CharOffset >= len(lastLine)
	}
	return false
}

// currentLine returns the text of the current logical line.
func (e *Editor) currentLine() string {
	val := e.area.Value()
	lines := strings.Split(val, "\n")
	cur := e.area.Line()
	if cur < len(lines) {
		return lines[cur]
	}
	return ""
}

// StorePaste stores pasted text and returns a marker like "[paste #1 +123 lines]".
// Returns empty string if the text isn't large enough to warrant a marker.
// TS pi-mono: markers created for pastes >10 lines or >1000 chars.
func (e *Editor) StorePaste(text string) string {
	lineCount := strings.Count(text, "\n") + 1
	charCount := len(text)
	if lineCount <= 10 && charCount <= 1000 {
		return "" // small enough to insert directly
	}
	e.pasteID++
	marker := fmt.Sprintf("[paste #%d +%d lines]", e.pasteID, lineCount)
	if lineCount <= 1 {
		marker = fmt.Sprintf("[paste #%d %d chars]", e.pasteID, charCount)
	}
	e.pasteStore = append(e.pasteStore, text)
	return marker
}

// ExpandPastes replaces paste markers like [paste #1 +123 lines] with stored content.
func (e *Editor) ExpandPastes(text string) string {
	for i, stored := range e.pasteStore {
		marker := fmt.Sprintf("[paste #%d", i+1)
		if idx := strings.Index(text, marker); idx >= 0 {
			// Find end of marker
			end := strings.Index(text[idx:], "]")
			if end >= 0 {
				text = text[:idx] + stored + text[idx+end+1:]
			}
		}
	}
	return text
}

// isModifyingKey returns true for keys that modify the editor text content.
// Used for undo state tracking (TS pi-mono: UndoStack snapshot before mutation).
func (e *Editor) isModifyingKey(ks string) bool {
	if e.matches(ks, BindDeleteCharBackward, "backspace", "shift+backspace") ||
		e.matches(ks, BindDeleteCharForward, "delete", "ctrl+d", "shift+delete") ||
		e.matches(ks, BindDeleteToLineEnd, "ctrl+k") ||
		e.matches(ks, BindDeleteToLineStart, "ctrl+u") ||
		e.matches(ks, BindDeleteWordBackward, "ctrl+w", "alt+backspace") ||
		e.matches(ks, BindDeleteWordForward, "alt+d") ||
		e.matches(ks, BindSubmit, "enter") ||
		e.matches(ks, BindNewLine, "alt+enter") ||
		e.matches(ks, BindTab, "tab") ||
		ks == " " || ks == "space" {
		return true
	}
	if IsPrintableKeyString(ks) {
		return true
	}
	return false
}

// exitHistoryBrowse resets the history browsing state (TS pi-mono).
func (e *Editor) exitHistoryBrowse() {
	if e.historyIndex > 0 {
		e.historyIndex = 0
		e.historyDraft = ""
	}
}

// RecordSubmission saves text to prompt history (TS pi-mono: Up/Down navigation).
func (e *Editor) RecordSubmission(text string) {
	// Avoid consecutive duplicates
	if len(e.history) > 0 && e.history[len(e.history)-1] == text {
		return
	}
	e.history = append(e.history, text)
	e.historyIndex = 0
	e.historyDraft = ""
	// Cap history at 100 entries
	if len(e.history) > 100 {
		e.history = e.history[len(e.history)-100:]
	}
}

func (e *Editor) historyLen() int {
	return len(e.history)
}

// stringSlicesEqual compares two string slices for equality.
func stringSlicesEqual(a, b []string) bool {
	if len(a) != len(b) {
		return false
	}
	for i := range a {
		if a[i] != b[i] {
			return false
		}
	}
	return true
}

// FollowUpMsg is sent on Alt+Enter — queues message for after agent finishes.
type FollowUpMsg string
