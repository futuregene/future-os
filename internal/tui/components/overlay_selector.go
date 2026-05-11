package components

import (
	"strings"

	tea "github.com/charmbracelet/bubbletea"
)

// isPunct returns true for punctuation characters (word boundary, TS pi-mono style).

func (o *Overlay) updateSelector(ks string, s *overlayState) tea.Cmd {
	// Custom key handler takes priority
	if s.onKey != nil && s.onKey(ks) {
		return nil
	}

	// Keybinding-aware dispatch (TS pi-mono: kb.matches())
	if s.keyMatcher != nil {
		switch {
		case s.keyMatcher(ks, BindSelectCancel):
			o.dismissTop()
			return nil
		case s.keyMatcher(ks, BindSelectUp):
			s.selector.MoveUp()
			if s.onSelectionChange != nil {
				s.onSelectionChange(s.selector.Selected)
			}
			return nil
		case s.keyMatcher(ks, BindSelectDown):
			s.selector.MoveDown()
			if s.onSelectionChange != nil {
				s.onSelectionChange(s.selector.Selected)
			}
			return nil
		case s.keyMatcher(ks, BindSelectPageUp):
			s.selector.PageUp(max(1, s.selector.Height/2))
			if s.onSelectionChange != nil {
				s.onSelectionChange(s.selector.Selected)
			}
			return nil
		case s.keyMatcher(ks, BindSelectPageDn):
			s.selector.PageDown(max(1, s.selector.Height/2))
			if s.onSelectionChange != nil {
				s.onSelectionChange(s.selector.Selected)
			}
			return nil
		case s.keyMatcher(ks, BindSelectConfirm), ks == " ":
			val := ""
			if sel := s.selector.SelectedItem(); sel != nil {
				val = sel.Value
			}
			cb := s.onSelect
			if !s.stayOnSelect {
				o.Hide()
			}
			if cb != nil {
				cb(val)
			}
			return nil
		}
		// Filter handling (keys not mapped to bindings)
		if ks == "backspace" {
			if len(s.selector.Filter) > 0 {
				s.selector.Filter = s.selector.Filter[:len(s.selector.Filter)-1]
				s.selector.Selected = 0
			}
			return nil
		}
		if IsPrintableKeyString(ks) {
			s.selector.ApplyFilter(s.selector.Filter + ks)
			s.selector.Selected = 0
			return nil
		}
		return nil
	}

	// Fallback: hardcoded key checks
	switch ks {
	case "esc":
		o.dismissTop()
		return nil
	case "up", "ctrl+k", "ctrl+p":
		s.selector.MoveUp()
		if s.onSelectionChange != nil {
			s.onSelectionChange(s.selector.Selected)
		}
		return nil
	case "down", "ctrl+j", "ctrl+n":
		s.selector.MoveDown()
		if s.onSelectionChange != nil {
			s.onSelectionChange(s.selector.Selected)
		}
		return nil
	case "pgup":
		s.selector.PageUp(max(1, s.selector.Height/2))
		if s.onSelectionChange != nil {
			s.onSelectionChange(s.selector.Selected)
		}
		return nil
	case "pgdown":
		s.selector.PageDown(max(1, s.selector.Height/2))
		if s.onSelectionChange != nil {
			s.onSelectionChange(s.selector.Selected)
		}
		return nil
	case "enter", " ":
		val := ""
		if sel := s.selector.SelectedItem(); sel != nil {
			val = sel.Value
		}
		cb := s.onSelect
		if !s.stayOnSelect {
			o.Hide()
		}
		if cb != nil {
			cb(val)
		}
		return nil
	case "backspace":
		if len(s.selector.Filter) > 0 {
			s.selector.Filter = s.selector.Filter[:len(s.selector.Filter)-1]
			s.selector.Selected = 0
		}
		return nil
	default:
		// Single printable character = add to filter
		if IsPrintableKeyString(ks) {
			s.selector.ApplyFilter(s.selector.Filter + ks)
			s.selector.Selected = 0
			return nil
		}
		return nil
	}
}

// updateText handles key events for scrollable text overlays.
func (o *Overlay) updateText(ks string, s *overlayState) tea.Cmd {
	switch ks {
	case "esc":
		o.dismissTop()
		return nil
	case "enter", " ":
		o.Hide()
		return func() tea.Msg { return OverlayConfirmMsg{} }
	case "up", "ctrl+k", "ctrl+p":
		if s.scrollOffset > 0 {
			s.scrollOffset--
		}
		return nil
	case "down", "ctrl+j", "ctrl+n":
		s.scrollOffset++
		return nil
	case "pgup":
		visibleLines := s.height - 4
		if visibleLines < 1 {
			visibleLines = 1
		}
		s.scrollOffset -= visibleLines
		if s.scrollOffset < 0 {
			s.scrollOffset = 0
		}
		return nil
	case "pgdown":
		visibleLines := s.height - 4
		if visibleLines < 1 {
			visibleLines = 1
		}
		s.scrollOffset += visibleLines
		return nil
	case "home":
		s.scrollOffset = 0
		return nil
	case "end":
		lines := strings.Split(s.content, "\n")
		visibleLines := s.height - 4
		if visibleLines < 1 {
			visibleLines = 1
		}
		s.scrollOffset = len(lines) - visibleLines
		if s.scrollOffset < 0 {
			s.scrollOffset = 0
		}
		return nil
	}
	return nil
}

// View renders the topmost overlay content (styled, without placement).
// Placement is done by app.go which has terminal dimensions.
