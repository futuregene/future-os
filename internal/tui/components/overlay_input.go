package components

import (

	tea "github.com/charmbracelet/bubbletea"
	"unicode/utf8"
)

// isPunct returns true for punctuation characters (word boundary, TS pi-mono style).

func (o *Overlay) updateInput(ks string, s *overlayState) tea.Cmd {
	buf := s.inputBuf.String()

	// Keybinding-aware submit/cancel (TS pi-mono: kb.matches())
	if s.keyMatcher != nil {
		switch {
		case s.keyMatcher(ks, BindSelectConfirm):
			cb := s.onSubmit
			o.Hide()
			if cb != nil {
				cb(buf)
			}
			return nil
		case s.keyMatcher(ks, BindSelectCancel):
			cb := s.onCancel
			o.Hide()
			if cb != nil {
				cb()
			}
			return nil
		}
	}

	switch ks {
	case "enter":
		cb := s.onSubmit
		o.Hide()
		if cb != nil {
			cb(buf)
		}
		return nil
	case "esc":
		cb := s.onCancel
		o.Hide()
		if cb != nil {
			cb()
		}
		return nil
	case "ctrl+_", "ctrl+/":
		// Undo (TS pi-mono: Ctrl+_ / Ctrl+-)
		if len(s.inputUndoStack) > 0 {
			last := s.inputUndoStack[len(s.inputUndoStack)-1]
			s.inputUndoStack = s.inputUndoStack[:len(s.inputUndoStack)-1]
			s.inputBuf.Reset()
			s.inputBuf.WriteString(last.value)
			s.inputCursor = last.cursor
			s.lastAction = ""
		}
		return nil
	case "ctrl+a", "home":
		s.lastAction = ""
		s.inputCursor = 0
		return nil
	case "ctrl+e", "end":
		s.lastAction = ""
		s.inputCursor = len(buf)
		return nil
	case "ctrl+b", "left":
		s.lastAction = ""
		if s.inputCursor > 0 {
			s.inputCursor = prevRuneStart(buf, s.inputCursor)
		}
		return nil
	case "ctrl+f", "right":
		s.lastAction = ""
		if s.inputCursor < len(buf) {
			s.inputCursor = nextRuneStart(buf, s.inputCursor)
		}
		return nil
	case "ctrl+left", "alt+left", "alt+b":
		// Move cursor word left (TS pi-mono: cursorWordLeft)
		if s.inputCursor == 0 {
			return nil
		}
		s.lastAction = ""
		pos := s.inputCursor
		// Skip trailing whitespace/punctuation
		for pos > 0 {
			r, size := utf8.DecodeLastRuneInString(buf[:pos])
			if r != ' ' && !isPunct(r) {
				break
			}
			pos -= size
		}
		// Skip word characters
		for pos > 0 {
			r, size := utf8.DecodeLastRuneInString(buf[:pos])
			if r == ' ' || isPunct(r) {
				break
			}
			pos -= size
		}
		s.inputCursor = pos
		return nil
	case "ctrl+right", "alt+right", "alt+f":
		// Move cursor word right (TS pi-mono: cursorWordRight)
		if s.inputCursor >= len(buf) {
			return nil
		}
		s.lastAction = ""
		pos := s.inputCursor
		// Skip word characters
		for pos < len(buf) {
			r, size := utf8.DecodeRuneInString(buf[pos:])
			if r == ' ' || isPunct(r) {
				break
			}
			pos += size
		}
		// Skip whitespace/punctuation
		for pos < len(buf) {
			r, size := utf8.DecodeRuneInString(buf[pos:])
			if r != ' ' && !isPunct(r) {
				break
			}
			pos += size
		}
		s.inputCursor = pos
		return nil
	case "ctrl+w", "alt+backspace":
		// Delete word backward from cursor, save to kill ring (TS pi-mono: deleteWordBackward)
		if s.inputCursor == 0 {
			return nil
		}
		pushInputUndo(s)
		wasKill := s.lastAction == "kill"
		pos := s.inputCursor
		// Skip trailing whitespace/punctuation
		for pos > 0 {
			r, size := utf8.DecodeLastRuneInString(buf[:pos])
			if r != ' ' && !isPunct(r) {
				break
			}
			pos -= size
		}
		// Skip word characters
		for pos > 0 {
			r, size := utf8.DecodeLastRuneInString(buf[:pos])
			if r == ' ' || isPunct(r) {
				break
			}
			pos -= size
		}
		deleted := buf[pos:s.inputCursor]
		s.killRing.Push(deleted, true, wasKill)
		s.lastAction = "kill"
		s.inputBuf.Reset()
		s.inputBuf.WriteString(buf[:pos] + buf[s.inputCursor:])
		s.inputCursor = pos
		return nil
	case "alt+d", "alt+delete":
		// Delete word forward from cursor, save to kill ring (TS pi-mono: deleteWordForward)
		if s.inputCursor >= len(buf) {
			return nil
		}
		pushInputUndo(s)
		wasKill := s.lastAction == "kill"
		pos := s.inputCursor
		// Skip word characters
		for pos < len(buf) {
			r, size := utf8.DecodeRuneInString(buf[pos:])
			if r == ' ' || isPunct(r) {
				break
			}
			pos += size
		}
		// Skip trailing whitespace/punctuation
		for pos < len(buf) {
			r, size := utf8.DecodeRuneInString(buf[pos:])
			if r != ' ' && !isPunct(r) {
				break
			}
			pos += size
		}
		deleted := buf[s.inputCursor:pos]
		s.killRing.Push(deleted, false, wasKill)
		s.lastAction = "kill"
		s.inputBuf.Reset()
		s.inputBuf.WriteString(buf[:s.inputCursor] + buf[pos:])
		return nil
	case "ctrl+u":
		// Delete from start to cursor, save to kill ring (TS pi-mono: deleteToLineStart)
		if s.inputCursor == 0 {
			return nil
		}
		pushInputUndo(s)
		deleted := buf[:s.inputCursor]
		s.killRing.Push(deleted, true, s.lastAction == "kill")
		s.lastAction = "kill"
		s.inputBuf.Reset()
		s.inputBuf.WriteString(buf[s.inputCursor:])
		s.inputCursor = 0
		return nil
	case "ctrl+k":
		// Delete from cursor to end, save to kill ring (TS pi-mono: deleteToLineEnd)
		if s.inputCursor >= len(buf) {
			return nil
		}
		pushInputUndo(s)
		deleted := buf[s.inputCursor:]
		s.killRing.Push(deleted, false, s.lastAction == "kill")
		s.lastAction = "kill"
		s.inputBuf.Reset()
		s.inputBuf.WriteString(buf[:s.inputCursor])
		return nil
	case "ctrl+y":
		// Yank most recent kill ring entry at cursor (TS pi-mono: yank)
		text := s.killRing.Peek()
		if text == "" {
			return nil
		}
		pushInputUndo(s)
		s.inputBuf.Reset()
		s.inputBuf.WriteString(buf[:s.inputCursor] + text + buf[s.inputCursor:])
		s.inputCursor += len(text)
		s.lastAction = "yank"
		return nil
	case "alt+y":
		// Cycle through kill ring (TS pi-mono: yankPop)
		if s.lastAction != "yank" || s.killRing.Len() <= 1 {
			return nil
		}
		pushInputUndo(s)
		// Delete previously yanked text
		prevText := s.killRing.Peek()
		if s.inputCursor >= len(prevText) {
			prefix := buf[:s.inputCursor-len(prevText)]
			suffix := buf[s.inputCursor:]
			s.killRing.Rotate()
			newText := s.killRing.Peek()
			s.inputBuf.Reset()
			s.inputBuf.WriteString(prefix + newText + suffix)
			s.inputCursor = len(prefix) + len(newText)
		}
		s.lastAction = "yank"
		return nil
	case "backspace":
		if s.inputCursor > 0 {
			pushInputUndo(s)
			s.lastAction = ""
			prevCol := prevRuneStart(buf, s.inputCursor)
			s.inputBuf.Reset()
			s.inputBuf.WriteString(buf[:prevCol] + buf[s.inputCursor:])
			s.inputCursor = prevCol
		}
		return nil
	case "delete", "ctrl+d":
		if s.inputCursor < len(buf) {
			pushInputUndo(s)
			s.lastAction = ""
			nextCol := nextRuneStart(buf, s.inputCursor)
			s.inputBuf.Reset()
			s.inputBuf.WriteString(buf[:s.inputCursor] + buf[nextCol:])
		}
		return nil
	default:
		if IsPrintableKeyString(ks) {
			// Undo coalescing: space breaks type-word chain (TS pi-mono)
			if ks == " " || s.lastAction != "type-word" {
				pushInputUndo(s)
				s.lastAction = "type-word"
			}
			s.inputBuf.Reset()
			s.inputBuf.WriteString(buf[:s.inputCursor] + ks + buf[s.inputCursor:])
			s.inputCursor += utf8.RuneCountInString(ks)
		}
		return nil
	}
}

// pushInputUndo saves the current input state for undo (TS pi-mono: UndoStack).
