package components

import (
	"fmt"
	"strings"

	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
)

// isPunct returns true for punctuation characters (word boundary, TS pi-mono style).

func (o *Overlay) updateCustom(ks string, s *overlayState) tea.Cmd {
	// Keybinding-aware dispatch (TS pi-mono: kb.matches())
	if s.keyMatcher != nil {
		switch {
		case s.keyMatcher(ks, BindSelectCancel):
			o.dismissTop()
			return nil
		case s.keyMatcher(ks, BindSelectConfirm), ks == " ":
			if len(s.customButtons) > 0 && s.customSelected >= 0 && s.customSelected < len(s.customButtons) {
				val := s.customButtons[s.customSelected].Value
				cb := s.onCustomChoice
				o.Hide()
				if cb != nil {
					cb(val)
				}
			}
			return nil
		case s.keyMatcher(ks, BindSelectUp):
			if s.customSelected > 0 {
				s.customSelected--
			}
			return nil
		case s.keyMatcher(ks, BindSelectDown):
			if s.customSelected < len(s.customButtons)-1 {
				s.customSelected++
			}
			return nil
			case ks == "home":
				if len(s.customButtons) > 0 {
					s.customSelected = 0
				}
				return nil
			case ks == "end":
				if len(s.customButtons) > 0 {
					s.customSelected = len(s.customButtons) - 1
				}
				return nil
		}
		return nil
	}

	// Fallback: hardcoded key checks
	switch ks {
	case "esc":
		o.dismissTop()
		return nil
	case "enter", " ":
		if len(s.customButtons) > 0 && s.customSelected >= 0 && s.customSelected < len(s.customButtons) {
			val := s.customButtons[s.customSelected].Value
			cb := s.onCustomChoice
			o.Hide()
			if cb != nil {
				cb(val)
			}
		}
		return nil
	case "left", "up":
		if s.customSelected > 0 {
			s.customSelected--
		}
		return nil
	case "right", "down":
		if s.customSelected < len(s.customButtons)-1 {
			s.customSelected++
		}
		return nil
	case "home":
		if len(s.customButtons) > 0 {
			s.customSelected = 0
		}
		return nil
	case "end":
		if len(s.customButtons) > 0 {
			s.customSelected = len(s.customButtons) - 1
		}
		return nil
	}
	return nil
}

// renderCustom renders the custom dialog overlay content.
func (o Overlay) renderCustom(s *overlayState) string {
	titleStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#61afef")).
		Bold(true)
	borderLine := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#61afef")).
		Render(strings.Repeat("─", max(1, s.width)))
	contentStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#abb2bf"))
	btnStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#abb2bf")).
		Padding(0, 2)
	btnSelectedStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#1e1e2e")).
		Background(lipgloss.Color("#61afef")).
		Padding(0, 2)
	hintStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#5c6370"))

	var sb strings.Builder
	title := s.customTitle
	if s.countdownRemaining > 0 {
		title = fmt.Sprintf("%s (%ds)", title, s.countdownRemaining)
	}
	sb.WriteString(titleStyle.Render(title))
	sb.WriteByte('\n')
	sb.WriteString(borderLine)
	sb.WriteByte('\n')
	// Content text (clipped to available height minus title, buttons, and hint)
	if s.customContent != "" {
		lines := strings.Split(s.customContent, "\n")
		maxLines := s.height - 6
		if maxLines < 1 {
			maxLines = 1
		}
		for i, line := range lines {
			if i >= maxLines {
				break
			}
			sb.WriteString(contentStyle.Render(line))
			sb.WriteByte('\n')
		}
	}
	sb.WriteByte('\n')
	// Render buttons
	if len(s.customButtons) > 0 {
		for i, btn := range s.customButtons {
			if i == s.customSelected {
				sb.WriteString(btnSelectedStyle.Render(btn.Label))
			} else {
				sb.WriteString(btnStyle.Render(btn.Label))
			}
			if i < len(s.customButtons)-1 {
				sb.WriteByte(' ')
			}
		}
		sb.WriteByte('\n')
	}
	// Dynamic key hints (TS pi-mono: keyHint/keyText)
	hintSubmit := s.hints.Submit
	if hintSubmit == "" {
		hintSubmit = "Enter"
	}
	hintCancel := s.hints.Cancel
	if hintCancel == "" {
		hintCancel = "Esc"
	}
	sb.WriteString(hintStyle.Render(hintSubmit + " confirm  " + hintCancel + " cancel"))
	sb.WriteByte('\n')
	sb.WriteString(borderLine)
	return sb.String()
}

// dismissTop dismisses the top overlay, calling any cancel/dismiss callbacks.
func (o *Overlay) dismissTop() {
	if len(o.stack) == 0 {
		return
	}
	s := &o.stack[len(o.stack)-1]
	if s.inputMode && s.onCancel != nil {
		s.onCancel()
	}
	if s.editorMode && s.onEditorCancel != nil {
		s.onEditorCancel()
	}
	if s.customMode && s.onCancel != nil {
		s.onCancel()
	}
	if s.onDismiss != nil {
		s.onDismiss()
	}
	o.stack = o.stack[:len(o.stack)-1]
}

// updateInput handles key events for single-line text input overlays.
