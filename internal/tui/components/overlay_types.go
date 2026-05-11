package components

import (
	"strings"

	"github.com/charmbracelet/lipgloss"
)

// isPunct returns true for punctuation characters (word boundary, TS pi-mono style).

type OverlayAnchor string

const (
	AnchorCenter       OverlayAnchor = "center"
	AnchorTopLeft      OverlayAnchor = "top-left"
	AnchorTopRight     OverlayAnchor = "top-right"
	AnchorBottomLeft   OverlayAnchor = "bottom-left"
	AnchorBottomRight  OverlayAnchor = "bottom-right"
	AnchorTopCenter    OverlayAnchor = "top-center"
	AnchorBottomCenter OverlayAnchor = "bottom-center"
	AnchorLeftCenter   OverlayAnchor = "left-center"
	AnchorRightCenter  OverlayAnchor = "right-center"
)

// AnchorToLipgloss converts an OverlayAnchor to lipgloss.Position constants (public).
type overlayHints struct {
	Submit         string // e.g. "enter"
	Cancel         string // e.g. "esc"
	ExternalEditor string // e.g. "ctrl+g"
}

// overlayState holds a single overlay's full state for stack management.
type overlayState struct {
	content  string
	width    int
	height   int
	style    lipgloss.Style
	selector *ListSelector
	onSelect func(value string)
	onKey    func(key string) bool
	onSelectionChange func(index int) // called when selected index changes (TS pi-mono: onSelectionChange)
	anchor   OverlayAnchor
	termW    int // terminal width, used for percentage sizing
	termH    int // terminal height, used for percentage sizing
	stayOnSelect bool // if true, Enter does not auto-pop (used for navigation overlays)
	scrollOffset int  // scroll position for text overlays
	nonCapturing bool // if true, overlay renders but doesn't steal keyboard focus
		margin       int  // margin from anchor edge in rows/cols (TS pi-mono: OverlayOptions.margin)
		offsetX      int  // horizontal pixel/column offset from anchor (TS pi-mono: OverlayOptions.offsetX)
		offsetY      int  // vertical pixel/row offset from anchor (TS pi-mono: OverlayOptions.offsetY)
		visible      func(w, h int) bool // dynamic visibility callback (TS pi-mono: OverlayOptions.visible)
		hints        overlayHints // dynamic key hints (TS pi-mono: keyHint/keyText helpers)
		keyMatcher   func(keyString, bindingID string) bool // keybinding-aware dispatch

	// Input mode (single-line text input)
	inputMode  bool
	inputBuf   strings.Builder
	inputTitle string
		inputCursor int // cursor position in inputBuf
	onSubmit   func(value string)
	onCancel   func()

	// Editor mode (multi-line text area)
	editorMode          bool
	editorLines         []string
	editorCursor        int
	editorCol           int
	editorTitle         string
	editorPreferredCol  *int // sticky column for vertical navigation (TS pi-mono)
	onEditorSubmit      func(value string)
	onEditorCancel      func()

	// General dismiss callback (called for any overlay type on esc or countdown expiry)
	onDismiss func()

	// Custom dialog mode
	customMode      bool
	customTitle     string
	customContent   string
	customButtons   []CustomButton
	customSelected  int
	onCustomChoice  func(value string)

	// Countdown timer for auto-dismiss (extension dialogs)
	countdownRemaining int // seconds remaining, 0 = no countdown
	countdownExpired   bool

	// Kill ring for Emacs-style kill/yank (TS pi-mono: KillRing)
	killRing   *KillRing
	lastAction string // "kill", "yank", "type-word", "" — yank-pop chain detection

	// Undo stacks (TS pi-mono: UndoStack)
	inputUndoStack  []inputUndoState
	editorUndoStack []editorUndoState
}

// inputUndoState captures overlay input state for undo (TS pi-mono: UndoStack).
type inputUndoState struct {
	value  string
	cursor int
}

// editorUndoState captures overlay editor state for undo (TS pi-mono: UndoStack).
type editorUndoState struct {
	lines  []string
	cursor int
	col    int
}

// CustomButton represents a button in a custom overlay dialog.
type CustomButton struct {
	Label string
	Value string
}

// Overlay manages modal selectors displayed on top of the main content.
// Uses a stack for nested overlays (TS pi-mono style): pushing a new overlay
// preserves the previous one; popping restores it.
type Overlay struct {
	stack          []overlayState
	defHints       overlayHints // default hints inherited by new interactive overlays
	defSelHelp     string       // default help text for new selector overlays
	defKeyMatcher  func(keyString, bindingID string) bool // default key matcher inherited by new overlays
}

// NewOverlay creates a new overlay manager.
