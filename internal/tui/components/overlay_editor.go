package components

import (
	"strings"
	"unicode/utf8"

	tea "github.com/charmbracelet/bubbletea"
)

// isPunct returns true for punctuation characters (word boundary, TS pi-mono style).

func (o *Overlay) updateEditor(ks string, s *overlayState) tea.Cmd {
	// Keybinding-aware dispatch for submit and cancel (TS pi-mono: kb.matches())
	if s.keyMatcher != nil {
		switch {
		case s.keyMatcher(ks, BindSelectConfirm):
			val := strings.Join(s.editorLines, "\n")
			cb := s.onEditorSubmit
			o.Hide()
			if cb != nil {
				cb(val)
			}
			return nil
		case s.keyMatcher(ks, BindSelectCancel):
			cb := s.onEditorCancel
			o.Hide()
			if cb != nil {
				cb()
			}
			return nil
		}
	}

	switch ks {
	case "ctrl+_", "ctrl+/":
		// Undo (TS pi-mono: Ctrl+_ / Ctrl+-)
		s.editorPreferredCol = nil
		if len(s.editorUndoStack) > 0 {
			last := s.editorUndoStack[len(s.editorUndoStack)-1]
			s.editorUndoStack = s.editorUndoStack[:len(s.editorUndoStack)-1]
			s.editorLines = last.lines
			s.editorCursor = last.cursor
			s.editorCol = last.col
			s.lastAction = ""
		}
		return nil
	case "shift+enter", "alt+enter":
		// Insert newline at cursor (TS pi-mono: tui.input.newLine)
		pushEditorUndo(s)
		s.lastAction = ""
		line := s.editorLines[s.editorCursor]
		before := line[:s.editorCol]
		after := line[s.editorCol:]
		s.editorLines[s.editorCursor] = before
		s.editorLines = append(s.editorLines[:s.editorCursor+1], append([]string{after}, s.editorLines[s.editorCursor+1:]...)...)
		s.editorCursor++
		s.editorCol = 0
		return nil
	case "enter":
		val := strings.Join(s.editorLines, "\n")
		cb := s.onEditorSubmit
		o.Hide()
		if cb != nil {
			cb(val)
		}
		return nil
	case "esc":
		cb := s.onEditorCancel
		o.Hide()
		if cb != nil {
			cb()
		}
		return nil
	case "ctrl+g":
		pushEditorUndo(s)
		// Open external editor ($EDITOR or nano/vi) with current content
		if newText := openExternalEditor(strings.Join(s.editorLines, "\n")); newText != "" {
			s.editorLines = strings.Split(newText, "\n")
			if len(s.editorLines) == 0 {
				s.editorLines = []string{""}
			}
			s.editorCursor = len(s.editorLines) - 1
			s.editorCol = len(s.editorLines[s.editorCursor])
		}
		return nil
	case "up":
		s.lastAction = ""
		if s.editorCursor > 0 {
			if s.editorPreferredCol == nil {
				s.editorPreferredCol = &s.editorCol
			}
			s.editorCursor--
			line := s.editorLines[s.editorCursor]
			if *s.editorPreferredCol <= len(line) {
				s.editorCol = *s.editorPreferredCol
				s.editorPreferredCol = nil
			} else {
				s.editorCol = len(line)
			}
		}
		return nil
	case "down":
		s.lastAction = ""
		if s.editorCursor < len(s.editorLines)-1 {
			if s.editorPreferredCol == nil {
				s.editorPreferredCol = &s.editorCol
			}
			s.editorCursor++
			line := s.editorLines[s.editorCursor]
			if *s.editorPreferredCol <= len(line) {
				s.editorCol = *s.editorPreferredCol
				s.editorPreferredCol = nil
			} else {
				s.editorCol = len(line)
			}
		}
		return nil
	case "pgup":
		// Page up: scroll by visibleLines amount (TS pi-mono: pageScroll)
		visibleLines := s.height - 5
		if visibleLines < 1 {
			visibleLines = 1
		}
		for i := 0; i < visibleLines && s.editorCursor > 0; i++ {
			if s.editorPreferredCol == nil {
				s.editorPreferredCol = &s.editorCol
			}
			s.editorCursor--
			line := s.editorLines[s.editorCursor]
			if *s.editorPreferredCol <= len(line) {
				s.editorCol = *s.editorPreferredCol
				s.editorPreferredCol = nil
			} else {
				s.editorCol = len(line)
			}
		}
		return nil
	case "pgdown":
		// Page down: scroll by visibleLines amount (TS pi-mono: pageScroll)
		visibleLines := s.height - 5
		if visibleLines < 1 {
			visibleLines = 1
		}
		for i := 0; i < visibleLines && s.editorCursor < len(s.editorLines)-1; i++ {
			if s.editorPreferredCol == nil {
				s.editorPreferredCol = &s.editorCol
			}
			s.editorCursor++
			line := s.editorLines[s.editorCursor]
			if *s.editorPreferredCol <= len(line) {
				s.editorCol = *s.editorPreferredCol
				s.editorPreferredCol = nil
			} else {
				s.editorCol = len(line)
			}
		}
		return nil
	case "left", "ctrl+b":
		s.lastAction = ""
		s.editorPreferredCol = nil
		if s.editorCol > 0 {
			line := s.editorLines[s.editorCursor]
			s.editorCol = prevRuneStart(line, s.editorCol)
		} else if s.editorCursor > 0 {
			s.editorCursor--
			s.editorCol = len(s.editorLines[s.editorCursor])
		}
		return nil
	case "right", "ctrl+f":
		s.lastAction = ""
		s.editorPreferredCol = nil
		line := s.editorLines[s.editorCursor]
		if s.editorCol < len(line) {
			s.editorCol = nextRuneStart(line, s.editorCol)
		} else if s.editorCursor < len(s.editorLines)-1 {
			s.editorCursor++
			s.editorCol = 0
		}
		return nil
	case "ctrl+left", "alt+left", "alt+b":
		// Move cursor word left (TS pi-mono: cursorWordLeft)
		s.lastAction = ""
		s.editorPreferredCol = nil
		if s.editorCol == 0 {
			if s.editorCursor > 0 {
				s.editorCursor--
				s.editorCol = len(s.editorLines[s.editorCursor])
			}
		} else {
			line := s.editorLines[s.editorCursor]
			pos := s.editorCol
			// Skip trailing whitespace/punctuation
			for pos > 0 {
				r, size := utf8.DecodeLastRuneInString(line[:pos])
				if r != ' ' && !isPunct(r) {
					break
				}
				pos -= size
			}
			// Skip word characters
			for pos > 0 {
				r, size := utf8.DecodeLastRuneInString(line[:pos])
				if r == ' ' || isPunct(r) {
					break
				}
				pos -= size
			}
			s.editorCol = pos
		}
		return nil
	case "ctrl+right", "alt+right", "alt+f":
		// Move cursor word right (TS pi-mono: cursorWordRight)
		s.lastAction = ""
		s.editorPreferredCol = nil
		line := s.editorLines[s.editorCursor]
		if s.editorCol >= len(line) {
			if s.editorCursor < len(s.editorLines)-1 {
				s.editorCursor++
				s.editorCol = 0
			}
			return nil
		}
		pos := s.editorCol
		// Skip word characters
		for pos < len(line) {
			r, size := utf8.DecodeRuneInString(line[pos:])
			if r == ' ' || isPunct(r) {
				break
			}
			pos += size
		}
		// Skip whitespace/punctuation
		for pos < len(line) {
			r, size := utf8.DecodeRuneInString(line[pos:])
			if r != ' ' && !isPunct(r) {
				break
			}
			pos += size
		}
		s.editorCol = pos
		return nil
	case "backspace":
		s.lastAction = ""
		if s.editorCol > 0 {
			pushEditorUndo(s)
			line := s.editorLines[s.editorCursor]
			prevCol := prevRuneStart(line, s.editorCol)
			s.editorLines[s.editorCursor] = line[:prevCol] + line[s.editorCol:]
			s.editorCol = prevCol
		} else if s.editorCursor > 0 {
			pushEditorUndo(s)
			// Merge with previous line
			prevLine := s.editorLines[s.editorCursor-1]
			s.editorCol = len(prevLine)
			s.editorLines[s.editorCursor-1] = prevLine + s.editorLines[s.editorCursor]
			s.editorLines = append(s.editorLines[:s.editorCursor], s.editorLines[s.editorCursor+1:]...)
			s.editorCursor--
		}
		return nil
	case "ctrl+w", "alt+backspace":
		// Delete word backward, save to kill ring (TS pi-mono: deleteWordBackward)
		wasKill := s.lastAction == "kill"
		line := s.editorLines[s.editorCursor]
		if s.editorCol == 0 {
			if s.editorCursor > 0 {
				pushEditorUndo(s)
				s.killRing.Push("\n", true, wasKill)
				s.lastAction = "kill"
				prevLine := s.editorLines[s.editorCursor-1]
				s.editorCol = len(prevLine)
				s.editorLines[s.editorCursor-1] = prevLine + line
				s.editorLines = append(s.editorLines[:s.editorCursor], s.editorLines[s.editorCursor+1:]...)
				s.editorCursor--
			}
		} else {
			pushEditorUndo(s)
			pos := s.editorCol
			// Skip trailing whitespace/punctuation
			for pos > 0 {
				r, size := utf8.DecodeLastRuneInString(line[:pos])
				if r != ' ' && !isPunct(r) {
					break
				}
				pos -= size
			}
			// Skip word characters
			for pos > 0 {
				r, size := utf8.DecodeLastRuneInString(line[:pos])
				if r == ' ' || isPunct(r) {
					break
				}
				pos -= size
			}
			deleted := line[pos:s.editorCol]
			s.killRing.Push(deleted, true, wasKill)
			s.lastAction = "kill"
			s.editorLines[s.editorCursor] = line[:pos] + line[s.editorCol:]
			s.editorCol = pos
		}
		return nil
	case "alt+d", "alt+delete":
		// Delete word forward, save to kill ring (TS pi-mono: deleteWordForward)
		wasKill := s.lastAction == "kill"
		line := s.editorLines[s.editorCursor]
		if s.editorCol >= len(line) {
			if s.editorCursor < len(s.editorLines)-1 {
				pushEditorUndo(s)
				s.killRing.Push("\n", false, wasKill)
				s.lastAction = "kill"
				nextLine := s.editorLines[s.editorCursor+1]
				s.editorLines[s.editorCursor] = line + nextLine
				s.editorLines = append(s.editorLines[:s.editorCursor+1], s.editorLines[s.editorCursor+2:]...)
			}
		} else {
			pushEditorUndo(s)
			pos := s.editorCol
			// Skip word characters
			for pos < len(line) {
				r, size := utf8.DecodeRuneInString(line[pos:])
				if r == ' ' || isPunct(r) {
					break
				}
				pos += size
			}
			// Skip trailing whitespace/punctuation
			for pos < len(line) {
				r, size := utf8.DecodeRuneInString(line[pos:])
				if r != ' ' && !isPunct(r) {
					break
				}
				pos += size
			}
			deleted := line[s.editorCol:pos]
			s.killRing.Push(deleted, false, wasKill)
			s.lastAction = "kill"
			s.editorLines[s.editorCursor] = line[:s.editorCol] + line[pos:]
		}
		return nil
	case "ctrl+u":
		// Delete from start of line to cursor, save to kill ring (TS pi-mono: deleteToLineStart)
		line := s.editorLines[s.editorCursor]
		if s.editorCol == 0 {
			if s.editorCursor > 0 {
				pushEditorUndo(s)
				s.killRing.Push("\n", true, s.lastAction == "kill")
				s.lastAction = "kill"
				prevLine := s.editorLines[s.editorCursor-1]
				s.editorLines[s.editorCursor-1] = prevLine + line
				s.editorLines = append(s.editorLines[:s.editorCursor], s.editorLines[s.editorCursor+1:]...)
				s.editorCursor--
				s.editorCol = len(prevLine)
			}
		} else {
			pushEditorUndo(s)
			deleted := line[:s.editorCol]
			s.killRing.Push(deleted, true, s.lastAction == "kill")
			s.lastAction = "kill"
			s.editorLines[s.editorCursor] = line[s.editorCol:]
			s.editorCol = 0
		}
		return nil
	case "ctrl+k":
		// Delete from cursor to end of line, save to kill ring (TS pi-mono: deleteToLineEnd)
		line := s.editorLines[s.editorCursor]
		if s.editorCol < len(line) {
			pushEditorUndo(s)
			deleted := line[s.editorCol:]
			s.killRing.Push(deleted, false, s.lastAction == "kill")
			s.lastAction = "kill"
			s.editorLines[s.editorCursor] = line[:s.editorCol]
		} else if s.editorCursor < len(s.editorLines)-1 {
			pushEditorUndo(s)
			s.killRing.Push("\n", false, s.lastAction == "kill")
			s.lastAction = "kill"
			s.editorLines[s.editorCursor] = line + s.editorLines[s.editorCursor+1]
			s.editorLines = append(s.editorLines[:s.editorCursor+1], s.editorLines[s.editorCursor+2:]...)
		}
		return nil
	case "ctrl+y":
		// Yank most recent kill ring entry at cursor (TS pi-mono: yank)
		pushEditorUndo(s)
		text := s.killRing.Peek()
		if text == "" {
			return nil
		}
		lines := strings.Split(text, "\n")
		line := s.editorLines[s.editorCursor]
		if len(lines) == 1 {
			// Single line: insert at cursor
			s.editorLines[s.editorCursor] = line[:s.editorCol] + text + line[s.editorCol:]
			s.editorCol += len(text)
		} else {
			// Multi-line insert
			before := line[:s.editorCol]
			after := line[s.editorCol:]
			s.editorLines[s.editorCursor] = before + lines[0]
			// Insert middle lines
			for i := 1; i < len(lines)-1; i++ {
				s.editorLines = append(s.editorLines[:s.editorCursor+i], append([]string{lines[i]}, s.editorLines[s.editorCursor+i:]...)...)
			}
			// Last line merges with text after cursor
			lastIdx := s.editorCursor + len(lines) - 1
			s.editorLines = append(s.editorLines[:lastIdx], append([]string{lines[len(lines)-1] + after}, s.editorLines[lastIdx:]...)...)
			s.editorCursor = lastIdx
			s.editorCol = len(lines[len(lines)-1])
		}
		s.lastAction = "yank"
		return nil
	case "alt+y":
		// Cycle through kill ring (TS pi-mono: yankPop)
		if s.lastAction != "yank" || s.killRing.Len() <= 1 {
			return nil
		}
		pushEditorUndo(s)
		// Delete the previously yanked text
		prevText := s.killRing.Peek()
		yankLines := strings.Split(prevText, "\n")
		if len(yankLines) == 1 {
			// Single line: delete backward from cursor
			if s.editorCol >= len(prevText) {
				line := s.editorLines[s.editorCursor]
				s.editorLines[s.editorCursor] = line[:s.editorCol-len(prevText)] + line[s.editorCol:]
				s.editorCol -= len(prevText)
			}
		} else {
			// Multi-line: remove the yanked lines
			startLine := s.editorCursor - (len(yankLines) - 1)
			if startLine >= 0 {
				startCol := len(s.editorLines[startLine]) - len(yankLines[0])
				afterCursor := s.editorLines[s.editorCursor][s.editorCol:]
				beforeYank := s.editorLines[startLine][:startCol]
				// Replace range with merged line
				newLine := beforeYank + afterCursor
				s.editorLines = append(s.editorLines[:startLine], append([]string{newLine}, s.editorLines[s.editorCursor+1:]...)...)
				s.editorCursor = startLine
				s.editorCol = startCol
			}
		}
		s.killRing.Rotate()
		newText := s.killRing.Peek()
		newLines := strings.Split(newText, "\n")
		line := s.editorLines[s.editorCursor]
		if len(newLines) == 1 {
			s.editorLines[s.editorCursor] = line[:s.editorCol] + newText + line[s.editorCol:]
			s.editorCol += len(newText)
		} else {
			before := line[:s.editorCol]
			after := line[s.editorCol:]
			s.editorLines[s.editorCursor] = before + newLines[0]
			for i := 1; i < len(newLines)-1; i++ {
				s.editorLines = append(s.editorLines[:s.editorCursor+i], append([]string{newLines[i]}, s.editorLines[s.editorCursor+i:]...)...)
			}
			lastIdx := s.editorCursor + len(newLines) - 1
			s.editorLines = append(s.editorLines[:lastIdx], append([]string{newLines[len(newLines)-1] + after}, s.editorLines[lastIdx:]...)...)
			s.editorCursor = lastIdx
			s.editorCol = len(newLines[len(newLines)-1])
		}
		s.lastAction = "yank"
		return nil
	case "delete", "ctrl+d":
		s.lastAction = ""
		line := s.editorLines[s.editorCursor]
		if s.editorCol < len(line) {
			pushEditorUndo(s)
			nextCol := nextRuneStart(line, s.editorCol)
			s.editorLines[s.editorCursor] = line[:s.editorCol] + line[nextCol:]
		} else if s.editorCursor < len(s.editorLines)-1 {
			pushEditorUndo(s)
			// Merge with next line
			s.editorLines[s.editorCursor] = line + s.editorLines[s.editorCursor+1]
			s.editorLines = append(s.editorLines[:s.editorCursor+1], s.editorLines[s.editorCursor+2:]...)
		}
		return nil
	case "home", "ctrl+a":
		s.lastAction = ""
		s.editorPreferredCol = nil
		s.editorCol = 0
		return nil
	case "end", "ctrl+e":
		s.lastAction = ""
		s.editorPreferredCol = nil
		s.editorCol = len(s.editorLines[s.editorCursor])
		return nil
	default:
		if IsPrintableKeyString(ks) {
			// Undo coalescing: space breaks type-word chain (TS pi-mono)
			if ks == " " || s.lastAction != "type-word" {
				pushEditorUndo(s)
				s.lastAction = "type-word"
			}
			line := s.editorLines[s.editorCursor]
			s.editorLines[s.editorCursor] = line[:s.editorCol] + ks + line[s.editorCol:]
			s.editorCol += utf8.RuneCountInString(ks)
		} else if ks == "enter" && false { // disabled: enter submits
		} else if ks == "tab" {
			pushEditorUndo(s)
			line := s.editorLines[s.editorCursor]
			s.editorLines[s.editorCursor] = line[:s.editorCol] + "\t" + line[s.editorCol:]
			s.editorCol++
		}
		return nil
	}
}

// pushEditorUndo saves the current editor state for undo (TS pi-mono: UndoStack).
