package components

import (
	"fmt"
	"os"
	"os/exec"
	"strconv"
	"strings"
	"time"
	"unicode/utf8"

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
func NewOverlay() Overlay {
	return Overlay{}
}

// SetDefaultHints sets the default key hints for new interactive overlays.
// These are inherited by ShowInput, ShowEditor, and ShowCustom.
// submit, cancel, extEditor correspond to InputSubmit, SelectCancel, GlobalExternalEditor.
func (o *Overlay) SetDefaultHints(submit, cancel, extEditor string) {
	o.defHints = overlayHints{Submit: submit, Cancel: cancel, ExternalEditor: extEditor}
}

// SetDefaultKeyMatcher sets the default keybinding matcher inherited by all new overlays.
// This enables keybinding-aware dispatch in selectors, custom dialogs, and text overlays.
func (o *Overlay) SetDefaultKeyMatcher(fn func(keyString, bindingID string) bool) {
	o.defKeyMatcher = fn
}

// SetDefaultSelectorHelp sets the default help text for new selector overlays.
// Use this to show keybinding-aware hints (TS pi-mono: keyHint/keyText).
// Example: "↑↓ navigate  Enter select  Esc cancel  / filter"
func (o *Overlay) SetDefaultSelectorHelp(text string) {
	o.defSelHelp = text
}

// SetHelpText updates the help text on the currently displayed selector overlay.
// This is used for dynamic hints like delete confirmation (TS pi-mono: confirmingDeletePath).
func (o *Overlay) SetHelpText(text string) {
	if len(o.stack) > 0 {
		top := &o.stack[len(o.stack)-1]
		if top.selector != nil {
			top.selector.HelpText = text
		}
	}
}

// SetSelectionInfoFunc sets the selection info callback on the current selector overlay.
// The function receives the selected index and item, and returns an info line string.
// Used for "Model Name: GPT-4o" display in model selector (TS pi-mono: ModelSelectorComponent).
func (o *Overlay) SetSelectionInfoFunc(fn func(index int, item SelectorItem) string) {
	if len(o.stack) > 0 {
		top := &o.stack[len(o.stack)-1]
		if top.selector != nil {
			top.selector.SelectionInfoFunc = fn
		}
	}
}

// SetNoMatchText sets the no-match message on the current selector overlay.
func (o *Overlay) SetNoMatchText(text string) {
	if len(o.stack) > 0 {
		top := &o.stack[len(o.stack)-1]
		if top.selector != nil {
			top.selector.NoMatchText = text
		}
	}
}

// RefreshItems updates the items on the current selector without hiding/reopening.
// Preserves the selected index (clamped to new range). Use this to avoid the visual
// stutter of closing and reopening the overlay when items change (TS pi-mono: SettingsList.updateValue).
func (o *Overlay) RefreshItems(items []SelectorItem) {
	if len(o.stack) > 0 && o.stack[len(o.stack)-1].selector != nil {
		sel := o.stack[len(o.stack)-1].selector
		oldIdx := sel.Selected
		sel.SetItems(items)
		if oldIdx < len(items) {
			sel.Selected = oldIdx
		}
	}
}

// Active returns whether an overlay is currently displayed.
func (o *Overlay) Active() bool {
	return len(o.stack) > 0
}

// Depth returns the number of overlays in the stack.
func (o *Overlay) Depth() int {
	return len(o.stack)
}

// SetTermSize sets the terminal dimensions used for percentage-based sizing.
func (o *Overlay) SetTermSize(w, h int) {
	for i := range o.stack {
		o.stack[i].termW = w
		o.stack[i].termH = h
	}
}

// NonCapturing returns true if the top overlay is non-capturing (renders but doesn't steal focus).
func (o *Overlay) NonCapturing() bool {
	s := o.top()
	return s != nil && s.nonCapturing
}

// Anchor returns the anchor position of the top overlay, or AnchorCenter if none.
func (o *Overlay) Anchor() OverlayAnchor {
	s := o.top()
	if s == nil {
		return AnchorCenter
	}
	return s.anchor
}

// resolveSize computes an overlay dimension from a string value.
// Integers are used directly; percentage strings (e.g. "80%") are computed from the terminal size.
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
func (o *Overlay) Show(content string, w, h int) {
	o.ShowAnchored(content, w, h, AnchorCenter)
}

// ShowNonCapturingText displays a text overlay that doesn't steal keyboard focus.
// Useful for transient notifications like changelog or update banners.
func (o *Overlay) ShowNonCapturingText(content string, w, h int) {
	s := overlayState{
		content:      content,
		width:        w,
		height:       h,
		nonCapturing: true,
		style: lipgloss.NewStyle().
			Padding(0, 1).
			Background(lipgloss.Color("#282c34")),
		anchor: AnchorTopCenter,
	}
	o.stack = append(o.stack, s)
}

// ShowAnchored displays a text overlay at the specified anchor position.
func (o *Overlay) ShowAnchored(content string, w, h int, anchor OverlayAnchor) {
	s := overlayState{
		content: content,
		width:   w,
		height:  h,
		style: lipgloss.NewStyle().
			Padding(1, 2).
			Background(lipgloss.Color("#282c34")),
		anchor: anchor,
	}
	o.stack = append(o.stack, s)
}

// ShowScrollableText displays a text overlay that supports scrolling (pgup/pgdn/up/down).
// The content can be longer than the overlay height; the user scrolls to view it.
func (o *Overlay) ShowScrollableText(content string, w, h int) {
	o.ShowScrollableTextAnchored(content, w, h, AnchorCenter)
}

// ShowScrollableTextAnchored displays a scrollable text overlay at the specified anchor.
func (o *Overlay) ShowScrollableTextAnchored(content string, w, h int, anchor OverlayAnchor) {
	s := overlayState{
		content: content,
		width:   w,
		height:  h,
		style: lipgloss.NewStyle().
			Padding(1, 2).
			Background(lipgloss.Color("#282c34")),
		anchor:      anchor,
		scrollOffset: 0,
		keyMatcher:   o.defKeyMatcher,
	}
	o.stack = append(o.stack, s)
}

// ShowSelector displays a ListSelector in the overlay, pushing onto the stack.
func (o *Overlay) ShowSelector(title string, items []SelectorItem, onSelect func(value string), w, h int) {
	o.ShowSelectorAnchored(title, items, onSelect, nil, w, h, AnchorCenter)
}

// ShowSelectorWithKeyHandler displays a ListSelector with an optional custom key handler.
func (o *Overlay) ShowSelectorWithKeyHandler(title string, items []SelectorItem, onSelect func(value string), onKey func(key string) bool, w, h int) {
	o.ShowSelectorAnchored(title, items, onSelect, onKey, w, h, AnchorCenter)
}

// ShowSelectorStayOnSelect displays a ListSelector that stays visible after Enter.
// The callback is invoked but the overlay is NOT popped, so the callback can
// push a sub-overlay for nested navigation (e.g. settings → thinking selector).
func (o *Overlay) ShowSelectorStayOnSelect(title string, items []SelectorItem, onSelect func(value string), onKey func(key string) bool, w, h int) {
	sel := NewListSelector(title, items)
	sel.Width = w - 6
	sel.Height = h - 4
	if o.defSelHelp != "" {
		sel.HelpText = o.defSelHelp
	}
	s := overlayState{
		selector:      &sel,
		onSelect:      onSelect,
		onKey:         onKey,
		width:         w,
		height:        h,
		stayOnSelect:  true,
		anchor:        AnchorCenter,
		style: lipgloss.NewStyle().
			Padding(1, 2).
			Background(lipgloss.Color("#282c34")),
	}
	o.stack = append(o.stack, s)
}

// ShowSelectorAnchored displays a ListSelector at the specified anchor position.
func (o *Overlay) ShowSelectorAnchored(title string, items []SelectorItem, onSelect func(value string), onKey func(key string) bool, w, h int, anchor OverlayAnchor) {
	sel := NewListSelector(title, items)
	sel.Width = w - 6
	sel.Height = h - 4
	if o.defSelHelp != "" {
		sel.HelpText = o.defSelHelp
	}
	s := overlayState{
		selector: &sel,
		onSelect: onSelect,
		onKey:    onKey,
		width:    w,
		height:   h,
		anchor:   anchor,
		keyMatcher: o.defKeyMatcher,
		style: lipgloss.NewStyle().
			Padding(1, 2).
			Background(lipgloss.Color("#282c34")),
	}
	o.stack = append(o.stack, s)
}

// ShowSettingsList displays a two-column settings selector (TS pi-mono: SettingsList).
func (o *Overlay) ShowSettingsList(title string, items []SelectorItem, onSelect func(value string), onKey func(key string) bool, w, h int) {
	sel := NewListSelector(title, items)
	sel.Width = w - 6
	sel.Height = h - 4
	sel.TwoColumn = true
	if o.defSelHelp != "" {
		sel.HelpText = o.defSelHelp
	}
	s := overlayState{
		selector:      &sel,
		onSelect:      onSelect,
		onKey:         onKey,
		width:         w,
		height:        h,
		stayOnSelect:  true,
		anchor:        AnchorCenter,
		keyMatcher:    o.defKeyMatcher,
		style: lipgloss.NewStyle().
			Padding(1, 2).
			Background(lipgloss.Color("#282c34")),
	}
	o.stack = append(o.stack, s)
}

// ShowForkSelector displays a 2-line per item fork selector (TS pi-mono: UserMessageSelectorComponent).
func (o *Overlay) ShowForkSelector(title, description string, items []SelectorItem, onSelect func(value string), onKey func(key string) bool, w, h int) {
	sel := NewListSelector(title, items)
	sel.Width = w - 6
	sel.Height = h - 4
	sel.ForkMode = true
	sel.ForkDescription = description
	if o.defSelHelp != "" {
		sel.HelpText = o.defSelHelp
	}
	s := overlayState{
		selector:    &sel,
		onSelect:    onSelect,
		onKey:       onKey,
		width:       w,
		height:      h,
		anchor:      AnchorCenter,
		keyMatcher:  o.defKeyMatcher,
		style: lipgloss.NewStyle().
			Padding(1, 2).
			Background(lipgloss.Color("#282c34")),
	}
	o.stack = append(o.stack, s)
}

// ShowInput displays a single-line text input overlay.
func (o *Overlay) ShowInput(title string, onSubmit func(value string), onCancel func(), w, h int) {
	s := overlayState{
		inputMode:  true,
		inputTitle: title,
			inputCursor: 0,
		killRing:   &KillRing{},
		onSubmit:   onSubmit,
		onCancel:   onCancel,
		width:      w,
		height:     h,
		anchor:     AnchorCenter,
		hints:      o.defHints,
		keyMatcher: o.defKeyMatcher,
		style: lipgloss.NewStyle().
			Padding(1, 2).
			Background(lipgloss.Color("#282c34")),
	}
	o.stack = append(o.stack, s)
}

// ShowCustom displays a custom dialog overlay with content text and action buttons.
func (o *Overlay) ShowCustom(title string, contentText string, buttons []CustomButton, onChoice func(value string), onCancel func(), w, h int) {
	s := overlayState{
		customMode:     true,
		customTitle:    title,
		customContent:  contentText,
		customButtons:  buttons,
		customSelected: 0,
		onCustomChoice: onChoice,
		onCancel:       onCancel,
		width:          w,
		height:         h,
		anchor:         AnchorCenter,
		hints:           o.defHints,
		keyMatcher:      o.defKeyMatcher,
		style: lipgloss.NewStyle().
			Padding(1, 2).
			Background(lipgloss.Color("#282c34")),
	}
	o.stack = append(o.stack, s)
}

// ShowEditor displays a multi-line text editor overlay.
func (o *Overlay) ShowEditor(title string, prefill string, onSubmit func(value string), onCancel func(), w, h int) {
	lines := strings.Split(prefill, "\n")
	if len(lines) == 1 && lines[0] == "" {
		lines = []string{""}
	}
	s := overlayState{
		editorMode:     true,
		editorTitle:    title,
		editorLines:    lines,
		editorCursor:   len(lines) - 1,
		editorCol:      len(lines[len(lines)-1]),
		killRing:       &KillRing{},
		onEditorSubmit: onSubmit,
		onEditorCancel: onCancel,
		width:          w,
		height:         h,
		anchor:         AnchorCenter,
		hints:           o.defHints,
		keyMatcher:      o.defKeyMatcher,
		style: lipgloss.NewStyle().
			Padding(1, 2).
			Background(lipgloss.Color("#282c34")),
	}
	o.stack = append(o.stack, s)
}

// top returns a pointer to the top overlay state, or nil if empty.
func (o *Overlay) top() *overlayState {
	if len(o.stack) == 0 {
		return nil
	}
	return &o.stack[len(o.stack)-1]
}

// ReplaceItems updates the selector's items and title without recreating the overlay.
func (o *Overlay) ReplaceItems(title string, items []SelectorItem) {
	s := o.top()
	if s == nil || s.selector == nil {
		return
	}
	s.selector.SetItems(items)
	s.selector.SetTitle(title)
	s.selector.Filter = ""
	s.selector.Selected = 0
}

// SetFilter sets the filter text on the inner selector.
func (o *Overlay) SetFilter(filter string) {
	s := o.top()
	if s != nil && s.selector != nil {
		s.selector.Filter = filter
	}
}

// SelectedValue returns the value of the currently selected item, or "".
func (o *Overlay) SelectedValue() string {
	s := o.top()
	if s == nil || s.selector == nil {
		return ""
	}
	sel := s.selector.SelectedItem()
	if sel == nil {
		return ""
	}
	return sel.Value
}

// SelectIdx sets the selected index in the active selector (clamped).
func (o *Overlay) SelectIdx(idx int) {
	s := o.top()
	if s != nil && s.selector != nil {
		s.selector.SelectIdx(idx)
	}
}

// SelectedIndex returns the current selected index, or -1 if no selector.
func (o *Overlay) SelectedIndex() int {
	s := o.top()
	if s == nil || s.selector == nil || len(s.selector.Items) == 0 {
		return -1
	}
	return s.selector.Selected
}

// ItemCount returns the number of items in the active selector.
func (o *Overlay) ItemCount() int {
	s := o.top()
	if s == nil || s.selector == nil {
		return 0
	}
	return len(s.selector.Items)
}

// Hide dismisses the topmost overlay and restores the previous one (TS pi-mono pop behavior).
func (o *Overlay) Hide() {
	if len(o.stack) == 0 {
		return
	}
	o.stack = o.stack[:len(o.stack)-1]
}

// SetMargin sets the margin (in rows/cols) for the top overlay.
// Positive values add spacing from the anchor edge (TS pi-mono: OverlayOptions.margin).
func (o *Overlay) SetMargin(margin int) {
	s := o.top()
	if s != nil {
		s.margin = margin
	}
}

// OffsetX returns the horizontal offset of the top overlay.
func (o *Overlay) OffsetX() int {
	if s := o.top(); s != nil {
		return s.offsetX
	}
	return 0
}

// OffsetY returns the vertical offset of the top overlay.
func (o *Overlay) OffsetY() int {
	if s := o.top(); s != nil {
		return s.offsetY
	}
	return 0
}

// SetOffset sets the position offset from the anchor point for the top overlay.
// Positive offsetX shifts right, positive offsetY shifts down (TS pi-mono).
func (o *Overlay) SetOffset(x, y int) {
	if s := o.top(); s != nil {
		s.offsetX = x
		s.offsetY = y
	}
}


// SetVisible sets a visibility callback for the top overlay.
// The callback receives the terminal width and height and should return
// true if the overlay should be rendered (TS pi-mono: OverlayOptions.visible).
func (o *Overlay) SetVisible(fn func(w, h int) bool) {
	s := o.top()
	if s != nil {
		s.visible = fn
	}
}

// SetOnSelectionChange sets a callback invoked when the selector cursor moves.
// Used for live preview (TS pi-mono: theme preview on cursor move).
func (o *Overlay) SetOnSelectionChange(cb func(int)) {
	if s := o.top(); s != nil {
		s.onSelectionChange = cb
	}
}

// SetKeyMatcher sets a keybinding-aware matcher function on the top overlay.
// When set, the overlay uses this to check key presses against binding IDs
// instead of hardcoded key strings (TS pi-mono: kb.matches()).
func (o *Overlay) SetKeyMatcher(fn func(keyString, bindingID string) bool) {
	s := o.top()
	if s != nil {
		s.keyMatcher = fn
	}
}

// SetHints sets dynamic keybinding-aware hint strings for the top overlay.
// Empty strings are ignored (the overlay uses its built-in defaults).
// submit, cancel, extEditor correspond to InputSubmit, SelectCancel, GlobalExternalEditor.
func (o *Overlay) SetHints(submit, cancel, extEditor string) {
	s := o.top()
	if s != nil {
		if submit != "" {
			s.hints.Submit = submit
		}
		if cancel != "" {
			s.hints.Cancel = cancel
		}
		if extEditor != "" {
			s.hints.ExternalEditor = extEditor
		}
	}
}

// StartCountdown starts a countdown timer on the top overlay.
// When the countdown reaches zero, the overlay is auto-dismissed.
func (o *Overlay) StartCountdown(seconds int) {
	s := o.top()
	if s != nil && seconds > 0 {
		s.countdownRemaining = seconds
	}
}

// OnDismiss sets a callback invoked when the top overlay is dismissed
// (via esc, countdown expiry, or other cancel mechanism).
func (o *Overlay) OnDismiss(fn func()) {
	s := o.top()
	if s != nil {
		s.onDismiss = fn
	}
}

// HideAll dismisses all overlays.
func (o *Overlay) HideAll() {
	o.stack = nil
}

// CountdownTickMsg is sent every second when a dialog has an active countdown timer.
type CountdownTickMsg struct{}

// CountdownTick returns a command that fires a CountdownTickMsg after one second.
func CountdownTick() tea.Cmd {
	return tea.Tick(time.Second, func(t time.Time) tea.Msg {
		return CountdownTickMsg{}
	})
}

// Update handles Bubble Tea messages for the top overlay.
func (o *Overlay) Update(msg tea.Msg) tea.Cmd {
	s := o.top()
	if s == nil {
		return nil
	}

	// Countdown tick — decrement and auto-dismiss on expiry
	if _, ok := msg.(CountdownTickMsg); ok {
		if s.countdownRemaining > 0 {
			s.countdownRemaining--
			if s.countdownRemaining <= 0 {
				s.countdownExpired = true
				o.dismissTop()
				return nil
			}
		}
		if s.countdownRemaining > 0 {
			return CountdownTick()
		}
		return nil
	}

	if keyMsg, ok := msg.(tea.KeyMsg); ok {
		ks := keyMsg.String()

		// Input mode
		if s.inputMode {
			return o.updateInput(ks, s)
		}

		// Editor mode
		if s.editorMode {
			return o.updateEditor(ks, s)
		}

		// Custom dialog mode
		if s.customMode {
			return o.updateCustom(ks, s)
		}

		// In selector mode, handle navigation keys
		if s.selector != nil {
			return o.updateSelector(ks, s)
		}

		// Text mode: scrolling + confirm/cancel
		return o.updateText(ks, s)
	}
	return nil
}

// updateCustom handles key events for custom dialog overlays.
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
func pushInputUndo(s *overlayState) {
	s.inputUndoStack = append(s.inputUndoStack, inputUndoState{
		value:  s.inputBuf.String(),
		cursor: s.inputCursor,
	})
	if len(s.inputUndoStack) > 100 {
		s.inputUndoStack = s.inputUndoStack[1:]
	}
}


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
func (o Overlay) View() string {
	s := o.top()
	if s == nil {
		return ""
	}

	// Check dynamic visibility callback (TS pi-mono: OverlayOptions.visible)
	if s.visible != nil && !s.visible(s.termW, s.termH) {
		return ""
	}

	var content string
	if s.inputMode {
		content = o.renderInput(s)
	} else if s.editorMode {
		content = o.renderEditor(s)
	} else if s.customMode {
		content = o.renderCustom(s)
	} else if s.selector != nil {
		content = s.selector.View()
	} else {
		borderLine := lipgloss.NewStyle().
			Foreground(lipgloss.Color("#61afef")).
			Render(strings.Repeat("─", max(1, s.width)))
		content = borderLine + "\n" + o.scrollText(s) + "\n" + borderLine
	}

	style := s.style.Copy().Width(s.width).Height(s.height)
	if s.margin > 0 {
		style = style.Margin(s.margin, s.margin)
	}
		// Apply offsetX/offsetY by padding (TS pi-mono: offsetX/offsetY)
		if s.offsetX > 0 {
			content = strings.Repeat(" ", s.offsetX) + strings.ReplaceAll(content, "\n", "\n"+strings.Repeat(" ", s.offsetX))
		}
		if s.offsetY > 0 {
			content = strings.Repeat("\n", s.offsetY) + content
		}
		return style.Render(content)
}

// renderInput renders the text input overlay content with horizontal scrolling.
func (o Overlay) renderInput(s *overlayState) string {
	titleStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#61afef")).
		Bold(true)
	borderLine := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#61afef")).
		Render(strings.Repeat("─", max(1, s.width)))
	inputStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#abb2bf"))
	cursorStyle := lipgloss.NewStyle().
		Background(lipgloss.Color("#89b4fa")).
		Foreground(lipgloss.Color("#1e1e2e"))
	hintStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#5c6370"))
	overflowStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#5c6370"))

	text := s.inputBuf.String()
	runes := []rune(text)
	numRunes := len(runes)
	cursor := s.inputCursor // rune position (set by insert handler)

	// Horizontal scrolling: center cursor in available width (TS pi-mono)
	availWidth := s.width - 2
	if availWidth < 4 {
		availWidth = 4
	}
	var line string
	if visibleWidth(text) <= availWidth {
		// Text fits: render full text with cursor
		if numRunes == 0 {
			line = cursorStyle.Render(" ")
		} else if cursor >= numRunes {
			line = inputStyle.Render(text) + cursorStyle.Render(" ")
		} else if cursor == 0 {
			line = cursorStyle.Render(string(runes[0])) + inputStyle.Render(string(runes[1:]))
		} else {
			line = inputStyle.Render(string(runes[:cursor])) +
				cursorStyle.Render(string(runes[cursor])) +
				inputStyle.Render(string(runes[cursor+1:]))
		}
	} else {
		// Text too long: scroll window centered on cursor (rune-based)
		scrollRunes := availWidth / 2 // approximate rune count for visual width
		if scrollRunes < 1 {
			scrollRunes = 1
		}
		if cursor >= numRunes {
			scrollRunes = availWidth - 1 // leave room for cursor at end
			if scrollRunes < 1 {
				scrollRunes = 1
			}
		}
		start := 0
		if cursor > scrollRunes/2 {
			start = cursor - scrollRunes/2
		}
		if start+scrollRunes > numRunes {
			start = numRunes - scrollRunes
		}
		if start < 0 {
			start = 0
		}
		end := start + scrollRunes
		if end > numRunes {
			end = numRunes
		}

		visibleRunes := runes[start:end]
		visibleText := string(visibleRunes)
		cursorInVisible := cursor - start
		if cursorInVisible > len(visibleRunes) {
			cursorInVisible = len(visibleRunes)
		}
		if cursorInVisible < 0 {
			cursorInVisible = 0
		}

		// Build line with overflow indicators
		var lineBuf strings.Builder
		if start > 0 {
			lineBuf.WriteString(overflowStyle.Render("\u00ab")) // «
		}
		// Render visible portion with cursor (rune-safe)
		if cursorInVisible >= len(visibleRunes) {
			lineBuf.WriteString(inputStyle.Render(visibleText))
			lineBuf.WriteString(cursorStyle.Render(" "))
		} else {
			before := string(visibleRunes[:cursorInVisible])
			atCursor := string(visibleRunes[cursorInVisible])
			after := string(visibleRunes[cursorInVisible+1:])
			if before != "" {
				lineBuf.WriteString(inputStyle.Render(before))
			}
			lineBuf.WriteString(cursorStyle.Render(atCursor))
			if after != "" {
				lineBuf.WriteString(inputStyle.Render(after))
			}
		}
		if end < numRunes {
			lineBuf.WriteString(overflowStyle.Render("\u00bb")) // »
		}
		line = lineBuf.String()
	}

	var sb strings.Builder
	title := s.inputTitle
	if s.countdownRemaining > 0 {
		title = fmt.Sprintf("%s (%ds)", title, s.countdownRemaining)
	}
	sb.WriteString(titleStyle.Render(title))
	sb.WriteByte('\n')
	sb.WriteString(borderLine)
	sb.WriteByte('\n')
	sb.WriteString(line)
	sb.WriteByte('\n')
	// Dynamic key hints (TS pi-mono: keyHint/keyText)
	hintSubmit := s.hints.Submit
	if hintSubmit == "" {
		hintSubmit = "Enter"
	}
	hintCancel := s.hints.Cancel
	if hintCancel == "" {
		hintCancel = "Esc"
	}
	sb.WriteString(hintStyle.Render(hintSubmit + " submit  " + hintCancel + " cancel"))
	sb.WriteByte('\n')
	sb.WriteString(borderLine)

	return sb.String()
}

// renderEditor renders the multi-line editor overlay content.
func (o Overlay) renderEditor(s *overlayState) string {
	titleStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#61afef")).
		Bold(true)
	borderLine := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#61afef")).
		Render(strings.Repeat("─", max(1, s.width)))
	lineStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#abb2bf"))
	cursorStyle := lipgloss.NewStyle().
		Background(lipgloss.Color("#89b4fa")).
		Foreground(lipgloss.Color("#1e1e2e"))
	hintStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#5c6370"))

	visibleLines := s.height - 5
	if visibleLines < 1 {
		visibleLines = 1
	}

	// Calculate scroll window
	totalLines := len(s.editorLines)
	start := s.editorCursor - visibleLines/2
	if start < 0 {
		start = 0
	}
	end := start + visibleLines
	if end > totalLines {
		end = totalLines
		start = end - visibleLines
		if start < 0 {
			start = 0
		}
	}

	scrollStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#5c6370"))

	var sb strings.Builder
	title := s.editorTitle
	if s.countdownRemaining > 0 {
		title = fmt.Sprintf("%s (%ds)", title, s.countdownRemaining)
	}
	sb.WriteString(titleStyle.Render(title))
	sb.WriteByte('\n')
	sb.WriteString(borderLine)
	sb.WriteByte('\n')

	// Top scroll indicator (TS pi-mono: ─── ↑ N more lines ↑ ───)
	if start > 0 {
		sb.WriteString(scrollStyle.Render(fmt.Sprintf("─── ↑ %d more lines ↑ ───", start)))
		sb.WriteByte('\n')
	}

	for i := start; i < end; i++ {
		line := s.editorLines[i]
		if i == s.editorCursor {
			// Show cursor position within the line
			if s.editorCol >= len(line) {
				sb.WriteString(lineStyle.Render(line))
				sb.WriteString(cursorStyle.Render(" "))
			} else {
				before := line[:s.editorCol]
				cursor := string(line[s.editorCol])
				after := line[s.editorCol+1:]
				sb.WriteString(lineStyle.Render(before))
				sb.WriteString(cursorStyle.Render(cursor))
				sb.WriteString(lineStyle.Render(after))
			}
		} else {
			sb.WriteString(lineStyle.Render(line))
		}
		sb.WriteByte('\n')
	}

	// Bottom scroll indicator (TS pi-mono: ─── ↓ N more lines ↓ ───)
	if end < totalLines {
		remaining := totalLines - end
		sb.WriteString(scrollStyle.Render(fmt.Sprintf("─── ↓ %d more lines ↓ ───", remaining)))
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
	hintExtEditor := s.hints.ExternalEditor
	if hintExtEditor == "" {
		hintExtEditor = "Ctrl+G"
	}
	sb.WriteString(hintStyle.Render(hintSubmit + " submit  " + hintCancel + " cancel  " + hintExtEditor + " external editor"))
	sb.WriteByte('\n')
	sb.WriteString(borderLine)
	return sb.String()
}

// scrollText returns the visible portion of text content based on scroll offset.
// Available height (for content lines) is height - 4 (2 padding + 2 border).
func (o Overlay) scrollText(s *overlayState) string {
	lines := strings.Split(s.content, "\n")
	visibleLines := s.height - 4
	if visibleLines < 1 {
		visibleLines = 1
	}

	totalLines := len(lines)
	if totalLines <= visibleLines {
		return s.content // no scrolling needed
	}

	// Clamp scroll offset
	if s.scrollOffset > totalLines-visibleLines {
		s.scrollOffset = totalLines - visibleLines
	}
	if s.scrollOffset < 0 {
		s.scrollOffset = 0
	}

	start := s.scrollOffset
	end := start + visibleLines
	if end > totalLines {
		end = totalLines
	}

	var sb strings.Builder

	// Top scroll indicator
	if s.scrollOffset > 0 {
		indicator := fmt.Sprintf("─── ↑ %d more lines ↑ ───", s.scrollOffset)
		sb.WriteString(indicatorStyle.Render(indicator))
		sb.WriteByte('\n')
	}

	sb.WriteString(strings.Join(lines[start:end], "\n"))

	// Bottom scroll indicator
	if end < totalLines {
		remaining := totalLines - end
		indicator := fmt.Sprintf("─── ↓ %d more lines ↓ ───", remaining)
		sb.WriteByte('\n')
		sb.WriteString(indicatorStyle.Render(indicator))
	}

	return sb.String()
}

// indicatorStyle is the style for scroll indicators in text overlays.
var indicatorStyle = lipgloss.NewStyle().
	Foreground(lipgloss.Color("#5c6370")).
	Faint(true)

// OverlayConfirmMsg is sent when the user confirms the overlay selection.
type OverlayConfirmMsg struct{}

// SelectorChosenMsg is sent when the user selects an item from a ListSelector overlay.
type SelectorChosenMsg struct {
	Value string
}

// openExternalEditor opens $EDITOR (or nano/vi) on a temp file and returns the content.
// Used by the overlay editor's Ctrl+G handler.
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
