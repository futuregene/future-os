package components

import (
	"os"
	"os/exec"
	"strconv"
	"strings"
	"time"

	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
)

// isPunct returns true for punctuation characters (word boundary, TS pi-mono style).

func isPunct(c rune) bool {
	return c == '.' || c == ',' || c == ':' || c == ';' || c == '!' || c == '?' ||
		c == '-' || c == '_' || c == '/' || c == '\\' || c == '(' || c == ')' ||
		c == '[' || c == ']' || c == '{' || c == '}' || c == '<' || c == '>' ||
		c == '"' || c == '\'' || c == '|' || c == '&' || c == '*' || c == '#' ||
		c == '@' || c == '~' || c == '`' || c == '=' || c == '+' || c == '%' ||
		c == '^' || c == '$'
}

// IsPrintableKeyString returns true when ks represents printable text input
// (one or more runes, none of which are control characters). This replaces
// the old ASCII-only check and matches TS pi-mono's full Unicode support.
func IsPrintableKeyString(ks string) bool {
	if len(ks) == 0 {
		return false
	}
	for _, r := range ks {
		if r < 32 || r == 127 {
			return false
		}
	}
	return true
}

// prevRuneStart returns the byte position of the start of the rune before pos.
func prevRuneStart(s string, pos int) int {
	_, bytePos, _ := lastGraphemeCluster(s[:pos])
	return bytePos
}

// nextRuneStart returns the byte position of the start of the rune after pos.
func nextRuneStart(s string, pos int) int {
	_, size := firstGraphemeCluster(s[pos:])
	if size == 0 {
		return pos
	}
	return pos + size
}

// OverlayAnchor specifies the anchor position for overlay placement.
func resolveSize(val string, termSize int) int {
	if strings.HasSuffix(val, "%") {
		pct, err := strconv.ParseFloat(strings.TrimSuffix(val, "%"), 64)
		if err != nil || pct <= 0 || pct > 100 {
			return termSize
		}
		return int(float64(termSize) * pct / 100.0)
	}
	n, err := strconv.Atoi(val)
	if err != nil || n <= 0 {
		return termSize
	}
	return n
}

// Show displays a new overlay with the given text content, pushing onto the stack.
func AnchorToLipgloss(a OverlayAnchor) (h, v lipgloss.Position) {
	return anchorToLipgloss(a)
}

// anchorToLipgloss converts an OverlayAnchor to lipgloss.Position constants (internal).
func anchorToLipgloss(a OverlayAnchor) (h, v lipgloss.Position) {
	switch a {
	case AnchorTopLeft:
		return lipgloss.Left, lipgloss.Top
	case AnchorTopRight:
		return lipgloss.Right, lipgloss.Top
	case AnchorBottomLeft:
		return lipgloss.Left, lipgloss.Bottom
	case AnchorBottomRight:
		return lipgloss.Right, lipgloss.Bottom
	case AnchorTopCenter:
		return lipgloss.Center, lipgloss.Top
	case AnchorBottomCenter:
		return lipgloss.Center, lipgloss.Bottom
	case AnchorLeftCenter:
		return lipgloss.Left, lipgloss.Center
	case AnchorRightCenter:
		return lipgloss.Right, lipgloss.Center
	default: // AnchorCenter
		return lipgloss.Center, lipgloss.Center
	}
}

// overlayHints holds formatted key strings for the overlay's help line.
// Empty strings cause the renderer to fall back to hardcoded defaults.
func pushInputUndo(s *overlayState) {
	s.inputUndoStack = append(s.inputUndoStack, inputUndoState{
		value:  s.inputBuf.String(),
		cursor: s.inputCursor,
	})
	if len(s.inputUndoStack) > 100 {
		s.inputUndoStack = s.inputUndoStack[1:]
	}
}


func pushEditorUndo(s *overlayState) {
	linesCopy := make([]string, len(s.editorLines))
	copy(linesCopy, s.editorLines)
	s.editorUndoStack = append(s.editorUndoStack, editorUndoState{
		lines:  linesCopy,
		cursor: s.editorCursor,
		col:    s.editorCol,
	})
	if len(s.editorUndoStack) > 100 {
		s.editorUndoStack = s.editorUndoStack[1:]
	}
}

func openExternalEditor(currentText string) string {
	editor := os.Getenv("EDITOR")
	if editor == "" {
		editor = os.Getenv("VISUAL")
	}
	if editor == "" {
		editor = "nano"
	}

	f, err := os.CreateTemp("", "xihu-overlay-edit-*.md")
	if err != nil {
		return ""
	}
	defer os.Remove(f.Name())

	if currentText != "" {
		f.WriteString(currentText)
	}
	f.Close()

	cmd := exec.Command(editor, f.Name())
	cmd.Stdin = os.Stdin
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	if err := cmd.Run(); err != nil {
		return ""
	}

	content, err := os.ReadFile(f.Name())
	if err != nil {
		return ""
	}
	return strings.TrimSpace(string(content))
}
func CountdownTick() tea.Cmd {
	return tea.Tick(time.Second, func(t time.Time) tea.Msg {
		return CountdownTickMsg{}
	})
}

// Update handles Bubble Tea messages for the top overlay.
